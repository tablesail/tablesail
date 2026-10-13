<!--
  Window toolbar, TablePlus-style. On macOS it sits inside the title bar
  (titleBarStyle: Overlay), so it leaves room for the traffic lights and is the
  window's drag region. Buttons come in small capsule groups around a wide
  status pill tinted with the connection's colour.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import Icon from '../ui/Icon.svelte'
  import { inspector } from '../stores/inspector.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { ENGINE_LABEL, envColor } from '../util/env'
  import { isMac, modLabel } from '../util/format'

  interface Props {
    session: SessionInfo | null
  }

  let { session }: Props = $props()

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)
  let database = $derived(session ? sessions.database(session.sessionId) : '')
  let entry = $derived(session ? sessions.schemaEntry(session.sessionId, database) : null)
  let activeTab = $derived(session ? tabs.active(session.sessionId) : null)
  let isProd = $derived(session?.config.env === 'production')
  let version = $derived.by(() => {
    if (!session) return ''
    const label = ENGINE_LABEL[session.config.engine]
    return session.serverVersion.toLowerCase().startsWith(label.toLowerCase())
      ? session.serverVersion
      : `${label} ${session.serverVersion.replace(/-(MySQL|MariaDB).*$/i, '')}`
  })
  let location = $derived.by(() => {
    const t = activeTab
    if (!t) return ''
    if (t.kind === 'table') return [t.table.schema, t.table.name].filter(Boolean).join('.')
    return t.title
  })

  const envLabel = { local: 'local', development: 'dev', staging: 'staging', production: 'production' }

  function openAt(e: MouseEvent, items: MenuItem[], alignRight = false) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    menu = { x: alignRight ? r.right - 210 : r.left, y: r.bottom + 6, items }
  }

  let appItems = $derived<MenuItem[]>([
    { label: theme.isDark ? 'Light appearance' : 'Dark appearance', hint: `${modLabel}⇧L`, onclick: () => theme.toggle() },
    { label: 'Use system appearance', disabled: theme.mode === 'system', onclick: () => theme.set('system') },
    { separator: true },
    { label: 'Query history', onclick: () => (ui.rail = 'history') },
    { label: 'Saved queries', onclick: () => (ui.rail = 'saved') },
    { separator: true },
    { label: 'Keyboard shortcuts', hint: `${modLabel}/`, onclick: () => (ui.shortcutsHelp = true) },
    { label: 'Settings…', hint: `${modLabel},`, onclick: () => (ui.settings = true) },
    { label: 'About / Check for updates…', onclick: () => (ui.about = true) },
  ])

  let pillItems = $derived<MenuItem[]>(
    session
      ? [
          ...sessions.list.map((s) => ({
            label: s.sessionId === session.sessionId ? `✓  ${s.config.name}` : `    ${s.config.name}`,
            onclick: () => sessions.activate(s.sessionId),
          })),
          { separator: true },
          { label: 'Open another connection…', onclick: () => (sessions.view = 'connections') },
          { label: 'Disconnect', danger: true, onclick: () => sessions.close(session.sessionId) },
        ]
      : [],
  )

  let dbItems = $derived<MenuItem[]>(
    session
      ? session.databases.map((db) => ({
          label: db === database ? `✓  ${db}` : `    ${db}`,
          onclick: () => sessions.setDatabase(session.sessionId, db),
        }))
      : [],
  )

  function openInspector(tab: 'details' | 'assistant') {
    if ((inspector.open || inspector.opening) && inspector.tab === tab) inspector.close()
    else {
      inspector.tab = tab
      inspector.open = true
    }
  }
</script>

<!-- Toolbar buttons don't keep focus after a click (Space would toggle them again). -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="toolbar"
  class:mac={isMac}
  class:prod={isProd}
  style:--env={session ? envColor(session.config) : 'var(--accent)'}
  data-tauri-drag-region
  onmousedown={(e) => (e.target as HTMLElement).closest('button') && e.preventDefault()}
>
  {#if session}
    <div class="grp">
      <button class="tb" class:on={!ui.sidebarHidden} title="Toggle sidebar" aria-label="Toggle sidebar" onclick={() => (ui.sidebarHidden = !ui.sidebarHidden)}>
        <Icon name="sidebar" size={16} />
      </button>
      <button class="tb" title="Connections" aria-label="Connections" onclick={() => (sessions.view = 'connections')}>
        <Icon name="server" size={15} />
      </button>
    </div>
    <div class="grp">
      <button
        class="tb"
        class:spin={entry?.loading}
        title={entry?.loading ? 'Refreshing…' : 'Refresh (⌘R)'}
        aria-label="Refresh schema"
        onclick={() => session && sessions.refreshSchema(session.sessionId, database)}
      >
        <Icon name="refresh" size={15} />
      </button>
      {#if session.config.readOnly}
        <span class="tb lock" title="Read-only connection"><Icon name="lock" size={14} /></span>
      {/if}
    </div>
    <div class="grp">
      <button class="tb" title="Switch database" aria-label="Switch database" disabled={dbItems.length < 2} onclick={(e) => openAt(e, dbItems)}>
        <Icon name="database" size={15} />
      </button>
      <button class="tb txt" title="New SQL query ({modLabel}T)" aria-label="New SQL query" onclick={() => session && tabs.newQuery(session.sessionId, database)}>
        SQL
      </button>
    </div>

    <button class="pill" title="Switch connection" onclick={(e) => openAt(e, pillItems)}>
      <span class="ptext">
        <span>{version}</span>
        <span class="colon">:</span>
        <span>{session.config.name}</span>
        {#if database}<span class="colon">:</span><span>{database}</span>{/if}
        {#if location}<span class="colon">:</span><span>{location}</span>{/if}
      </span>
      <span class="tag">{envLabel[session.config.env]}</span>
    </button>

    <div class="grp">
      <button class="tb" title="Search tables and commands ({modLabel}K)" aria-label="Search" onclick={() => (ui.palette = true)}>
        <Icon name="search" size={15} />
      </button>
      <button class="tb" title="Settings" aria-label="Menu" onclick={(e) => openAt(e, appItems, true)}>
        <Icon name="more" size={15} />
      </button>
    </div>
    <div class="grp">
      <button class="tb" class:on={ui.console} title="Query log" aria-label="Toggle query log" onclick={() => (ui.console = !ui.console)}>
        <Icon name="panel-bottom" size={16} />
      </button>
      <button class="tb" class:on={inspector.open && inspector.tab === 'assistant'} title="AI assistant" aria-label="AI assistant" onclick={() => openInspector('assistant')}>
        <Icon name="sparkle" size={16} />
      </button>
      <button class="tb" class:on={inspector.open && inspector.tab === 'details'} title="Row details" aria-label="Toggle details panel" onclick={() => openInspector('details')}>
        <Icon name="panel" size={16} />
      </button>
    </div>
  {:else}
    <span class="grow" data-tauri-drag-region></span>
    <div class="brand" data-tauri-drag-region>
      <span class="mark"><Icon name="pulse" size={14} strokeWidth={1.6} /></span>
      <b>table<span>soft</span></b>
    </div>
    <span class="grow" data-tauri-drag-region></span>
    <div class="grp">
      <button class="tb" title="Search ({modLabel}K)" aria-label="Search" onclick={() => (ui.palette = true)}>
        <Icon name="search" size={15} />
      </button>
      <button class="tb" aria-label="Menu" title="Menu" onclick={(e) => openAt(e, appItems, true)}>
        <Icon name="more" size={15} />
      </button>
    </div>
  {/if}
</header>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .toolbar {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 46px;
    padding: 0 10px;
    flex: none;
    background: var(--bg-chrome);
    border-bottom: 0.5px solid var(--border-strong);
  }

  /* Room for the macOS traffic lights (titleBarStyle: Overlay). */
  .toolbar.mac {
    padding-left: 80px;
  }

  .grow {
    flex: 1;
  }

  /* Capsule button group. */
  .grp {
    display: flex;
    align-items: center;
    flex: none;
    height: 28px;
    padding: 0 1px;
    border: 0.5px solid var(--border-input);
    border-radius: 7px;
    background: var(--bg-chip);
    box-shadow: var(--btn-shadow);
  }

  .tb {
    gap: 5px;
    width: 32px;
    height: 26px;
    min-height: 26px;
    padding: 0;
    border: none;
    background: transparent;
    box-shadow: none;
    border-radius: 6px;
    color: var(--fg-muted);
  }

  .tb.txt {
    width: auto;
    padding: 0 8px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }

  .tb:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .tb.on,
  .tb.on:hover:not(:disabled) {
    color: var(--fg);
  }

  .tb.lock {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--warning);
  }

  .tb.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .pill {
    flex: 1;
    min-width: 120px;
    justify-content: flex-start;
    gap: 8px;
    height: 30px;
    min-height: 30px;
    padding: 0 6px 0 14px;
    border-radius: 7px;
    border: 0.5px solid color-mix(in srgb, var(--env) 45%, transparent);
    background: color-mix(in srgb, var(--env) 24%, var(--bg));
    box-shadow: none;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg);
    overflow: hidden;
  }

  .pill:hover:not(:disabled) {
    background: color-mix(in srgb, var(--env) 30%, var(--bg));
  }

  .ptext {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 7px;
    overflow: hidden;
    white-space: nowrap;
  }

  .ptext span {
    flex: none;
  }

  .ptext span:last-child {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .colon {
    opacity: 0.55;
  }

  .tag {
    flex: none;
    height: 18px;
    line-height: 18px;
    padding: 0 7px;
    border-radius: 5px;
    font-family: -apple-system, BlinkMacSystemFont, system-ui, sans-serif;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.02em;
    color: color-mix(in srgb, var(--env) 60%, var(--fg));
    background: color-mix(in srgb, var(--env) 22%, transparent);
  }

  .prod .tag {
    color: #fff;
    background: var(--env);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .mark {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--accent);
    color: var(--accent-fg);
  }

  .brand b {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: -0.03em;
  }

  .brand b span {
    color: color-mix(in srgb, var(--accent) 70%, var(--fg));
  }
</style>
