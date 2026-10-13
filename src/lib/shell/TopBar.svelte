<!-- Window chrome colored by the connection's environment. -->
<script lang="ts">
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import Icon from '../ui/Icon.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { ENGINE_LABEL, describeConnection, envColor } from '../util/env'
  import { modLabel } from '../util/format'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let menu = $state<{ x: number; y: number } | null>(null)
  let isProd = $derived(session.config.env === 'production')
  let color = $derived(envColor(session.config))
  // Postgres/Mongo/SQLite versions already start with the product name; MySQL's ("8.4.6-MySQL") don't.
  let version = $derived(
    session.serverVersion.toLowerCase().startsWith(ENGINE_LABEL[session.config.engine].toLowerCase())
      ? session.serverVersion
      : `${ENGINE_LABEL[session.config.engine]} ${session.serverVersion.replace(/-(MySQL|MariaDB).*$/i, '')}`,
  )

  function openMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    menu = { x: r.right - 190, y: r.bottom + 4 }
  }

  let items = $derived<MenuItem[]>([
    { label: 'Keyboard shortcuts', hint: `${modLabel}/`, onclick: () => (ui.shortcutsHelp = true) },
    {
      label: `Theme: ${theme.mode === 'system' ? 'System' : theme.mode === 'dark' ? 'Dark' : 'Light'}`,
      hint: `${modLabel}⇧L`,
      onclick: () => theme.toggle(),
    },
    { label: 'Use system theme', disabled: theme.mode === 'system', onclick: () => theme.set('system') },
    { separator: true },
    { label: 'About / Check for updates…', onclick: () => (ui.about = true) },
    { separator: true },
    { label: 'Disconnect', danger: true, onclick: () => sessions.close(session.sessionId) },
  ])
</script>

<header class="top" class:prod={isProd} style:--env={color}>
  <button class="icon back" title="Connections ({modLabel}K)" aria-label="Back to connections" onclick={() => (sessions.view = 'connections')}>
    <Icon name="chevron-left" size={15} />
  </button>

  <div class="title">
    <span class="dot" aria-hidden="true"></span>
    {#if sessions.list.length > 1}
      <select
        class="switch"
        value={session.sessionId}
        onchange={(e) => sessions.activate(e.currentTarget.value)}
        aria-label="Open connections"
      >
        {#each sessions.list as s (s.sessionId)}
          <option value={s.sessionId}>{s.config.name}</option>
        {/each}
      </select>
    {:else}
      <b class="name">{session.config.name}</b>
    {/if}
    <span class="env">{isProd ? 'Production' : session.config.env}</span>
    {#if session.config.readOnly}<span class="badge ro"><Icon name="lock" size={10} /> Read-only</span>{/if}
    <span class="meta" title="Server version and address">{version} · {describeConnection(session.config)}</span>
  </div>

  <span class="spacer"></span>
  <button class="icon" aria-label="Menu" title="Menu" onclick={openMenu}><Icon name="more" size={16} /></button>
</header>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} {items} onClose={() => (menu = null)} />
{/if}

<style>
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 8px;
    flex: none;
    background: var(--bg-chrome);
    border-top: 3px solid var(--env);
    border-bottom: 1px solid var(--border);
  }

  .top.prod {
    background: color-mix(in srgb, var(--env) 14%, var(--bg-chrome));
    border-bottom-color: color-mix(in srgb, var(--env) 35%, var(--border));
  }

  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--env);
    flex: none;
  }

  .name {
    font-weight: 600;
    white-space: nowrap;
  }

  .env {
    flex: none;
    height: 17px;
    line-height: 17px;
    padding: 0 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--env);
    background: color-mix(in srgb, var(--env) 14%, transparent);
  }

  .prod .env {
    color: #fff;
    background: var(--env);
    font-weight: 700;
  }

  .ro {
    gap: 3px;
  }

  .meta {
    color: var(--fg-muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .spacer {
    flex: 1;
  }

  .switch {
    font-weight: 600;
    min-height: 24px;
    padding-top: 2px;
    padding-bottom: 2px;
  }
</style>
