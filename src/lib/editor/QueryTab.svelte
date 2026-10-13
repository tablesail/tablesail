<!-- A SQL editor tab: editor on top, results below, resizable split. -->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import { onDestroy } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import ResultsPanel from './ResultsPanel.svelte'
  import SqlEditor from './SqlEditor.svelte'
  import { QueryRunner } from './runner.svelte'
  import { api, errorMessage } from '../api'
  import { sessions } from '../stores/sessions.svelte'
  import type { QueryTab } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import { history } from '../stores/history.svelte'
  import { pulse } from '../stores/pulse.svelte'
  import { saved } from '../stores/saved.svelte'
  import { inspector } from '../stores/inspector.svelte'
  import { ENGINE_LABEL } from '../util/env'
  import type { SessionInfo, StatementSpan } from '../types'
  import { bytesToUtf16Many } from '../util/utf8'
  import { formatDuration, modLabel } from '../util/format'
  import type { SchemaConfig } from './cm'

  interface Props {
    tab: QueryTab
    session: SessionInfo
    active: boolean
  }

  let { tab, session, active }: Props = $props()

  const runner = new QueryRunner()
  let editor = $state<ReturnType<typeof SqlEditor>>()
  let container = $state<HTMLDivElement>()
  let frac = $state(0.45)
  let confirm = $state<{ sql: string; destructive: StatementSpan[] } | null>(null)
  let notice = $state<string | null>(null)
  let saving = $state<{ name: string } | null>(null)

  let engine = $derived(session.config.engine)
  let entry = $derived(sessions.schemaEntry(session.sessionId, tab.database))
  let schemaCfg = $derived<SchemaConfig>({
    engine,
    columns: entry.snapshot?.columns ?? {},
    defaultSchema: engine === 'postgres' ? 'public' : undefined,
  })

  $effect(() => {
    if (!entry.snapshot && !entry.loading && !entry.error) {
      void sessions.ensureSchema(session.sessionId, tab.database)
    }
  })

  $effect(() => {
    if (active) queueMicrotask(() => editor?.focus())
  })

  onDestroy(() => {
    void runner.close()
    inspector.clear(tab.id)
  })

  /** The statement containing the cursor; between statements, the one before it. */
  async function statementAt(text: string, pos: number): Promise<string | null> {
    const spans = await api.sqlSplit(text, engine)
    if (spans.length === 0) return null
    const offs = bytesToUtf16Many(
      text,
      spans.flatMap((s) => [s.start, s.end]),
    )
    let best = -1
    for (let i = 0; i < spans.length; i++) {
      if (pos >= offs[2 * i] && pos <= offs[2 * i + 1]) {
        best = i
        break
      }
    }
    if (best < 0) {
      for (let i = 0; i < spans.length; i++) if (offs[2 * i + 1] <= pos) best = i
      if (best < 0) best = 0
    }
    return spans[best].sql
  }

  async function execute(all: boolean) {
    if (!editor || runner.running) return
    notice = null
    try {
      const text = editor.getText()
      const sel = editor.getSelection()
      let sql: string | null
      if (sel) sql = sel.text
      else if (all) sql = text
      else sql = await statementAt(text, editor.getCursor())
      if (!sql || !sql.trim()) return

      if (session.config.env === 'production') {
        const spans = await api.sqlSplit(sql, engine)
        const destructive = spans.filter((s) => s.destructive)
        if (destructive.length) {
          confirm = { sql, destructive }
          return
        }
      }
      await start(sql)
    } catch (e) {
      notice = errorMessage(e)
    }
  }

  async function start(sql: string) {
    confirm = null
    await runner.run(session.sessionId, tab.database, sql)
    const st = runner.status
    const ok = !runner.runError && !(st?.statements.some((x) => x.state === 'error') ?? false)
    history.add({
      sql,
      connectionId: session.config.id,
      connectionName: session.config.name,
      database: tab.database,
      elapsedMs: runner.elapsedMs,
      ok,
    })
    if (ok) pulse.record(session.sessionId, runner.elapsedMs)
  }

  const FORMAT_LANG = { postgres: 'postgresql', mysql: 'mysql', mariadb: 'mariadb', sqlite: 'sqlite' } as const

  async function formatSql() {
    if (!editor || engine === 'mongodb') return
    try {
      const { format } = await import('sql-formatter')
      const sel = editor.getSelection()
      const text = editor.getText()
      const out = format(sel ? sel.text : text, { language: FORMAT_LANG[engine], keywordCase: 'upper', tabWidth: 2 })
      editor.setText(sel ? text.slice(0, sel.from) + out + text.slice(sel.to) : out)
    } catch (e) {
      notice = `Could not format: ${errorMessage(e)}`
    }
  }

  function saveQuery() {
    const text = editor?.getText() ?? tab.sql
    if (!text.trim()) return
    saving = { name: tab.title.startsWith('Query ') ? '' : tab.title }
  }

  function confirmSave() {
    if (!saving) return
    const name = saving.name.trim() || `Query ${new Date().toLocaleString()}`
    saved.add({ name, sql: editor?.getText() ?? tab.sql, connectionId: session.config.id, database: tab.database })
    tab.title = name
    saving = null
    ui.toast(`Saved “${name}”`, 'success')
  }

  function onSplitDown(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      if (!container) return
      const r = container.getBoundingClientRect()
      frac = Math.min(0.85, Math.max(0.15, (ev.clientY - r.top) / r.height))
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }

  function exportResult(sql: string) {
    ui.exportQuery(session, tab.database, sql)
  }

  let summary = $derived.by(() => {
    if (runner.running) return `Running… ${formatDuration(runner.elapsedMs)}`
    const st = runner.status
    if (!st) return ''
    const n = st.statements.length
    const errored = st.statements.some((s) => s.state === 'error')
    const cancelled = st.statements.some((s) => s.state === 'cancelled')
    const state = errored ? 'failed' : cancelled ? 'cancelled' : 'finished'
    return `${n} statement${n === 1 ? '' : 's'} ${state} in ${formatDuration(runner.elapsedMs)}`
  })
</script>

<div class="query" bind:this={container}>
  <div class="toolbar">
    <div class="title"><Icon name="code" size={14} /> SQL Editor</div>
    <span class="meta">{ENGINE_LABEL[engine]} · {session.config.readOnly ? 'Read only' : session.config.env}</span>
    <span class="status" class:err={!!notice || !!runner.runError}>{notice ?? summary}</span>
    <span class="spacer"></span>
    <label class="db" title="Database">
      <Icon name="database" size={13} />
      <select bind:value={tab.database}>
        {#each session.databases as db (db)}
          <option value={db}>{db}</option>
        {/each}
        {#if !session.databases.includes(tab.database)}
          <option value={tab.database}>{tab.database || '(default)'}</option>
        {/if}
      </select>
    </label>
    {#if engine !== 'mongodb'}
      <button class="bare" onclick={formatSql} title="Format SQL"><Icon name="bolt" size={13} /> Format</button>
    {/if}
    <button class="bare" onclick={saveQuery} title="Save to saved queries"><Icon name="star" size={13} /> Save</button>
    <button class="bare" onclick={() => execute(true)} disabled={runner.running} title="Run the whole script (⇧{modLabel}↵)"><Icon name="play-all" size={13} /> Run all</button>
    {#if runner.running}
      <button class="danger run" onclick={() => runner.cancel()} title="Cancel the running query"><Icon name="stop" size={11} strokeWidth={2} /> Stop</button>
    {:else}
      <button class="primary run" onclick={() => execute(false)} title="Run the statement under the cursor ({modLabel}↵)">
        <Icon name="play" size={11} strokeWidth={2} /> Run query <span class="kbd">{modLabel}↵</span>
      </button>
    {/if}
  </div>

  <div class="pane editor" style:height="{frac * 100}%">
    <SqlEditor
      bind:this={editor}
      value={tab.sql}
      schema={schemaCfg}
      onChange={(t) => (tab.sql = t)}
      onRun={() => execute(false)}
      onRunAll={() => execute(true)}
      autofocus={active}
    />
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="split" onpointerdown={onSplitDown}></div>
  <div class="pane results">
    <ResultsPanel {runner} onExport={exportResult} tabId={tab.id} {session} {active} />
  </div>
</div>

{#if saving}
  {@const sv = saving}
  <Modal title="Save query" onClose={() => (saving = null)} width="420px">
    <label class="save">
      <span class="muted">Name</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input type="text" bind:value={sv.name} placeholder="Revenue by month" autofocus onkeydown={(e) => e.key === 'Enter' && confirmSave()} />
    </label>
    {#snippet footer()}
      <button onclick={() => (saving = null)}>Cancel</button>
      <button class="primary" onclick={confirmSave}>Save</button>
    {/snippet}
  </Modal>
{/if}

{#if confirm}
  {@const pending = confirm}
  <Modal title="Run on production?" onClose={() => (confirm = null)} width="560px">
    <p class="warn">
      <b>{session.config.name}</b> is a production connection. These statements change or remove data:
    </p>
    <ul class="stmts">
      {#each pending.destructive as s, i (i)}
        <li><pre>{s.sql}</pre></li>
      {/each}
    </ul>
    {#snippet footer()}
      <button onclick={() => (confirm = null)}>Cancel</button>
      <button class="danger" onclick={() => start(pending.sql)}>Run on production</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .query {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 11px;
    min-height: 46px;
    padding: 0 16px 0 24px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }

  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
  }

  .title :global(svg) {
    color: var(--violet);
  }

  .meta {
    font-size: 10px;
    color: var(--fg-faint);
    white-space: nowrap;
  }

  .bare {
    gap: 6px;
    min-height: 28px;
    padding: 0 7px;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
    color: var(--fg-muted);
  }

  .bare:hover:not(:disabled) {
    background: none;
    color: var(--fg);
  }

  .run {
    gap: 7px;
    min-height: 30px;
    padding: 0 12px;
    border-radius: 6px;
    font-size: 11px;
  }

  .kbd {
    margin-left: 4px;
    font-size: 9px;
    opacity: 0.6;
  }

  .save {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .status {
    color: var(--fg-muted);
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status.err {
    color: var(--danger);
  }

  .spacer {
    flex: 1;
  }

  .db {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--fg-muted);
  }

  .db select {
    min-height: 28px;
    font-size: 11px;
    color: var(--fg);
  }

  .pane {
    min-height: 0;
    overflow: hidden;
  }

  .results {
    flex: 1;
  }

  .split {
    height: 1px;
    cursor: row-resize;
    background: var(--border);
    flex: none;
    position: relative;
    z-index: 2;
    touch-action: none;
  }

  /* Taller invisible grab area around the 1px divider. */
  .split::after {
    content: '';
    position: absolute;
    inset: -4px 0;
  }

  .split:hover {
    background: var(--accent);
  }

  .warn {
    margin-top: 0;
  }

  .stmts {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  pre {
    margin: 0;
    padding: 6px 8px;
    background: var(--bg-sidebar);
    border-radius: 5px;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 120px;
    overflow: auto;
  }
</style>
