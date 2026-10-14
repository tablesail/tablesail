// Manual transaction mode of a query tab: the first run opens a transaction on a dedicated connection,
// every later run goes through it, and Commit / Roll back end it. The ids live on the tab object
// (runtime only, never saved); closing the session rolls back whatever is still open.

import { api, errorMessage, isCoreError } from '../api'
import { sessions } from '../stores/sessions.svelte'
import { tabs, type QueryTab } from '../stores/tabs.svelte'
import { ui } from '../stores/ui.svelte'
import type { Engine, QueryStatus } from '../types'
import { editorRequests } from './requests.svelte'

/** MongoDB has no SQL transactions here: the Auto-commit / Manual switch is not offered for it. */
export function txSupported(engine: Engine): boolean {
  return engine !== 'mongodb'
}

/** Statements of a finished run that executed: what the open transaction has accumulated. */
export function ranStatements(status: QueryStatus | null): number {
  return status?.statements.filter((s) => s.state === 'done').length ?? 0
}

/**
 * True when a failed commit / rollback means the backend no longer has the transaction
 * (already ended, or its connection dropped) rather than a problem with the statement itself.
 */
export function txGone(e: unknown): boolean {
  if (isCoreError(e) && (e.kind === 'notFound' || e.kind === 'invalidInput' || e.kind === 'connection' || e.kind === 'ssh')) return true
  return /no longer open|not open|no such transaction|unknown transaction|already (?:ended|closed|finished)/i.test(errorMessage(e))
}

/** The tab's open transaction id, beginning one first when it has none. */
export async function ensureTx(tab: QueryTab): Promise<string> {
  if (tab.txId) return tab.txId
  const id = await api.txBegin(tab.sessionId, tab.database)
  tabs.updateQuery(tab.id, { txId: id, txStatements: 0 })
  return id
}

const finishing = new Set<string>()

/** True while a commit / rollback of this tab's transaction is on its way. */
export const isEnding = (tab: Pick<QueryTab, 'id'>) => finishing.has(tab.id)

/**
 * Commits or rolls back the tab's transaction. Returns true when it ended (or there was none).
 * When the backend says the transaction is gone the tab's state is cleared and the user is told so;
 * any other failure leaves the transaction open so it can be retried or rolled back.
 */
export async function finishTx(tab: QueryTab, how: 'commit' | 'rollback'): Promise<boolean> {
  const id = tab.txId
  if (!id) return true
  if (finishing.has(tab.id)) return false
  if (tab.running) {
    ui.toast(`A query is still running in ${tab.title}. Stop it before you ${how === 'commit' ? 'commit' : 'roll back'}.`, 'error')
    return false
  }
  finishing.add(tab.id)
  try {
    if (how === 'commit') await api.txCommit(id)
    else await api.txRollback(id)
  } catch (e) {
    if (txGone(e)) {
      tabs.updateQuery(tab.id, { txId: null, txStatements: 0 })
      ui.toast(
        how === 'commit'
          ? 'Commit failed: the transaction was no longer open (the connection may have dropped), so its changes were not saved.'
          : 'The transaction was no longer open (the connection may have dropped), so there was nothing to roll back.',
        'error',
      )
    } else if (how === 'commit' && /transaction was rolled back/i.test(errorMessage(e))) {
      // Postgres ends the transaction when COMMIT fails; the backend has already forgotten it.
      tabs.updateQuery(tab.id, { txId: null, txStatements: 0 })
      ui.toast(`Commit failed: ${errorMessage(e)}`, 'error')
    } else {
      ui.toast(`${how === 'commit' ? 'Commit' : 'Rollback'} failed: ${errorMessage(e)}`, 'error')
    }
    return false
  } finally {
    finishing.delete(tab.id)
  }
  tabs.updateQuery(tab.id, { txId: null, txStatements: 0 })
  ui.toast(how === 'commit' ? 'Transaction committed' : 'Transaction rolled back', 'success')
  return true
}

/** Switches a tab between Auto-commit and Manual. Leaving Manual with a transaction open asks first. */
export function setManual(tab: QueryTab, on: boolean) {
  if (on) tabs.updateQuery(tab.id, { manual: true })
  else if (tab.txId) editorRequests.leaveManual = tab.id
  else tabs.updateQuery(tab.id, { manual: undefined })
}

/** The session's query tab that still has a transaction open, if any. */
export function openTxTab(sessionId: string): QueryTab | undefined {
  return tabs.forSession(sessionId).find((t): t is QueryTab => t.kind === 'query' && !!t.txId)
}

/**
 * Disconnects a session. Closing the connection rolls back whatever its tabs left open, so with an open
 * transaction the workspace asks first (see Workspace.svelte); otherwise it just disconnects.
 */
export function disconnect(sessionId: string) {
  if (openTxTab(sessionId)) editorRequests.disconnect = sessionId
  else void sessions.close(sessionId)
}
