// Command palette entries for SQL files, dump import and manual transactions.

import type { IconName } from '../ui/Icon.svelte'
import { tabs } from '../stores/tabs.svelte'
import type { SessionInfo } from '../types'
import { modLabel } from '../util/format'
import { importBlockedReason, openSqlFile, pickSqlImport, saveTab, saveTabAs } from './sqlFiles'
import { finishTx, setManual, txSupported } from './transaction'

export interface EditorCommand {
  group: string
  icon: IconName
  label: string
  hint?: string
  run: () => void
}

export function editorCommands(session: SessionInfo): EditorCommand[] {
  const tab = tabs.active(session.sessionId)
  const q = tab?.kind === 'query' ? tab : null
  const out: EditorCommand[] = []

  // An open transaction is the thing to act on, so these sit with the quick actions.
  if (q?.txId) {
    out.push(
      { group: 'Quick actions', icon: 'check', label: 'Commit transaction', hint: `${modLabel}⇧C`, run: () => void finishTx(q, 'commit') },
      { group: 'Quick actions', icon: 'undo', label: 'Roll back transaction', hint: `${modLabel}⇧R`, run: () => void finishTx(q, 'rollback') },
    )
  }

  out.push({ group: 'File', icon: 'code', label: 'Open SQL file…', hint: `${modLabel}O`, run: () => void openSqlFile(session) })
  if (q) {
    if (q.file) out.push({ group: 'File', icon: 'export', label: 'Save SQL file', hint: `${modLabel}S`, run: () => void saveTab(q) })
    // Without a file ⌘S asks for a path as well, so it is this entry's shortcut too.
    out.push({ group: 'File', icon: 'export', label: 'Save to file…', hint: `${modLabel}${q.file ? '⇧S' : 'S'}`, run: () => void saveTabAs(q) })
  }
  if (!importBlockedReason(session)) {
    out.push({ group: 'File', icon: 'import', label: 'Import SQL dump…', run: () => void pickSqlImport(session) })
  }
  if (q && txSupported(session.config.engine)) {
    out.push(
      q.manual
        ? { group: 'Transactions', icon: 'bolt', label: 'Switch to auto-commit', run: () => setManual(q, false) }
        : { group: 'Transactions', icon: 'shield', label: 'Switch to manual transactions', run: () => setManual(q, true) },
    )
  }
  return out
}
