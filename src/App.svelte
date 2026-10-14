<script lang="ts">
  import { onMount } from 'svelte'
  import Welcome from './lib/connections/Welcome.svelte'
  import Workspace from './lib/shell/Workspace.svelte'
  import ExportDialog from './lib/dialogs/ExportDialog.svelte'
  import ImportDialog from './lib/dialogs/ImportDialog.svelte'
  import AboutDialog from './lib/shell/AboutDialog.svelte'
  import CommandPalette from './lib/shell/CommandPalette.svelte'
  import SettingsDialog from './lib/shell/SettingsDialog.svelte'
  import { api } from './lib/api'
  import ShortcutsHelp from './lib/shell/ShortcutsHelp.svelte'
  import Toasts from './lib/shell/Toasts.svelte'
  import { handleShortcut } from './lib/shell/shortcuts'
  import { connections } from './lib/stores/connections.svelte'
  import { sessions } from './lib/stores/sessions.svelte'
  import { theme } from './lib/stores/theme.svelte'
  import { ui } from './lib/stores/ui.svelte'

  theme.init()

  // Browser preview only (src/dev/mockTauri.ts): stand-ins for the macOS window controls.
  const fakeLights = import.meta.env.DEV && (window as unknown as { __TABLESAIL_MOCK__?: boolean }).__TABLESAIL_MOCK__ === true

  let appVersion = $state('')

  onMount(() => {
    void connections.load()
    api.appInfo().then((i) => (appVersion = i.version), () => {})
  })

  function onKeydown(e: KeyboardEvent) {
    if (handleShortcut(e)) {
      e.preventDefault()
      e.stopPropagation()
    }
  }

  // The Tauri webview shows the browser context menu otherwise.
  function onContextMenu(e: MouseEvent) {
    const t = e.target as HTMLElement | null
    if (!t?.closest('input, textarea, .cm-editor, [data-native-menu]')) e.preventDefault()
  }
</script>

<svelte:window onkeydowncapture={onKeydown} oncontextmenu={onContextMenu} />

{#if fakeLights}
  <div class="fake-lights" aria-hidden="true"><i></i><i></i><i></i></div>
{/if}

<div class="app">
  {#if sessions.view === 'connections' || !sessions.active}
    <Welcome />
  {/if}
  <Workspace visible={sessions.view === 'workspace' && !!sessions.active} {appVersion} />
</div>

{#if ui.exportRequest}
  {@const r = ui.exportRequest}
  <ExportDialog session={r.session} source={r.source} name={r.name} onClose={() => (ui.exportRequest = null)} />
{/if}
{#if ui.importRequest}
  {@const r = ui.importRequest}
  <ImportDialog session={r.session} table={r.table} onClose={() => (ui.importRequest = null)} />
{/if}
{#if ui.palette}
  <CommandPalette onClose={() => (ui.palette = false)} />
{/if}

{#if ui.settings}
  <SettingsDialog onClose={() => (ui.settings = false)} />
{/if}

{#if ui.shortcutsHelp}
  <ShortcutsHelp onClose={() => (ui.shortcutsHelp = false)} />
{/if}
{#if ui.about}
  <AboutDialog onClose={() => (ui.about = false)} />
{/if}
<Toasts />

<style>
  .fake-lights {
    position: fixed;
    top: 17px;
    left: 16px;
    display: flex;
    gap: 8px;
    z-index: 1000;
    pointer-events: none;
  }

  .fake-lights i {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #ff5f57;
  }

  .fake-lights i:nth-child(2) {
    background: #febc2e;
  }

  .fake-lights i:nth-child(3) {
    background: #28c840;
  }

  .app {
    height: 100%;
  }
</style>
