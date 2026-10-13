<!-- Window chrome colored by the connection's environment. -->
<script lang="ts">
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { ENGINE_LABEL, envColor } from '../util/env'
  import { modLabel } from '../util/format'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let menu = $state<{ x: number; y: number } | null>(null)
  let isProd = $derived(session.config.env === 'production')

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

<header class="top" style:background={envColor(session.config)}>
  <button class="chip" title="Back to connections ({modLabel}K)" onclick={() => (sessions.view = 'connections')}>
    ‹ Connections
  </button>

  <div class="title">
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
    <span class="ver" title="Server version">{ENGINE_LABEL[session.config.engine]} {session.serverVersion}</span>
    {#if isProd}<span class="prod">PRODUCTION</span>{/if}
    {#if session.config.readOnly}<span class="ro">READ-ONLY</span>{/if}
  </div>

  <span class="spacer"></span>
  <button class="chip" aria-label="Menu" title="Menu" onclick={openMenu}>⋯</button>
</header>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} {items} onClose={() => (menu = null)} />
{/if}

<style>
  .top {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 34px;
    padding: 0 10px;
    color: #fff;
    flex: none;
  }

  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .name {
    white-space: nowrap;
  }

  .ver {
    opacity: 0.85;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .prod,
  .ro {
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    padding: 1px 7px;
    border-radius: 4px;
    background: #fff;
    color: var(--env-production);
    white-space: nowrap;
  }

  .ro {
    color: #333;
    background: rgb(255 255 255 / 0.75);
  }

  .spacer {
    flex: 1;
  }

  .chip {
    background: rgb(255 255 255 / 0.18);
    border-color: transparent;
    color: #fff;
    padding: 1px 10px;
    white-space: nowrap;
  }

  .chip:hover:not(:disabled) {
    background: rgb(255 255 255 / 0.3);
  }

  .switch {
    background: rgb(255 255 255 / 0.18);
    color: #fff;
    border-color: transparent;
    font-weight: 700;
  }

  .switch option {
    color: #000;
  }
</style>
