<!--
  Left explorer: the active connection card, then (by rail view) the database tree,
  query history or saved queries.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import EngineBadge from '../ui/EngineBadge.svelte'
  import Icon, { type IconName } from '../ui/Icon.svelte'
  import { history } from '../stores/history.svelte'
  import { saved } from '../stores/saved.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo, TableInfo, TableKind, TableRef } from '../types'
  import { ENGINE_LABEL } from '../util/env'
  import { compactCount, formatCount } from '../util/format'
  import { plainName, selectStarter } from '../util/sql'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let filter = $state('')
  let collapsed = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)

  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  let activeTab = $derived(tabs.active(session.sessionId))
  let version = $derived(
    session.serverVersion.toLowerCase().startsWith(ENGINE_LABEL[session.config.engine].toLowerCase())
      ? session.serverVersion
      : `${ENGINE_LABEL[session.config.engine]} ${session.serverVersion.replace(/-(MySQL|MariaDB).*$/i, '')}`,
  )

  const KIND: Record<TableKind, { icon: IconName; group: string }> = {
    table: { icon: 'table', group: 'Tables' },
    virtual: { icon: 'table', group: 'Tables' },
    foreignTable: { icon: 'table', group: 'Tables' },
    system: { icon: 'table', group: 'Tables' },
    collection: { icon: 'collection', group: 'Collections' },
    timeSeries: { icon: 'collection', group: 'Collections' },
    view: { icon: 'view', group: 'Views' },
    materializedView: { icon: 'matview', group: 'Materialized views' },
    sequence: { icon: 'sequence', group: 'Sequences' },
  }
  const GROUP_ORDER = ['Tables', 'Collections', 'Views', 'Materialized views', 'Sequences']

  // schema → kind group → tables, filtered by the search box.
  let tree = $derived.by(() => {
    const snap = entry.snapshot
    if (!snap) return []
    const q = filter.trim().toLowerCase()
    const rank = (x: string) => (x === 'public' || x === '' ? 0 : /^(pg_|information_schema)/.test(x) ? 2 : 1)
    return Object.entries(snap.schemas)
      .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
      .map(([schema, list]) => {
        const tables = q ? list.filter((t) => t.name.toLowerCase().includes(q)) : list
        const groups = GROUP_ORDER.map((g) => ({
          name: g,
          tables: tables.filter((t) => (KIND[t.kind] ?? KIND.table).group === g).sort((a, b) => a.name.localeCompare(b.name)),
        })).filter((g) => g.tables.length > 0)
        return { schema, total: list.length, count: tables.length, groups }
      })
      .filter((s) => !q || s.count > 0)
  })

  let singleSchema = $derived(tree.length === 1 && tree[0].schema === '')

  function key(...parts: string[]) {
    return parts.join('\u0000')
  }

  function isCollapsed(k: string, defaultCollapsed = false): boolean {
    if (filter.trim()) return false
    return collapsed[k] ?? defaultCollapsed
  }

  function toggle(k: string, defaultCollapsed = false) {
    collapsed[k] = !isCollapsed(k, defaultCollapsed)
  }

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

  function connectionMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const items: MenuItem[] = [
      ...sessions.list.map((s) => ({
        label: s.sessionId === session.sessionId ? `✓  ${s.config.name}` : `    ${s.config.name}`,
        onclick: () => sessions.activate(s.sessionId),
      })),
      { separator: true },
      ...session.databases.map((db) => ({
        label: db === database ? `✓  ${db}` : `    ${db}`,
        onclick: () => sessions.setDatabase(session.sessionId, db),
      })),
      { separator: true },
      { label: 'Refresh schema', onclick: () => sessions.refreshSchema(session.sessionId, database) },
      { label: 'Manage connections…', onclick: () => (sessions.view = 'connections') },
      { label: 'Disconnect', danger: true, onclick: () => sessions.close(session.sessionId) },
    ]
    menu = { x: r.left, y: r.bottom + 4, items }
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

  let placeholder = $derived(ui.rail === 'history' ? 'Filter history…' : ui.rail === 'saved' ? 'Filter saved queries…' : 'Filter objects…')
</script>

<aside class="explorer">
  <div class="head">
    <div class="overline">Active connection</div>
    <button class="connection" onclick={connectionMenu} title="Switch connection or database">
      <EngineBadge engine={session.config.engine} size={30} />
      <span class="cm">
        <b>{session.config.name}</b>
        <small><i class="live-dot"></i> {version} · {database || 'Connected'}</small>
      </span>
      <Icon name="chevron-down" size={13} />
    </button>
  </div>

  <label class="search">
    <Icon name="search" size={13} />
    <input type="search" {placeholder} bind:value={filter} spellcheck="false" />
  </label>

  {#if ui.rail === 'browser'}
    <div class="caption">
      <span>Database explorer</span>
      <button class="mini" title={entry.loading ? 'Refreshing…' : 'Refresh schema'} aria-label="Refresh schema" class:spin={entry.loading} onclick={() => sessions.refreshSchema(session.sessionId, database)}>
        <Icon name="refresh" size={12} />
      </button>
    </div>

    <div class="tree" role="tree">
      {#if !entry.snapshot && entry.loading}
        <div class="note">Loading schema…</div>
      {:else if !entry.snapshot && entry.error}
        <div class="note error-text">{entry.error}</div>
      {:else if entry.snapshot && tree.length === 0}
        <div class="note">{filter ? 'No matching objects.' : 'This database is empty.'}</div>
      {/if}

      {#each tree as s (s.schema)}
        {@const sk = key('s', s.schema)}
        {@const sDefault = !singleSchema && s.schema !== 'public' && tree.length > 1 && s !== tree[0]}
        {#if !singleSchema}
          <button class="group" onclick={() => toggle(sk, sDefault)} aria-expanded={!isCollapsed(sk, sDefault)}>
            <Icon name={isCollapsed(sk, sDefault) ? 'chevron-right' : 'chevron-down'} size={12} strokeWidth={1.8} />
            <strong>{s.schema || '(default)'}</strong>
            <span class="count">{s.count}</span>
          </button>
        {/if}
        {#if singleSchema || !isCollapsed(sk, sDefault)}
          {#each s.groups as g (g.name)}
            {@const gk = key('g', s.schema, g.name)}
            <button class="group sub" class:flat={singleSchema} onclick={() => toggle(gk)} aria-expanded={!isCollapsed(gk)}>
              <Icon name={isCollapsed(gk) ? 'chevron-right' : 'chevron-down'} size={12} strokeWidth={1.8} />
              <strong>{g.name}</strong>
              <span class="count">{g.tables.length}</span>
            </button>
            {#if !isCollapsed(gk)}
              {#each g.tables as t (key(t.schema ?? '', t.name))}
                <button
                  class="entry"
                  class:flat={singleSchema}
                  class:active={isActive(t)}
                  title={t.comment ? `${t.name}\n${t.comment}` : t.name}
                  onclick={() => tabs.openTable(session.sessionId, ref(t))}
                  oncontextmenu={(e) => {
                    e.preventDefault()
                    menu = { x: e.clientX, y: e.clientY, items: tableMenu(t) }
                  }}
                >
                  <Icon name={(KIND[t.kind] ?? KIND.table).icon} size={14} />
                  <span class="name">{t.name}</span>
                  {#if t.estimatedRows != null}
                    <span class="tiny" title="≈ {formatCount(t.estimatedRows)} rows">{compactCount(t.estimatedRows)}</span>
                  {/if}
                </button>
              {/each}
            {/if}
          {/each}
        {/if}
      {/each}
      {#if entry.error && entry.snapshot}
        <div class="note error-text" title={entry.error}>Refresh failed: {entry.error}</div>
      {/if}
    </div>
  {:else if ui.rail === 'history'}
    <div class="caption">
      <span>Query history</span>
      {#if history.entries.length > 0}
        <button class="mini text" onclick={() => history.clear()}>Clear</button>
      {/if}
    </div>
    <div class="tree">
      {#if historyList.length === 0}
        <div class="note">{filter ? 'No matching queries.' : 'Queries you run appear here.'}</div>
      {/if}
      {#each historyList as h (h.id)}
        <button class="item" onclick={() => openSql(h.sql, h.database)} title={h.sql}>
          <code class:err={!h.ok}>{h.sql.split('\n')[0]}</code>
          <small>
            {#if !h.ok}<span class="bad">failed</span> · {/if}{h.connectionName} · {ago(h.at)}{h.elapsedMs != null ? ` · ${Math.round(h.elapsedMs)} ms` : ''}
          </small>
        </button>
      {/each}
    </div>
  {:else}
    <div class="caption"><span>Saved queries</span></div>
    <div class="tree">
      {#if savedList.length === 0}
        <div class="note">{filter ? 'No matching queries.' : 'Save a query from the SQL editor (star) to keep it here.'}</div>
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
          <span class="saved-name"><Icon name="star" size={12} /> {q.name}</span>
          <code>{q.sql.split('\n')[0]}</code>
        </button>
      {/each}
    </div>
  {/if}

  <div class="bottom">
    <button class="link" class:on={ui.rail === 'saved'} onclick={() => (ui.rail = ui.rail === 'saved' ? 'browser' : 'saved')}>
      <Icon name="star" size={15} /> Saved queries <span class="n">{saved.list.length}</span>
    </button>
    <button class="link" class:on={ui.rail === 'history'} onclick={() => (ui.rail = ui.rail === 'history' ? 'browser' : 'history')}>
      <Icon name="clock" size={15} /> Query history <span class="n">{history.entries.length}</span>
    </button>
  </div>
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--bg-sidebar);
  }

  .head {
    padding: 20px 16px 12px;
  }

  .connection {
    width: 100%;
    justify-content: flex-start;
    gap: 10px;
    margin-top: 13px;
    padding: 10px;
    min-height: 0;
    text-align: left;
    background: var(--bg-input);
    border: 1px solid var(--border-input);
    border-radius: 9px;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .connection:hover:not(:disabled) {
    background: var(--bg-input);
    border-color: var(--fg-faint);
  }

  .cm {
    flex: 1;
    min-width: 0;
  }

  .cm b {
    display: block;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: -0.012em;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cm small {
    display: block;
    margin-top: 4px;
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cm .live-dot {
    margin-right: 3px;
    vertical-align: middle;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 33px;
    margin: 0 16px 12px;
    padding: 0 9px;
    border: 1px solid var(--border-strong);
    border-radius: 7px;
    background: var(--bg-input);
    color: var(--fg-muted);
  }

  .search input {
    flex: 1;
    width: 100%;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    background: transparent;
    box-shadow: none;
    font-size: 11px;
  }

  .search input:focus {
    box-shadow: none;
  }

  .search:focus-within {
    border-color: var(--accent);
  }

  .caption {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px 8px;
    font-size: 10px;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }

  .mini {
    width: 20px;
    min-width: 20px;
    height: 20px;
    min-height: 20px;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    color: var(--fg-faint);
  }

  .mini.text {
    width: auto;
    padding: 0 4px;
    font-size: 10px;
    letter-spacing: 0;
    text-transform: none;
  }

  .mini:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .mini.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .tree {
    flex: 1;
    overflow-y: auto;
    padding: 0 9px 8px;
  }

  .note {
    padding: 8px 9px;
    font-size: 11px;
    color: var(--fg-muted);
    word-break: break-word;
  }

  .group {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 7px;
    width: 100%;
    min-height: 0;
    padding: 9px 9px 7px;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
    color: var(--fg-muted);
  }

  .group.sub {
    padding-left: 19px;
  }

  .group.sub.flat {
    padding-left: 9px;
  }

  .group:hover:not(:disabled) {
    background: none;
    color: var(--fg);
  }

  .group strong {
    flex: 1;
    text-align: left;
    font-weight: 600;
    color: color-mix(in srgb, var(--fg) 70%, var(--fg-muted));
  }

  .count {
    padding: 2px 5px;
    border-radius: 4px;
    font-size: 10px;
    color: var(--fg-faint);
    background: var(--bg-active);
  }

  .entry {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 9px;
    width: 100%;
    height: 31px;
    min-height: 31px;
    margin: 1px 0;
    padding: 0 10px 0 31px;
    border: none;
    border-radius: 6px;
    background: none;
    box-shadow: none;
    text-align: left;
    font-size: 11px;
    color: color-mix(in srgb, var(--fg) 72%, var(--fg-muted));
  }

  .entry.flat {
    padding-left: 21px;
  }

  .entry :global(svg) {
    color: var(--fg-faint);
  }

  .entry:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .entry.active,
  .entry.active:hover:not(:disabled) {
    background: var(--accent-solid);
    color: var(--accent-solid-fg);
  }

  .entry.active :global(svg) {
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .entry.active :global(svg) {
    color: color-mix(in srgb, var(--accent) 60%, #2d3a12);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tiny {
    font-size: 9px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }

  .entry.active .tiny {
    color: inherit;
    opacity: 0.7;
  }

  .item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
    width: 100%;
    min-height: 0;
    margin: 1px 0;
    padding: 8px 10px;
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

  .saved-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--fg);
  }

  .saved-name :global(svg) {
    color: var(--warning);
  }

  .bottom {
    padding: 12px 14px;
    border-top: 1px solid var(--border);
  }

  .link {
    justify-content: flex-start;
    gap: 10px;
    width: 100%;
    min-height: 0;
    padding: 8px 5px;
    border: none;
    background: none;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .link:hover:not(:disabled),
  .link.on {
    background: none;
    color: var(--fg);
  }

  .n {
    margin-left: auto;
    color: var(--fg-faint);
  }
</style>
