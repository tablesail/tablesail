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

  // Schemas in this database, public (or the engine's single "") first, system-ish last.
  let schemaNames = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return [] as string[]
    const rank = (x: string) => (x === 'public' || x === '' ? 0 : /^(pg_|information_schema)/.test(x) ? 2 : 1)
    return Object.keys(snap.schemas).sort((x, y) => rank(x) - rank(y) || x.localeCompare(y))
  })

  let chosenSchema = $state<string | null>(null)
  let schema = $derived(
    chosenSchema !== null && schemaNames.includes(chosenSchema) ? chosenSchema : (schemaNames[0] ?? ''),
  )

  const SECTIONS: { title: string; kinds: TableKind[] }[] = [
    { title: 'Tables', kinds: ['table', 'virtual', 'foreignTable', 'system'] },
    { title: 'Collections', kinds: ['collection', 'timeSeries'] },
    { title: 'Views', kinds: ['view'] },
    { title: 'Materialized views', kinds: ['materializedView'] },
    { title: 'Sequences', kinds: ['sequence'] },
  ]

  let sections = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return []
    const q = filter.trim().toLowerCase()
    // A search looks across every schema; otherwise show the selected one.
    const pool = q ? Object.values(snap.schemas).flat() : (snap.schemas[schema] ?? [])
    const list = q ? pool.filter((t) => t.name.toLowerCase().includes(q)) : pool
    return SECTIONS.map((sec) => ({
      title: sec.title,
      tables: list.filter((t) => sec.kinds.includes(t.kind)).sort((x, y) => x.name.localeCompare(y.name)),
    })).filter((sec) => sec.tables.length > 0)
  })

  let total = $derived(sections.reduce((n, sec) => n + sec.tables.length, 0))
  let showSchemaInName = $derived(filter.trim() !== '' && schemaNames.length > 1)

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

  function toggle(title: string) {
    collapsed[title] = !collapsed[title]
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
  <div class="search">
    <span class="search-icon"><Icon name="search" size={12} /></span>
    <input type="search" placeholder="Search for item…" bind:value={filter} spellcheck="false" />
  </div>

  <div class="tree" role="tree">
    {#if !entry.snapshot && entry.loading}
      <div class="note muted">Loading schema…</div>
    {:else if !entry.snapshot && entry.error}
      <div class="note error-text">{entry.error}</div>
    {:else if entry.snapshot && total === 0}
      <div class="note muted">{filter ? 'No matching items.' : 'Nothing here yet.'}</div>
    {/if}

    {#each sections as sec (sec.title)}
      <button class="section" onclick={() => toggle(sec.title)} aria-expanded={!collapsed[sec.title]}>
        {sec.title}
        <span class="count">{sec.tables.length}</span>
        <span class="chev" class:open={!collapsed[sec.title]}><Icon name="chevron-right" size={9} strokeWidth={2.2} /></span>
      </button>
      {#if !collapsed[sec.title]}
        {#each sec.tables as t (t.schema + '\u0000' + t.name)}
          {@const k = KIND[t.kind] ?? KIND.table}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="row"
            class:active={isActive(t)}
            role="treeitem"
            aria-selected={isActive(t)}
            tabindex="-1"
            title={t.comment ? `${t.name}\n${t.comment}` : t.name}
            onclick={() => open(t)}
            oncontextmenu={(e) => onContext(e, t)}
          >
            <span class="kind" style:color={k.color} title={k.label}><Icon name={k.icon} size={14} /></span>
            <span class="name">{#if showSchemaInName && t.schema}<span class="muted">{t.schema}.</span>{/if}{t.name}</span>
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

  <div class="bottom">
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
    {#if schemaNames.length > 1 || (schemaNames.length === 1 && schemaNames[0] !== '')}
      <select value={schema} onchange={(e) => (chosenSchema = e.currentTarget.value)} title="Schema" aria-label="Schema">
        {#each schemaNames as name (name)}
          <option value={name}>{name || '(default)'}</option>
        {/each}
      </select>
    {/if}
  </div>
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

  .search {
    position: relative;
    padding: 8px 8px 6px;
  }

  .search-icon {
    position: absolute;
    left: 16px;
    top: 50%;
    transform: translateY(-40%);
    color: var(--fg-faint);
    pointer-events: none;
  }

  .search input {
    width: 100%;
    min-height: 24px;
    padding: 2px 8px 2px 25px;
    font-size: 12px;
    border-radius: 6px;
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

  .section {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 5px;
    width: 100%;
    min-height: 22px;
    height: 22px;
    margin-top: 6px;
    padding: 0 6px;
    background: transparent;
    border: none;
    box-shadow: none;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }

  .section:hover:not(:disabled) {
    background: transparent;
    color: var(--fg-muted);
  }

  .count {
    font-weight: 500;
    letter-spacing: 0;
  }

  .chev {
    display: inline-flex;
    margin-left: auto;
    opacity: 0;
    transition:
      transform 0.12s,
      opacity 0.12s;
  }

  .section:hover .chev {
    opacity: 1;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 6px 0 8px;
    border-radius: 5px;
    white-space: nowrap;
    font-size: 12.5px;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.active {
    background: var(--accent-solid);
    color: #fff;
  }

  .row.active .kind,
  .row.active .rows,
  .row.active .muted {
    color: inherit !important;
    opacity: 0.85;
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
    font-size: 10.5px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }

  .bottom {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    background: var(--bg-chrome);
    flex: none;
  }

  .bottom select {
    flex: 1;
    min-width: 0;
    min-height: 22px;
    padding: 1px 20px 1px 7px;
    font-size: 12px;
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
  }

  .bottom select:hover {
    background-color: var(--bg-hover);
  }
</style>
