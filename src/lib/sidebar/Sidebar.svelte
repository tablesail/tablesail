<!--
  Left sidebar, TablePlus-style: search, the selected schema's tables grouped by
  kind, and database/schema pickers at the bottom. The clock and star buttons
  swap the list for query history or saved queries.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import Icon, { type IconName } from '../ui/Icon.svelte'
  import { history } from '../stores/history.svelte'
  import { saved } from '../stores/saved.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui, type RailView } from '../stores/ui.svelte'
  import type { SessionInfo, TableInfo, TableKind, TableRef } from '../types'
  import { formatCount } from '../util/format'
  import { plainName, selectStarter } from '../util/sql'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let filter = $state('')
  let collapsed = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)
  let chosenSchema = $state<string | null>(null)

  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  let activeTab = $derived(tabs.active(session.sessionId))

  const KIND: Record<TableKind, IconName> = {
    table: 'table',
    virtual: 'table',
    foreignTable: 'table',
    system: 'table',
    collection: 'collection',
    timeSeries: 'collection',
    view: 'view',
    materializedView: 'matview',
    sequence: 'sequence',
  }

  const SECTIONS: { title: string; kinds: TableKind[] }[] = [
    { title: 'Tables', kinds: ['table', 'virtual', 'foreignTable', 'system'] },
    { title: 'Collections', kinds: ['collection', 'timeSeries'] },
    { title: 'Views', kinds: ['view'] },
    { title: 'Materialized views', kinds: ['materializedView'] },
    { title: 'Sequences', kinds: ['sequence'] },
  ]

  let schemaNames = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return [] as string[]
    const rank = (x: string) => (x === 'public' || x === '' ? 0 : /^(pg_|information_schema)/.test(x) ? 2 : 1)
    return Object.keys(snap.schemas).sort((x, y) => rank(x) - rank(y) || x.localeCompare(y))
  })

  let schema = $derived(chosenSchema !== null && schemaNames.includes(chosenSchema) ? chosenSchema : (schemaNames[0] ?? ''))

  let sections = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return []
    const q = filter.trim().toLowerCase()
    // A search looks across every schema; otherwise show the selected one.
    const pool = q ? Object.values(snap.schemas).flat() : (snap.schemas[schema] ?? [])
    const list = q ? pool.filter((t) => t.name.toLowerCase().includes(q)) : pool
    return SECTIONS.map((sec) => ({
      title: sec.title,
      tables: list.filter((t) => sec.kinds.includes(t.kind)).sort((a, b) => a.name.localeCompare(b.name)),
    })).filter((sec) => sec.tables.length > 0)
  })

  let total = $derived(sections.reduce((n, sec) => n + sec.tables.length, 0))
  let showSchemaInName = $derived(filter.trim() !== '' && schemaNames.length > 1)

  function ref(t: TableInfo): TableRef {
    return { database: t.database, schema: t.schema ?? null, name: t.name }
  }

  function isActive(t: TableInfo): boolean {
    const a = activeTab
    return a?.kind === 'table' && a.table.name === t.name && (a.table.schema ?? null) === (t.schema ?? null) && a.table.database === t.database
  }

  function tableMenu(t: TableInfo): MenuItem[] {
    const r = ref(t)
    const isTable = t.kind === 'table' || t.kind === 'collection' || t.kind === 'virtual'
    return [
      { label: 'Open', onclick: () => tabs.openTable(session.sessionId, r) },
      { label: 'New query', onclick: () => tabs.newQuery(session.sessionId, database, selectStarter(session.config.engine, r)) },
      { separator: true },
      { label: 'Export…', onclick: () => ui.exportTable(session, { table: r, filters: [], matchAll: true, rawWhere: null, sort: [], columns: [] }) },
      { label: 'Import CSV…', disabled: !isTable || session.config.readOnly, onclick: () => ui.importInto(session, r) },
      { separator: true },
      { label: 'Copy name', onclick: () => void navigator.clipboard?.writeText(plainName(r)) },
      { label: 'Refresh', onclick: () => sessions.refreshSchema(session.sessionId, database) },
    ]
  }

  function setView(v: RailView) {
    ui.rail = ui.rail === v ? 'browser' : v
    filter = ''
  }

  function openSql(sql: string, db: string | null) {
    tabs.newQuery(session.sessionId, db && session.databases.includes(db) ? db : database, sql)
  }

  function ago(at: number): string {
    const s = Math.round((Date.now() - at) / 1000)
    if (s < 60) return 'just now'
    if (s < 3600) return `${Math.floor(s / 60)}m ago`
    if (s < 86400) return `${Math.floor(s / 3600)}h ago`
    return `${Math.floor(s / 86400)}d ago`
  }

  let historyList = $derived.by(() => {
    const q = filter.trim().toLowerCase()
    return history.entries.filter((h) => !q || h.sql.toLowerCase().includes(q)).slice(0, 200)
  })

  let savedList = $derived.by(() => {
    const q = filter.trim().toLowerCase()
    return saved.list.filter((s) => !q || s.name.toLowerCase().includes(q) || s.sql.toLowerCase().includes(q))
  })

  let placeholder = $derived(ui.rail === 'history' ? 'Search history…' : ui.rail === 'saved' ? 'Search saved queries…' : 'Search for item…')
</script>

<aside class="sidebar">
  <div class="top">
    <label class="search">
      <Icon name="search" size={12} />
      <input type="search" {placeholder} bind:value={filter} spellcheck="false" />
    </label>
    <button class="mini" class:on={ui.rail === 'history'} title="Query history" aria-label="Query history" onclick={() => setView('history')}>
      <Icon name="clock" size={14} />
    </button>
    <button class="mini" class:on={ui.rail === 'saved'} title="Saved queries" aria-label="Saved queries" onclick={() => setView('saved')}>
      <Icon name="star" size={14} />
    </button>
  </div>

  <div class="list">
    {#if ui.rail === 'browser'}
      {#if !entry.snapshot && entry.loading}
        <div class="note">Loading schema…</div>
      {:else if !entry.snapshot && entry.error}
        <div class="note error-text">{entry.error}</div>
      {:else if entry.snapshot && total === 0}
        <div class="note">{filter ? 'No matching items.' : 'Nothing here yet.'}</div>
      {/if}

      {#each sections as sec (sec.title)}
        <button class="section" onclick={() => (collapsed[sec.title] = !collapsed[sec.title])} aria-expanded={!collapsed[sec.title]}>
          <span class="chev" class:open={!collapsed[sec.title]}><Icon name="chevron-right" size={10} strokeWidth={2} /></span>
          {sec.title}
          <span class="count">{sec.tables.length}</span>
        </button>
        {#if !collapsed[sec.title]}
          {#each sec.tables as t (t.schema + '\u0000' + t.name)}
            <button
              class="row"
              class:active={isActive(t)}
              title={`${t.name}${t.estimatedRows != null ? ` · ≈ ${formatCount(t.estimatedRows)} rows` : ''}${t.comment ? `\n${t.comment}` : ''}`}
              onclick={() => tabs.openTable(session.sessionId, ref(t))}
              oncontextmenu={(e) => {
                e.preventDefault()
                menu = { x: e.clientX, y: e.clientY, items: tableMenu(t) }
              }}
            >
              <Icon name={KIND[t.kind] ?? 'table'} size={14} />
              <span class="name">{#if showSchemaInName && t.schema}<span class="dim">{t.schema}.</span>{/if}{t.name}</span>
            </button>
          {/each}
        {/if}
      {/each}
      {#if entry.error && entry.snapshot}
        <div class="note error-text" title={entry.error}>Refresh failed: {entry.error}</div>
      {/if}
    {:else if ui.rail === 'history'}
      <div class="section static">
        Query history <span class="count">{history.entries.length}</span>
        {#if history.entries.length > 0}<button class="clear" onclick={() => history.clear()}>Clear</button>{/if}
      </div>
      {#if historyList.length === 0}
        <div class="note">{filter ? 'No matching queries.' : 'Queries you run appear here.'}</div>
      {/if}
      {#each historyList as h (h.id)}
        <button class="item" onclick={() => openSql(h.sql, h.database)} title={h.sql}>
          <code class:err={!h.ok}>{h.sql.split('\n')[0]}</code>
          <small>{#if !h.ok}<span class="bad">failed</span> · {/if}{h.connectionName} · {ago(h.at)}{h.elapsedMs != null ? ` · ${Math.round(h.elapsedMs)} ms` : ''}</small>
        </button>
      {/each}
    {:else}
      <div class="section static">Saved queries <span class="count">{saved.list.length}</span></div>
      {#if savedList.length === 0}
        <div class="note">{filter ? 'No matching queries.' : 'Save a query from the SQL editor to keep it here.'}</div>
      {/if}
      {#each savedList as q (q.id)}
        <button
          class="item"
          onclick={() => openSql(q.sql, q.database)}
          title={q.sql}
          oncontextmenu={(e) => {
            e.preventDefault()
            menu = {
              x: e.clientX,
              y: e.clientY,
              items: [
                { label: 'Open', onclick: () => openSql(q.sql, q.database) },
                { label: 'Copy SQL', onclick: () => void navigator.clipboard?.writeText(q.sql) },
                { separator: true },
                { label: 'Delete', danger: true, onclick: () => saved.remove(q.id) },
              ],
            }
          }}
        >
          <span class="qname">{q.name}</span>
          <code>{q.sql.split('\n')[0]}</code>
        </button>
      {/each}
    {/if}
  </div>

  <div class="bottom">
    <select value={database} onchange={(e) => sessions.setDatabase(session.sessionId, e.currentTarget.value)} title="Database" aria-label="Database">
      {#each session.databases as db (db)}
        <option value={db}>{db}</option>
      {/each}
      {#if session.databases.length === 0}
        <option value={database}>{database || '(default)'}</option>
      {/if}
    </select>
    {#if schemaNames.length > 1 || (schemaNames.length === 1 && schemaNames[0] !== '')}
      <select value={schema} onchange={(e) => (chosenSchema = e.currentTarget.value)} title="Schema" aria-label="Schema">
        {#each schemaNames as name (name)}
          <option value={name}>{name || '(default)'}</option>
        {/each}
      </select>
    {/if}
  </div>
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--bg-sidebar);
  }

  .top {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 10px 8px 8px 10px;
  }

  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    margin-right: 4px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: 7px;
    background: var(--bg-input);
    color: var(--fg-faint);
  }

  .search:focus-within {
    border-color: var(--accent);
  }

  .search input {
    flex: 1;
    width: 100%;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
  }

  .search input:focus {
    box-shadow: none;
  }

  .mini {
    width: 26px;
    min-width: 26px;
    height: 26px;
    min-height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    box-shadow: none;
    color: var(--fg-faint);
  }

  .mini:hover:not(:disabled) {
    background: var(--bg-active);
    color: var(--fg);
  }

  .mini.on,
  .mini.on:hover:not(:disabled) {
    background: var(--accent-solid);
    color: var(--accent-solid-fg);
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 10px;
  }

  .note {
    padding: 8px;
    font-size: 11px;
    color: var(--fg-muted);
    word-break: break-word;
  }

  .section {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 5px;
    width: 100%;
    height: 24px;
    min-height: 24px;
    margin-top: 4px;
    padding: 0 4px;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 12px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .section:hover:not(:disabled) {
    background: none;
    color: var(--fg-muted);
  }

  .section .count {
    margin-left: auto;
    font-size: 10px;
    font-weight: 500;
    color: var(--fg-faint);
  }

  .chev {
    display: inline-flex;
    transition: transform 0.12s;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .clear {
    margin-left: auto;
    min-height: 0;
    padding: 0 4px;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 10px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg-faint);
  }

  .clear:hover:not(:disabled) {
    background: none;
    color: var(--fg);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 7px;
    width: 100%;
    height: 22px;
    min-height: 22px;
    padding: 0 8px 0 19px;
    border: none;
    border-radius: 5px;
    background: none;
    box-shadow: none;
    text-align: left;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg-cell);
  }

  .row :global(svg) {
    color: var(--info);
  }

  .row:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .row.active,
  .row.active:hover:not(:disabled) {
    background: var(--accent-solid);
    color: var(--accent-solid-fg);
  }

  .row.active :global(svg) {
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .row.active :global(svg) {
    color: color-mix(in srgb, var(--accent) 60%, #2d3a12);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dim {
    color: var(--fg-faint);
  }

  .item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
    width: 100%;
    min-height: 0;
    padding: 7px 8px;
    border: none;
    border-radius: 6px;
    background: none;
    box-shadow: none;
    text-align: left;
  }

  .item:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .item code {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--fg-cell);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item code.err {
    color: var(--danger);
  }

  .item small {
    font-size: 10px;
    color: var(--fg-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bad {
    color: var(--danger);
  }

  .qname {
    font-weight: 600;
    color: var(--fg);
  }

  .bottom {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    flex: none;
  }

  .bottom select {
    flex: 1;
    min-width: 0;
    min-height: 24px;
    padding: 1px 20px 1px 8px;
    font-size: 11px;
    background-color: transparent;
    border-color: transparent;
  }

  .bottom select:hover {
    background-color: var(--bg-active);
  }
</style>
