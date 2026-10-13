<!--
  Window toolbar, TablePlus-style. On macOS it sits inside the title bar
  (titleBarStyle: Overlay), so it leaves room for the traffic lights and is the
  window's drag region. The centred pill names the connection and database.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import EngineBadge from '../ui/EngineBadge.svelte'
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
  let isProd = $derived(session?.config.env === 'production')
  let version = $derived.by(() => {
    if (!session) return ''
    const label = ENGINE_LABEL[session.config.engine]
    return session.serverVersion.toLowerCase().startsWith(label.toLowerCase())
      ? session.serverVersion
      : `${label} ${session.serverVersion.replace(/-(MySQL|MariaDB).*$/i, '')}`
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
    { label: 'Settings…', onclick: () => (ui.settings = true) },
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
</script>

<header class="toolbar" class:mac={isMac} class:prod={isProd} style:--env={session ? envColor(session.config) : 'var(--accent)'} data-tauri-drag-region>
  <div class="group" data-tauri-drag-region>
    {#if session}
      <button class="tb" title="Connections" aria-label="Connections" onclick={() => (sessions.view = 'connections')}>
        <Icon name="server" size={15} />
      </button>
      <button
        class="tb"
        class:spin={entry?.loading}
        title={entry?.loading ? 'Refreshing…' : 'Refresh schema'}
        aria-label="Refresh schema"
        onclick={() => session && sessions.refreshSchema(session.sessionId, database)}
      >
        <Icon name="refresh" size={15} />
      </button>
    {/if}
  </div>

  {#if session}
    <button class="pill" title="Switch connection" onclick={(e) => openAt(e, pillItems)}>
      <span class="tag">{envLabel[session.config.env]}</span>
      <EngineBadge engine={session.config.engine} size={16} />
      <span class="ver">{version}</span>
      <span class="colon">:</span>
      <b class="name">{session.config.name}</b>
      {#if database}<span class="colon">:</span><span class="db">{database}</span>{/if}
      {#if session.config.readOnly}<span class="lock" title="Read-only"><Icon name="lock" size={11} /></span>{/if}
    </button>
  {:else}
    <div class="brand" data-tauri-drag-region>
      <span class="mark"><Icon name="pulse" size={14} strokeWidth={1.6} /></span>
      <b>table<span>soft</span></b>
    </div>
  {/if}

  <div class="group right" data-tauri-drag-region>
    <button class="tb" title="Search tables and commands ({modLabel}K)" aria-label="Search" onclick={() => (ui.palette = true)}>
      <Icon name="search" size={15} />
    </button>
    {#if session}
      <button class="tb" title="New SQL query ({modLabel}T)" aria-label="New SQL query" onclick={() => session && tabs.newQuery(session.sessionId, database)}>
        <Icon name="code" size={15} /><span>SQL</span>
      </button>
      <button class="tb" class:on={inspector.open} title="Row inspector" aria-label="Toggle row inspector" onclick={() => inspector.toggle()}>
        <Icon name="panel" size={15} />
      </button>
    {/if}
    <button class="tb" aria-label="Menu" title="Menu" onclick={(e) => openAt(e, appItems, true)}>
      <Icon name="more" size={15} />
    </button>
  </div>
</header>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .toolbar {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 10px;
    flex: none;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
  }

  /* Room for the macOS traffic lights (titleBarStyle: Overlay). */
  .toolbar.mac {
    padding-left: 84px;
  }

  .toolbar.prod {
    background: color-mix(in srgb, var(--env) 10%, var(--bg-chrome));
  }

  .group {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .group.right {
    justify-content: flex-end;
  }

  .tb {
    gap: 5px;
    height: 28px;
    min-height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    box-shadow: none;
    border-radius: 7px;
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }

  .tb:hover:not(:disabled),
  .tb.on {
    background: var(--bg-active);
    color: var(--fg);
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
    flex: 0 1 auto;
    min-width: 0;
    max-width: 56%;
    height: 28px;
    min-height: 28px;
    gap: 7px;
    padding: 0 12px 0 5px;
    border-radius: 8px;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    box-shadow: none;
    font-size: 11px;
    color: var(--fg-muted);
    overflow: hidden;
  }

  .pill:hover:not(:disabled) {
    background: var(--bg-input);
    border-color: var(--border-input);
  }

  .prod .pill {
    border-color: color-mix(in srgb, var(--env) 60%, transparent);
  }

  .tag {
    flex: none;
    height: 18px;
    line-height: 18px;
    padding: 0 7px;
    border-radius: 5px;
    font-size: 10px;
    font-weight: 700;
    color: var(--env);
    background: color-mix(in srgb, var(--env) 16%, transparent);
  }

  .prod .tag {
    color: #fff;
    background: var(--env);
  }

  .ver,
  .db {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ver {
    flex: 0 2 auto;
  }

  .colon {
    color: var(--fg-faint);
  }

  .name {
    color: var(--fg);
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .db {
    color: var(--fg);
  }

  .lock {
    display: inline-flex;
    color: var(--warning);
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
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-fg);
  }

  .brand b {
    font-size: 13px;
    font-weight: 760;
    letter-spacing: -0.04em;
  }

  .brand b span {
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .brand b span {
    color: color-mix(in srgb, var(--accent) 65%, #2d3a12);
  }
</style>
