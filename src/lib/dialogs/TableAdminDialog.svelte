<!--
  Confirmation for dropping, truncating or renaming a table (view, collection) from the sidebar.
  Shows the exact statement the server will run (api.tableAdmin with preview=true) before anything happens.
  Drop and Truncate are destructive (red button); Drop also asks for the name to be typed.
  A statement that waits for a lock (a transaction left open in a query tab, another session) must not trap
  the user in the dialog: after a few seconds it says so and can be closed (the statement is not cancelled).
-->
<script lang="ts">
  import { onDestroy } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import { api, errorMessage } from '../api'
  import { ui } from '../stores/ui.svelte'
  import {
    actionFor,
    confirmLabel,
    dialogTitle,
    dropConfirmed,
    holdsRows,
    itemsNoun,
    noun,
    renameProblem,
    type AdminDone,
    type AdminKind,
  } from '../sidebar/tableAdmin'
  import type { SessionInfo, TableKind, TableRef } from '../types'
  import { plainName } from '../util/sql'
  import { highlightSql } from '../util/sqlHighlight'

  interface Props {
    session: SessionInfo
    table: TableRef
    kind: TableKind
    action: AdminKind
    /** A query tab on this connection has an open transaction, which may hold locks on the object. */
    txOpen?: boolean
    /** Called once the server ran the action; the caller closes the dialog. */
    onDone: (done: AdminDone) => void
    onClose: () => void
  }

  /** How long a statement runs before the dialog admits that it is probably waiting. */
  const SLOW_MS = 4000

  let { session, table, kind, action, txOpen = false, onDone, onClose }: Props = $props()

  let engine = $derived(session.config.engine)
  // svelte-ignore state_referenced_locally
  let newName = $state(table.name)
  let typed = $state('')
  let busy = $state(false)
  /** Running for long enough that the database is probably waiting on a lock. */
  let slow = $state(false)
  let error = $state<string | null>(null)
  let statements = $state<string[] | null>(null)
  let previewError = $state<string | null>(null)

  let destructive = $derived(action !== 'rename')
  let what = $derived(noun(kind, engine))

  let problem = $derived(action === 'rename' ? renameProblem(table.name, newName) : null)
  // The unchanged, prefilled name is not an error to nag about; an edit that leaves it unusable is.
  let shownProblem = $derived(problem && newName !== table.name ? problem : null)
  let confirmed = $derived(action !== 'drop' || dropConfirmed(table.name, typed))
  let canSubmit = $derived(!busy && !problem && confirmed && !previewError)

  // Preview the statement for the current input; a stale answer (typing went on meanwhile) is dropped.
  let previewSeq = 0
  $effect(() => {
    const act = actionFor(action, table.name, newName)
    const seq = ++previewSeq
    previewError = null
    if (!act) {
      statements = null
      return
    }
    const timer = setTimeout(
      async () => {
        try {
          const sql = await api.tableAdmin(session.sessionId, table, act, true)
          if (seq !== previewSeq) return
          statements = sql
          previewError = null
        } catch (e) {
          if (seq !== previewSeq) return
          statements = null
          previewError = errorMessage(e)
        }
      },
      act.kind === 'rename' ? 150 : 0,
    )
    return () => clearTimeout(timer)
  })
  let closed = false
  let slowTimer: ReturnType<typeof setTimeout> | undefined
  onDestroy(() => {
    previewSeq++
    closed = true
    clearTimeout(slowTimer)
  })

  async function submit() {
    const act = actionFor(action, table.name, newName)
    if (!act || !canSubmit) return
    // Taken now: the dialog may be closed (while it waits for a lock) before the statement finishes.
    const done = onDone
    const name = table.name
    busy = true
    slow = false
    error = null
    slowTimer = setTimeout(() => (slow = true), SLOW_MS)
    try {
      await api.tableAdmin(session.sessionId, table, act, false)
      clearTimeout(slowTimer)
      done({ action, newName: act.kind === 'rename' ? act.newName : undefined })
    } catch (e) {
      clearTimeout(slowTimer)
      // A dialog that was closed meanwhile can't show the error any more.
      if (closed) ui.toast(`${dialogTitle(action, kind, engine)} “${name}” failed: ${errorMessage(e)}`, 'error', 8000)
      error = errorMessage(e)
      busy = false
      slow = false
    }
  }

  function onFieldKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      void submit()
    }
  }
</script>

<Modal title={dialogTitle(action, kind, engine)} {onClose} width="520px" dismissable={!busy || slow}>
  <div class="form">
    {#if action === 'drop'}
      <p id="tadmin-what">
        This permanently deletes the {what} <strong class="name">{plainName(table)}</strong>{#if holdsRows(kind)}{' '}and all of its
          {itemsNoun(engine)}{/if}. This can't be undone.
      </p>
    {:else if action === 'truncate'}
      <p id="tadmin-what">
        This permanently removes every {engine === 'mongodb' ? 'document' : 'row'} from <strong class="name">{plainName(table)}</strong>. The
        {what} itself stays. This can't be undone.
      </p>
    {:else}
      <p id="tadmin-what">Rename the {what} <strong class="name">{plainName(table)}</strong>. Anything that refers to it by name will need updating.</p>
    {/if}

    {#if action === 'rename'}
      <label>
        New name
        <input
          type="text"
          bind:value={newName}
          spellcheck="false"
          autocomplete="off"
          disabled={busy}
          aria-invalid={shownProblem ? 'true' : undefined}
          aria-describedby={shownProblem ? 'tadmin-problem' : undefined}
          onkeydown={onFieldKey}
          onfocus={(e) => e.currentTarget.select()}
        />
      </label>
      {#if shownProblem}<div class="error-text" id="tadmin-problem">{shownProblem}</div>{/if}
    {:else if action === 'drop'}
      <label>
        <span>Type <strong class="name">{table.name}</strong> to confirm</span>
        <input
          type="text"
          bind:value={typed}
          spellcheck="false"
          autocomplete="off"
          disabled={busy}
          aria-describedby="tadmin-what"
          onkeydown={onFieldKey}
        />
      </label>
    {/if}

    <div class="stmt" role="group" aria-label="Statement to run">
      <span class="overline">{statements && statements.length > 1 ? 'Statements' : 'Statement'}</span>
      {#if statements}
        <pre class="sql">{@html highlightSql(statements.join('\n'))}</pre>
      {:else if previewError}
        <div class="error-text" role="alert">{previewError}</div>
      {:else}
        <pre class="sql idle">{problem ? 'Enter a new name to see the statement.' : 'Loading…'}</pre>
      {/if}
    </div>

    {#if slow}
      <div class="warn" role="status">
        Still waiting for the database. Another session{txOpen ? ', or the transaction open in a query tab,' : ''} may hold a lock on this
        {what}. Closing this window does not cancel the statement: it runs as soon as the lock is released.
      </div>
    {:else if txOpen && !busy}
      <div class="warn" role="note">
        A transaction is open on this connection. If it has used this {what}, the statement waits until you commit or roll it back.
      </div>
    {/if}

    {#if error}<div class="error-text" role="alert">{error}</div>{/if}
  </div>

  {#snippet footer()}
    <button onclick={onClose} disabled={busy && !slow}>{slow ? 'Close' : 'Cancel'}</button>
    <button class={destructive ? 'danger' : 'primary'} onclick={submit} disabled={!canSubmit} aria-busy={busy}>
      {confirmLabel(action, engine)}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }

  p {
    margin: 0;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .name {
    font-family: var(--mono);
    font-size: 12.5px;
    font-weight: 600;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--fg-muted);
    font-size: 12px;
  }

  label .name {
    color: var(--fg);
    font-size: 12px;
  }

  .warn {
    color: var(--warning);
    font-size: 12px;
    line-height: 1.45;
  }

  .stmt {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .sql {
    margin: 0;
    padding: 8px 10px;
    max-height: 160px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-sidebar);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--fg-cell);
  }

  .sql.idle {
    color: var(--fg-faint);
  }

  .sql :global(.k) {
    color: var(--syn-keyword);
  }

  .sql :global(.s) {
    color: var(--syn-string);
  }

  .sql :global(.n) {
    color: var(--syn-number);
  }

  .sql :global(.c) {
    color: var(--syn-comment);
  }
</style>
