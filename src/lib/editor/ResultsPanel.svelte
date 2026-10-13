<!-- Result tabs (one per result set) plus a Messages tab for statement states, timings and errors. -->
<script lang="ts">
  import DataGrid from '../grid/DataGrid.svelte'
  import type { QueryRunner } from './runner.svelte'
  import type { StatementStatus } from '../types'
  import { formatCount, formatDuration } from '../util/format'
  import { inspector } from '../stores/inspector.svelte'
  import type { SessionInfo } from '../types'
  import type { GridSelection } from '../grid/types'

  interface Props {
    runner: QueryRunner
    onExport: (sql: string) => void
    /** For the Row inspector: which tab and session these results belong to. */
    tabId?: string
    session?: SessionInfo
    active?: boolean
  }

  let { runner, onExport, tabId = '', session, active = false }: Props = $props()

  let gridSel = $state.raw<GridSelection | null>(null)

  interface SetTab {
    key: string
    stmt: number
    set: number
    label: string
    sql: string
  }

  let chosen = $state<{ runId: number; key: string } | null>(null)

  let setTabs = $derived.by<SetTab[]>(() => {
    const out: SetTab[] = []
    for (const s of runner.status?.statements ?? []) {
      for (const set of s.sets) {
        if (set.columns.length === 0) continue
        const multi = s.sets.length > 1
        out.push({
          key: `${s.index}:${set.setIndex}`,
          stmt: s.index,
          set: set.setIndex,
          label: multi ? `Result ${s.index + 1}.${set.setIndex + 1}` : `Result ${s.index + 1}`,
          sql: s.sql,
        })
      }
    }
    return out
  })

  let hasError = $derived(!!runner.runError || (runner.status?.statements ?? []).some((s) => s.state === 'error'))
  let finished = $derived(runner.status?.finished ?? false)

  // Automatic choice until the user clicks a tab: first result set; Messages when a run failed
  // or produced no rows.
  let activeKey = $derived.by(() => {
    if (chosen && chosen.runId === runner.runId) return chosen.key
    if (hasError) return 'messages'
    if (setTabs.length) return setTabs[0].key
    return 'messages'
  })

  let activeSet = $derived(setTabs.find((t) => t.key === activeKey) ?? null)

  function select(key: string) {
    chosen = { runId: runner.runId, key }
  }

  const STATE_LABEL: Record<StatementStatus['state'], string> = {
    pending: 'Pending',
    running: 'Running',
    done: 'Done',
    error: 'Error',
    cancelled: 'Cancelled',
  }

  function describe(s: StatementStatus): string {
    if (s.state === 'error' || s.state === 'cancelled') return STATE_LABEL[s.state]
    if (s.sets.length) {
      const n = s.sets.reduce((a, x) => a + x.loaded, 0)
      return s.state === 'done' ? `${formatCount(n)} ${n === 1 ? 'row' : 'rows'}` : `${formatCount(n)}+ rows`
    }
    if (s.affected != null) return `${formatCount(s.affected)} ${s.affected === 1 ? 'row' : 'rows'} affected`
    return STATE_LABEL[s.state]
  }

  // Publish the focused result row to the Row inspector while this tab is visible.
  $effect(() => {
    void runner.revision
    const sel = gridSel
    const set = activeSet
    if (!active || !tabId || !session || !sel || !set) return
    const info = runner.info(set.stmt, set.set)
    const row = runner.getRow(set.stmt, set.set, sel.activeRow)
    if (!row) return
    inspector.show({
      tabId,
      session,
      table: null,
      columns: info.columns,
      row,
      rowIndex: sel.activeRow,
      primaryKey: [],
      foreignKeys: [],
      editable: false,
    })
  })
</script>

<div class="results">
  <div class="tabs" role="tablist">
    {#each setTabs as t (t.key)}
      <button role="tab" class:on={activeKey === t.key} aria-selected={activeKey === t.key} onclick={() => select(t.key)}
        >{t.label}</button
      >
    {/each}
    <button
      role="tab"
      class:on={activeKey === 'messages'}
      aria-selected={activeKey === 'messages'}
      onclick={() => select('messages')}
    >
      Messages{#if hasError}<span class="badge">!</span>{/if}
    </button>
  </div>

  <div class="content">
    {#if activeSet}
      {@const info = runner.info(activeSet.stmt, activeSet.set)}
      <div class="grid">
        {#key activeSet.key + ':' + runner.runId}
          <DataGrid
            columns={info.columns}
            rowCount={runner.rowCount(activeSet.stmt, activeSet.set)}
            getRow={(i) => runner.getRow(activeSet.stmt, activeSet.set, i)}
            onNeedRows={(a, b) => runner.needRows(activeSet.stmt, activeSet.set, a, b)}
            revision={runner.revision}
            onSelectionChange={(sel) => (gridSel = sel)}
          />
        {/key}
      </div>
      <div class="foot">
        <span>
          {#if info.complete}{formatCount(info.loaded)} {info.loaded === 1 ? 'row' : 'rows'}{:else}≥ {formatCount(info.loaded)} rows{/if}
        </span>
        {#if info.truncated}
          <span class="warn">Showing the first {formatCount(info.loaded)} rows only (non-final statement of a script).</span>
        {/if}
        {#if info.failed}<span class="error-text" title={info.failed}>Fetch failed: {info.failed}</span>{/if}
        <span class="spacer"></span>
        <button class="outline" onclick={() => onExport(activeSet.sql)}>Export…</button>
      </div>
    {:else}
      <div class="messages">
        {#if runner.runError}
          <div class="msg bad">{runner.runError}</div>
        {/if}
        {#if !runner.status && !runner.runError}
          <div class="muted pad">{runner.running ? 'Starting…' : 'Run a query to see results here.'}</div>
        {/if}
        {#each runner.status?.statements ?? [] as s (s.index)}
          <div class="msg" class:bad={s.state === 'error'}>
            <div class="head">
              <span class="state state-{s.state}">{STATE_LABEL[s.state]}</span>
              <span class="n">Statement {s.index + 1}</span>
              <span class="muted">{describe(s)}</span>
              <span class="spacer"></span>
              <span class="muted">{formatDuration(s.elapsedMs)}</span>
            </div>
            <pre class="sql">{s.sql}</pre>
            {#if s.error}
              <div class="error-text err">
                {s.error.message}{#if s.error.code}
                  <span class="muted">({s.error.code})</span>{/if}
              </div>
            {/if}
          </div>
        {/each}
        {#if finished && !hasError && (runner.status?.statements.length ?? 0) > 0}
          <div class="muted pad">Finished in {formatDuration(runner.elapsedMs)}.</div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .results {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }

  .tabs {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 36px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex: none;
  }

  .tabs button {
    min-height: 24px;
    padding: 0 11px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    box-shadow: none;
    white-space: nowrap;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .tabs button:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .tabs button.on,
  .tabs button.on:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-chip);
    border-color: var(--border-input);
  }

  .badge {
    margin-left: 5px;
    background: var(--danger);
    color: #fff;
    border-radius: 8px;
    padding: 0 5px;
    font-size: 10px;
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .grid {
    flex: 1;
    min-height: 0;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    padding: 0 10px;
    border-top: 1px solid var(--border);
    background: var(--bg-chrome);
    font-size: 11px;
    color: var(--fg-muted);
    flex: none;
  }

  .foot button {
    min-height: 24px;
    padding: 0 9px;
    font-size: 11px;
  }

  .warn {
    color: var(--warning);
  }

  .spacer {
    flex: 1;
  }

  .messages {
    overflow: auto;
    padding: 8px 14px;
    user-select: text;
    -webkit-user-select: text;
  }

  .pad {
    padding: 8px 0;
  }

  .msg {
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }

  .msg.bad {
    color: var(--danger);
  }

  .head {
    display: flex;
    gap: 10px;
    align-items: center;
  }

  .n {
    font-weight: 600;
  }

  .state {
    font-size: 11px;
    padding: 0 7px;
    border-radius: 8px;
    border: 1px solid var(--border);
  }

  .state-done {
    color: var(--success);
    border-color: var(--success);
  }

  .state-error {
    color: var(--danger);
    border-color: var(--danger);
  }

  .state-running {
    color: var(--accent);
    border-color: var(--accent);
  }

  .sql {
    margin: 4px 0 0;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg-muted);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 90px;
    overflow: auto;
  }

  .err {
    margin-top: 4px;
    white-space: pre-wrap;
  }
</style>
