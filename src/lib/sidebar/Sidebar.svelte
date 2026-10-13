<!--
  Left sidebar, TablePlus-style: Items / Queries / History, a search field, the
  selected schema's objects as an expandable tree (tables open to show their
  columns), and database/schema pickers at the bottom.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import Icon, { type IconName } from '../ui/Icon.svelte'
  import { api } from '../api'
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
  let expanded = $state<Record<string, boolean>>({})
  let columnInfo = $state<Record<string, { name: string; type: string; pk: boolean }[]>>({})
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
    { title: 'Materialized Views', kinds: ['materializedView'] },
    { title: 'Sequences', kinds: ['sequence'] },
  ]

  const VIEWS: { id: RailView; label: string }[] = [
    { id: 'browser', label: 'Items' },
    { id: 'saved', label: 'Queries' },
    { id: 'history', label: 'History' },
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

  function key(t: TableInfo): string {
    return t.schema ? `${t.schema}.${t.name}` : t.name
  }

  function hasColumns(t: TableInfo): boolean {
    return t.kind !== 'sequence'
  }

  function isActive(t: TableInfo): boolean {
    const a = activeTab
    return a?.kind === 'table' && a.table.name === t.name && (a.table.schema ?? null) === (t.schema ?? null) && a.table.database === t.database
  }

  async function toggleExpand(t: TableInfo) {
    const k = key(t)
    expanded[k] = !expanded[k]
    if (!expanded[k] || columnInfo[k]) return
    try {
      const s = await api.tableStructure(session.sessionId, ref(t))
      columnInfo[k] = s.columns.map((c) => ({ name: c.name, type: c.dataType, pk: c.isPrimaryKey }))
    } catch {
      columnInfo[k] = (entry.snapshot?.columns[k] ?? []).map((name) => ({ name, type: '', pk: false }))
    }
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
    ui.rail = v
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

  let placeholder = $derived(ui.rail === 'history' ? 'Search history…' : ui.rail === 'saved' ? 'Search queries…' : 'Search for item…')
</script>

{#snippet chevron(open: boolean)}
  <span class="chev" class:open><Icon name="chevron-right" size={10} strokeWidth={1.8} /></span>
{/snippet}

<aside class="sidebar">
  <div class="views" role="tablist">
    {#each VIEWS as v (v.id)}
      <button role="tab" aria-selected={ui.rail === v.id} class:on={ui.rail === v.id} onclick={() => setView(v.id)}>{v.label}</button>
    {/each}
  </div>

  <div class="top">
    <label class="search">
      <Icon name="search" size={13} />
      <input type="search" {placeholder} bind:value={filter} spellcheck="false" />
    </label>
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
          {@render chevron(!collapsed[sec.title])}
          {sec.title}
        </button>
        {#if !collapsed[sec.title]}
          {#each sec.tables as t (key(t))}
            {@const k = key(t)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="row"
              class:active={isActive(t)}
              role="treeitem"
              aria-selected={isActive(t)}
              aria-expanded={hasColumns(t) ? !!expanded[k] : undefined}
              tabindex="-1"
              title={`${t.name}${t.estimatedRows != null ? `\n≈ ${formatCount(t.estimatedRows)} rows` : ''}${t.comment ? `\n${t.comment}` : ''}`}
              onclick={() => tabs.openTable(session.sessionId, ref(t), undefined, { preview: true })}
              ondblclick={() => tabs.openTable(session.sessionId, ref(t))}
              oncontextmenu={(e) => {
                e.preventDefault()
                menu = { x: e.clientX, y: e.clientY, items: tableMenu(t) }
              }}
            >
              {#if hasColumns(t)}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <span
                  class="twist"
                  onclick={(e) => {
                    e.stopPropagation()
                    void toggleExpand(t)
                  }}>{@render chevron(!!expanded[k])}</span
                >
              {:else}
                <span class="twist"></span>
              {/if}
              {#if KIND[t.kind] === 'table'}
                <svg class="ticon" width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
                  <rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="var(--table-icon-fill)" stroke="var(--table-icon)" stroke-width="1" />
                  <path d="M2 3h12v3H2z" fill="var(--table-icon)" />
                  <path d="M1.5 9.5h13M6 6v7.5M10.5 6v7.5" stroke="var(--table-icon)" stroke-width="1" fill="none" />
                </svg>
              {:else}
                <span class="kicon"><Icon name={KIND[t.kind] ?? 'table'} size={14} /></span>
              {/if}
              <span class="name">{#if showSchemaInName && t.schema}<span class="dim">{t.schema}.</span>{/if}{t.name}</span>
            </div>
            {#if expanded[k]}
              {@const cols = columnInfo[k] ?? (entry.snapshot?.columns[k] ?? []).map((name) => ({ name, type: '', pk: false }))}
              {#each cols as c (c.name)}
                <div class="col" title={c.type ? `${c.name} ${c.type}` : c.name}>
                  <span class="cicon" class:pk={c.pk}>{#if c.pk}<Icon name="key" size={11} />{:else}<Icon name="columns" size={11} />{/if}</span>
                  <span class="name">{c.name}</span>
                  {#if c.type}<span class="ctype">{c.type}</span>{/if}
                </div>
              {:else}
                <div class="col dim">Loading…</div>
              {/each}
            {/if}
          {/each}
        {/if}
      {/each}
      {#if entry.error && entry.snapshot}
        <div class="note error-text" title={entry.error}>Refresh failed: {entry.error}</div>
      {/if}
    {:else if ui.rail === 'history'}
      {#if history.entries.length > 0}
        <div class="list-head">
          <span>{history.entries.length} queries</span>
          <button class="clear" onclick={() => history.clear()}>Clear</button>
        </div>
      {/if}
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
    <button class="plus" title="New query" aria-label="New query" onclick={() => tabs.newQuery(session.sessionId, database)}>
      <Icon name="plus" size={13} />
    </button>
    {#if session.databases.length > 1}
      <select value={database} onchange={(e) => sessions.setDatabase(session.sessionId, e.currentTarget.value)} title="Database" aria-label="Database">
        {#each session.databases as db (db)}
          <option value={db}>{db}</option>
        {/each}
      </select>
    {/if}
    {#if schemaNames.length > 1 || (schemaNames.length === 1 && schemaNames[0] !== '')}
      <select value={schema} onchange={(e) => (chosenSchema = e.currentTarget.value)} title="Schema" aria-label="Schema">
        {#each schemaNames as name (name)}
          <option value={name}>{name || '(default)'}</option>
        {/each}
      </select>
    {:else if session.databases.length <= 1}
      <span class="dbname">{database || '(default)'}</span>
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

  .views {
    display: flex;
    justify-content: center;
    gap: 4px;
    padding: 8px 8px 4px;
  }

  .views button {
    min-height: 24px;
    padding: 0 12px;
    border: none;
    border-radius: 6px;
    background: none;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .views button:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .views button.on,
  .views button.on:hover:not(:disabled) {
    background: var(--bg-active);
    color: var(--fg);
  }

  .top {
    padding: 4px 8px 6px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 0.5px solid var(--border-input);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--fg-faint);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
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
    font-size: 13px;
  }

  .search input:focus {
    box-shadow: none;
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 2px 6px 10px;
  }

  .note {
    padding: 6px 8px;
    font-size: 12px;
    color: var(--fg-muted);
    word-break: break-word;
  }

  .section {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 4px;
    width: 100%;
    height: 22px;
    min-height: 22px;
    padding: 0 2px;
    border: none;
    background: none;
    box-shadow: none;
    font-family: var(--mono);
    font-size: 13px;
    color: var(--fg);
  }

  .section:hover:not(:disabled) {
    background: none;
  }

  .chev {
    display: inline-flex;
    width: 12px;
    justify-content: center;
    color: var(--fg-muted);
    transition: transform 0.12s;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 6px 0 14px;
    border-radius: 5px;
    font-family: var(--mono);
    font-size: 13px;
    color: var(--fg-cell);
    outline: none;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.active,
  .row.active:hover {
    background: var(--accent-solid);
    color: var(--accent-solid-fg);
  }

  .row.active .chev {
    color: inherit;
  }

  .twist {
    display: inline-flex;
    flex: none;
    width: 14px;
    height: 100%;
    align-items: center;
    justify-content: center;
  }

  .ticon {
    flex: none;
    margin-right: 3px;
  }

  .kicon {
    display: inline-flex;
    flex: none;
    margin-right: 3px;
    color: var(--table-icon);
  }

  .row.active .kicon {
    color: inherit;
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

  .col {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 6px 0 50px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg-cell);
  }

  .cicon {
    display: inline-flex;
    flex: none;
    color: var(--fg-faint);
  }

  .cicon.pk {
    color: var(--warning);
  }

  .ctype {
    flex: none;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-faint);
    font-size: 11px;
  }

  .list-head {
    display: flex;
    align-items: center;
    padding: 2px 8px 4px;
    font-size: 11px;
    color: var(--fg-muted);
  }

  .clear {
    margin-left: auto;
    min-height: 0;
    padding: 0 4px;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
    color: var(--fg-muted);
  }

  .clear:hover:not(:disabled) {
    background: none;
    color: var(--fg);
  }

  .item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    width: 100%;
    min-height: 0;
    padding: 6px 8px;
    border: none;
    border-radius: 5px;
    background: none;
    box-shadow: none;
    text-align: left;
  }

  .item:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .item code {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg-cell);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item code.err {
    color: var(--danger);
  }

  .item small {
    font-size: 11px;
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
    height: 38px;
    padding: 0 8px;
    border-top: 0.5px solid var(--border-strong);
    flex: none;
  }

  .plus {
    width: 26px;
    min-width: 26px;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .plus:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .bottom select {
    flex: 1;
    min-width: 0;
    min-height: 24px;
    height: 24px;
    padding: 0 22px 0 8px;
    font-size: 12px;
    background-color: var(--bg-active);
    border-color: transparent;
  }

  .dbname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 8px;
    line-height: 24px;
    border-radius: 6px;
    background: var(--bg-active);
    font-size: 12px;
  }
</style>
