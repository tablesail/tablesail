<!-- Database picker + filterable tree of tables/views/collections for one session. -->
<script lang="ts">
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo, TableInfo, TableKind, TableRef } from '../types'
  import { compactCount, formatCount } from '../util/format'
  import { plainName, selectStarter } from '../util/sql'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let filter = $state('')
  let collapsed = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; table: TableInfo } | null>(null)

  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  let activeTab = $derived(tabs.active(session.sessionId))

  const KIND: Record<TableKind, { glyph: string; label: string; color: string }> = {
    table: { glyph: 'T', label: 'Table', color: '#3b82c4' },
    view: { glyph: 'V', label: 'View', color: '#8b5cf6' },
    materializedView: { glyph: 'M', label: 'Materialized view', color: '#a855f7' },
    foreignTable: { glyph: 'F', label: 'Foreign table', color: '#0e9aa7' },
    virtual: { glyph: 'T', label: 'Virtual table', color: '#6b7280' },
    sequence: { glyph: '#', label: 'Sequence', color: '#6b7280' },
    collection: { glyph: 'C', label: 'Collection', color: '#2e9e4f' },
    timeSeries: { glyph: 'S', label: 'Time series', color: '#2e9e4f' },
    system: { glyph: 'S', label: 'System', color: '#6b7280' },
  }

  let groups = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return []
    const q = filter.trim().toLowerCase()
    const out: { schema: string; tables: TableInfo[]; total: number }[] = []
    for (const [schema, list] of Object.entries(snap.schemas)) {
      const tables = q ? list.filter((t) => t.name.toLowerCase().includes(q)) : list
      if (tables.length || (!q && list.length === 0)) out.push({ schema, tables, total: list.length })
    }
    out.sort((a, b) => {
      // public first, system-ish schemas last
      const rank = (s: string) => (s === 'public' || s === '' ? 0 : /^(pg_|information_schema)/.test(s) ? 2 : 1)
      return rank(a.schema) - rank(b.schema) || a.schema.localeCompare(b.schema)
    })
    return out
  })

  let showHeaders = $derived(groups.length > 1 || (groups.length === 1 && groups[0].schema !== ''))

  function ref(t: TableInfo): TableRef {
    return { database: t.database, schema: t.schema ?? null, name: t.name }
  }

  function isActive(t: TableInfo): boolean {
    const a = activeTab
    return (
      a?.kind === 'table' &&
      a.table.name === t.name &&
      (a.table.schema ?? null) === (t.schema ?? null) &&
      a.table.database === t.database
    )
  }

  function open(t: TableInfo) {
    tabs.openTable(session.sessionId, ref(t))
  }

  function isCollapsed(schema: string): boolean {
    if (filter.trim()) return false
    return collapsed[schema] ?? (schema !== 'public' && schema !== '' && groups.length > 1 && schema !== defaultSchema())
  }

  function defaultSchema(): string {
    return groups.some((g) => g.schema === 'public') ? 'public' : (groups[0]?.schema ?? '')
  }

  function toggle(schema: string) {
    collapsed[schema] = !isCollapsed(schema)
  }

  function menuItems(t: TableInfo): MenuItem[] {
    const r = ref(t)
    const isTable = t.kind === 'table' || t.kind === 'collection' || t.kind === 'virtual'
    return [
      { label: 'Open', onclick: () => open(t) },
      {
        label: 'New query',
        onclick: () => tabs.newQuery(session.sessionId, database, selectStarter(session.config.engine, r)),
      },
      { separator: true },
      {
        label: 'Export…',
        onclick: () => ui.exportTable(session, { table: r, filters: [], matchAll: true, rawWhere: null, sort: [], columns: [] }),
      },
      { label: 'Import CSV…', disabled: !isTable || session.config.readOnly, onclick: () => ui.importInto(session, r) },
      { separator: true },
      { label: 'Copy name', onclick: () => void navigator.clipboard?.writeText(plainName(r)) },
      { label: 'Refresh', onclick: () => sessions.refreshSchema(session.sessionId, database) },
    ]
  }

  function onContext(e: MouseEvent, t: TableInfo) {
    e.preventDefault()
    menu = { x: e.clientX, y: e.clientY, table: t }
  }
</script>

<div class="sidebar">
  <div class="top">
    <select
      value={database}
      onchange={(e) => sessions.setDatabase(session.sessionId, e.currentTarget.value)}
      title="Database"
      aria-label="Database"
    >
      {#each session.databases as db (db)}
        <option value={db}>{db}</option>
      {/each}
      {#if session.databases.length === 0}
        <option value={database}>{database || '(default)'}</option>
      {/if}
    </select>
    <button
      class="ghost refresh"
      class:spin={entry.loading}
      title="Refresh schema"
      aria-label="Refresh schema"
      onclick={() => sessions.refreshSchema(session.sessionId, database)}>↻</button
    >
  </div>
  <div class="filter">
    <input type="search" placeholder="Filter tables" bind:value={filter} spellcheck="false" />
  </div>

  <div class="tree" role="tree">
    {#if !entry.snapshot && entry.loading}
      <div class="note muted">Loading schema…</div>
    {:else if !entry.snapshot && entry.error}
      <div class="note error-text">{entry.error}</div>
    {:else if groups.length === 0}
      <div class="note muted">{filter ? 'No matching tables.' : 'No tables in this database.'}</div>
    {/if}

    {#each groups as g (g.schema)}
      {#if showHeaders}
        <button class="schema" onclick={() => toggle(g.schema)} aria-expanded={!isCollapsed(g.schema)}>
          <span class="chev" class:open={!isCollapsed(g.schema)}>▸</span>
          {g.schema || '(default)'}
          <span class="count">{g.tables.length}</span>
        </button>
      {/if}
      {#if !showHeaders || !isCollapsed(g.schema)}
        {#each g.tables as t (t.schema + '\u0000' + t.name)}
          {@const k = KIND[t.kind] ?? KIND.table}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="row"
            class:indent={showHeaders}
            class:active={isActive(t)}
            role="treeitem"
            aria-selected={isActive(t)}
            tabindex="-1"
            title={t.comment ? `${t.name}\n${t.comment}` : t.name}
            onclick={() => open(t)}
            oncontextmenu={(e) => onContext(e, t)}
          >
            <span class="kind" style:background={k.color} title={k.label}>{k.glyph}</span>
            <span class="name">{t.name}</span>
            {#if t.estimatedRows != null}
              <span class="rows" title="≈ {formatCount(t.estimatedRows)} rows">{compactCount(t.estimatedRows)}</span>
            {/if}
          </div>
        {/each}
      {/if}
    {/each}
  </div>

  {#if entry.error && entry.snapshot}
    <div class="note error-text" title={entry.error}>Refresh failed: {entry.error}</div>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.table)} onClose={() => (menu = null)} />
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-sidebar);
    min-width: 0;
  }

  .top {
    display: flex;
    gap: 4px;
    padding: 8px 8px 4px;
  }

  .top select {
    flex: 1;
  }

  .refresh {
    padding: 0 8px;
  }

  .refresh.spin {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .filter {
    padding: 4px 8px 6px;
  }

  .filter input {
    width: 100%;
  }

  .tree {
    flex: 1;
    overflow: auto;
    padding: 0 4px 8px;
  }

  .note {
    padding: 8px;
    font-size: 12px;
    word-break: break-word;
  }

  .schema {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    padding: 4px 6px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .chev {
    display: inline-block;
    transition: transform 0.1s;
    font-size: 10px;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .count {
    margin-left: auto;
    font-weight: 400;
    font-size: 11px;
    opacity: 0.7;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 3px 8px;
    border-radius: 5px;
    white-space: nowrap;
  }

  .row.indent {
    padding-left: 20px;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.active {
    background: var(--selection);
  }

  .kind {
    flex: none;
    width: 15px;
    height: 15px;
    border-radius: 3px;
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    line-height: 15px;
    text-align: center;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rows {
    flex: none;
    font-size: 11px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
