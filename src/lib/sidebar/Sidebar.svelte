<!-- Database picker + filterable tree of tables/views/collections for one session. -->
<script lang="ts">
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import Icon, { type IconName } from '../ui/Icon.svelte'
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

  const KIND: Record<TableKind, { icon: IconName; label: string; color: string }> = {
    table: { icon: 'table', label: 'Table', color: 'var(--kind-table)' },
    view: { icon: 'view', label: 'View', color: 'var(--kind-view)' },
    materializedView: { icon: 'matview', label: 'Materialized view', color: 'var(--kind-view)' },
    foreignTable: { icon: 'table', label: 'Foreign table', color: 'var(--kind-other)' },
    virtual: { icon: 'table', label: 'Virtual table', color: 'var(--kind-other)' },
    sequence: { icon: 'sequence', label: 'Sequence', color: 'var(--kind-other)' },
    collection: { icon: 'collection', label: 'Collection', color: 'var(--kind-collection)' },
    timeSeries: { icon: 'collection', label: 'Time series', color: 'var(--kind-collection)' },
    system: { icon: 'table', label: 'System', color: 'var(--kind-other)' },
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
      class="icon refresh"
      class:spin={entry.loading}
      title={entry.loading ? 'Refreshing schema…' : 'Refresh schema'}
      aria-label="Refresh schema"
      onclick={() => sessions.refreshSchema(session.sessionId, database)}><Icon name="refresh" /></button
    >
  </div>
  <div class="filter">
    <span class="search-icon"><Icon name="search" size={12} /></span>
    <input type="search" placeholder="Filter" bind:value={filter} spellcheck="false" />
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
          <span class="chev" class:open={!isCollapsed(g.schema)}><Icon name="chevron-right" size={10} strokeWidth={2} /></span>
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
            <span class="kind" style:color={k.color} title={k.label}><Icon name={k.icon} size={14} /></span>
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
    --kind-table: #3b82c4;
    --kind-view: #8b5cf6;
    --kind-collection: #2e9e4f;
    --kind-other: var(--fg-muted);
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-sidebar);
    min-width: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 8px 4px;
  }

  .top select {
    flex: 1;
    font-weight: 500;
  }

  .refresh.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .filter {
    position: relative;
    padding: 4px 8px 8px;
  }

  .search-icon {
    position: absolute;
    left: 16px;
    top: 11px;
    color: var(--fg-faint);
    pointer-events: none;
  }

  .filter input {
    width: 100%;
    padding-left: 25px;
    background: var(--bg-active);
    border-color: transparent;
  }

  .filter input:focus {
    background: var(--bg-input);
  }

  .tree {
    flex: 1;
    overflow: auto;
    padding: 0 6px 8px;
  }

  .note {
    padding: 8px;
    font-size: 12px;
    word-break: break-word;
  }

  .schema {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 4px;
    width: 100%;
    min-height: 24px;
    text-align: left;
    background: transparent;
    border: none;
    box-shadow: none;
    padding: 2px 6px;
    margin-top: 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--fg-muted);
  }

  .schema:hover:not(:disabled) {
    background: transparent;
    color: var(--fg);
  }

  .chev {
    display: inline-flex;
    transition: transform 0.12s;
    opacity: 0.8;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .count {
    margin-left: auto;
    font-weight: 400;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 24px;
    padding: 0 8px;
    border-radius: 6px;
    white-space: nowrap;
  }

  .row.indent {
    padding-left: 18px;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.active {
    background: var(--selection);
  }

  .row.active .name {
    font-weight: 500;
  }

  .kind {
    flex: none;
    display: inline-flex;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rows {
    flex: none;
    font-size: 11px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
</style>
