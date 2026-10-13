<!--
  Window top bar. On macOS it sits inside the title bar (titleBarStyle: Overlay),
  so it leaves room for the traffic lights and is the window's drag region.
-->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import { inspector } from '../stores/inspector.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { envColor } from '../util/env'
  import { isMac, modLabel } from '../util/format'

  interface Props {
    session: SessionInfo | null
  }

  let { session }: Props = $props()

  let dark = $derived(theme.isDark)
</script>

<header class="topbar" class:mac={isMac} data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <span class="mark"><Icon name="pulse" size={18} strokeWidth={1.5} /></span>
    <b>table<span>soft</span></b>
  </div>

  <span class="crumb-sep" aria-hidden="true"></span>
  <div class="crumbs" data-tauri-drag-region>
    <Icon name="layers" size={13} />
    <button class="link" onclick={() => (sessions.view = 'connections')}>Connections</button>
    {#if session}
      <Icon name="chevron-right" size={11} />
      <span class="conn" style:--env={envColor(session.config)}>
        <i class="env-dot"></i>{session.config.name}
      </span>
    {/if}
  </div>

  <div class="center" data-tauri-drag-region>
    <button class="command" onclick={() => (ui.palette = true)}>
      <Icon name="search" size={13} />
      <span>Search tables, queries, commands…</span>
      <kbd>{modLabel} K</kbd>
    </button>
  </div>

  <div class="actions">
    <button class="ib" title="Toggle light / dark ({modLabel}⇧L)" aria-label="Toggle appearance" onclick={() => theme.set(dark ? 'light' : 'dark')}>
      <Icon name={dark ? 'sun' : 'moon'} size={16} />
    </button>
    {#if session}
      <button class="ib" class:on={inspector.open} title="Toggle row inspector" aria-label="Toggle row inspector" onclick={() => inspector.toggle()}>
        <Icon name="panel" size={16} />
      </button>
    {/if}
    <button class="ib" title="Keyboard shortcuts ({modLabel}/)" aria-label="Keyboard shortcuts" onclick={() => (ui.shortcutsHelp = true)}>
      <Icon name="command" size={16} />
    </button>
  </div>
</header>

<style>
  .topbar {
    height: 54px;
    min-height: 54px;
    display: flex;
    align-items: center;
    gap: 17px;
    padding: 0 17px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
    flex: none;
  }

  /* Room for the macOS traffic lights (titleBarStyle: Overlay). */
  .topbar.mac {
    padding-left: 84px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    white-space: nowrap;
  }

  .mark {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--accent-fg);
    box-shadow: 0 0 22px color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .brand b {
    font-size: 14px;
    font-weight: 760;
    letter-spacing: -0.045em;
  }

  .brand b span {
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .brand b span {
    color: color-mix(in srgb, var(--accent) 70%, #2d3a12);
  }

  .crumb-sep {
    width: 1px;
    height: 18px;
    background: var(--border-strong);
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-muted);
    font-size: 11px;
    white-space: nowrap;
    min-width: 0;
  }

  .link {
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    color: inherit;
    font-size: inherit;
  }

  .link:hover:not(:disabled) {
    background: none;
    color: var(--fg);
  }

  .conn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .env-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--env);
    box-shadow: 0 0 7px color-mix(in srgb, var(--env) 50%, transparent);
  }

  .center {
    flex: 1;
    display: flex;
    justify-content: center;
    min-width: 0;
  }

  .command {
    justify-content: flex-start;
    gap: 8px;
    width: 350px;
    max-width: 100%;
    height: 31px;
    padding: 0 10px;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    box-shadow: none;
    color: var(--fg-muted);
    font-size: 11px;
  }

  .command span {
    flex: 1;
    text-align: left;
  }

  .command:hover:not(:disabled) {
    background: var(--bg-input);
    border-color: var(--border-input);
    color: var(--fg);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .ib {
    width: 30px;
    min-width: 30px;
    height: 30px;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    border-radius: 7px;
    color: var(--fg-muted);
  }

  .ib:hover:not(:disabled),
  .ib.on {
    color: var(--fg);
    background: var(--bg-hover);
  }

  @media (max-width: 1120px) {
    .crumbs,
    .crumb-sep {
      display: none;
    }
  }
</style>
