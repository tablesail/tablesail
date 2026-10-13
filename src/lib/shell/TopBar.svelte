<!--
  Unified window toolbar, TablePlus style. On macOS it sits inside the title bar
  (titleBarStyle: Overlay), so it leaves room for the traffic lights and acts as
  the window's drag region. The centred pill names the connection and database.
-->
<script lang="ts">
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import EngineBadge from '../ui/EngineBadge.svelte'
  import Icon from '../ui/Icon.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { ENGINE_LABEL, envColor } from '../util/env'
  import { isMac, modLabel } from '../util/format'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)
  let isProd = $derived(session.config.env === 'production')
  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  // Postgres/Mongo/SQLite versions already start with the product name; MySQL's ("8.4.6-MySQL") don't.
  let version = $derived(
    session.serverVersion.toLowerCase().startsWith(ENGINE_LABEL[session.config.engine].toLowerCase())
      ? session.serverVersion
      : `${ENGINE_LABEL[session.config.engine]} ${session.serverVersion.replace(/-(MySQL|MariaDB).*$/i, '')}`,
  )

  const envLabel = { local: 'local', development: 'dev', staging: 'staging', production: 'production' }

  function openAt(e: MouseEvent, items: MenuItem[], alignRight = false) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    menu = { x: alignRight ? r.right - 200 : r.left, y: r.bottom + 4, items }
  }

  let appItems = $derived<MenuItem[]>([
    { label: 'Keyboard shortcuts', hint: `${modLabel}/`, onclick: () => (ui.shortcutsHelp = true) },
    {
      label: `Theme: ${theme.mode === 'system' ? 'System' : theme.mode === 'dark' ? 'Dark' : 'Light'}`,
      hint: `${modLabel}⇧L`,
      onclick: () => theme.toggle(),
    },
    { label: 'Use system theme', disabled: theme.mode === 'system', onclick: () => theme.set('system') },
    { separator: true },
    { label: 'About / Check for updates…', onclick: () => (ui.about = true) },
  ])

  let pillItems = $derived<MenuItem[]>([
    ...sessions.list.map((s) => ({
      label: s.sessionId === session.sessionId ? `✓ ${s.config.name}` : `   ${s.config.name}`,
      onclick: () => sessions.activate(s.sessionId),
    })),
    { separator: true },
    { label: 'Open another connection…', hint: `${modLabel}K`, onclick: () => (sessions.view = 'connections') },
    { label: 'Disconnect', danger: true, onclick: () => sessions.close(session.sessionId) },
  ])
</script>

<header class="toolbar" class:mac={isMac} class:prod={isProd} style:--env={envColor(session.config)} data-tauri-drag-region>
  <div class="group" data-tauri-drag-region>
    <button class="tb" title="Connections ({modLabel}K)" aria-label="Connections" onclick={() => (sessions.view = 'connections')}>
      <Icon name="server" size={15} />
    </button>
    <button
      class="tb"
      title={entry.loading ? 'Refreshing…' : 'Refresh schema'}
      aria-label="Refresh schema"
      class:spin={entry.loading}
      onclick={() => sessions.refreshSchema(session.sessionId, database)}
    >
      <Icon name="refresh" size={15} />
    </button>
  </div>

  <button class="pill" title="{version} · switch connection" onclick={(e) => openAt(e, pillItems)}>
    <span class="tag">{envLabel[session.config.env]}</span>
    <EngineBadge engine={session.config.engine} size={16} />
    <span class="ver">{version}</span>
    <span class="colon">:</span>
    <b class="name">{session.config.name}</b>
    {#if database}<span class="colon">:</span><span class="db">{database}</span>{/if}
    {#if session.config.readOnly}<span class="lock" title="Read-only"><Icon name="lock" size={11} /></span>{/if}
  </button>

  <div class="group right" data-tauri-drag-region>
    <button
      class="tb"
      title="New query ({modLabel}T)"
      aria-label="New query"
      onclick={() => tabs.newQuery(session.sessionId, database)}
    >
      <Icon name="query" size={15} />
      <span>SQL</span>
    </button>
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
    height: 46px;
    padding: 0 10px;
    flex: none;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border-strong);
  }

  /* Room for the macOS traffic lights (titleBarStyle: Overlay). */
  .toolbar.mac {
    padding-left: 84px;
  }

  .toolbar.prod {
    background: color-mix(in srgb, var(--env) 12%, var(--bg-chrome));
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
    gap: 4px;
    height: 26px;
    min-height: 26px;
    padding: 0 7px;
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    color: var(--fg-muted);
    font-size: 12px;
  }

  .tb:hover:not(:disabled) {
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
    gap: 6px;
    padding: 0 12px 0 5px;
    border-radius: 7px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-sm);
    font-size: 12px;
    color: var(--fg-muted);
    overflow: hidden;
  }

  .pill:hover:not(:disabled) {
    background: var(--bg);
    border-color: var(--fg-faint);
  }

  .prod .pill {
    border-color: var(--env);
  }

  .tag {
    flex: none;
    height: 18px;
    line-height: 18px;
    padding: 0 6px;
    border-radius: 4px;
    font-size: 10.5px;
    font-weight: 600;
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
    font-weight: 600;
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
</style>
