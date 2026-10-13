<script lang="ts">
  import { onMount } from 'svelte'
  import Welcome from './lib/connections/Welcome.svelte'
  import Workspace from './lib/shell/Workspace.svelte'
  import ExportDialog from './lib/dialogs/ExportDialog.svelte'
  import ImportDialog from './lib/dialogs/ImportDialog.svelte'
  import AboutDialog from './lib/shell/AboutDialog.svelte'
  import QuickOpen from './lib/shell/QuickOpen.svelte'
  import ShortcutsHelp from './lib/shell/ShortcutsHelp.svelte'
  import Toasts from './lib/shell/Toasts.svelte'
  import { handleShortcut } from './lib/shell/shortcuts'
  import { connections } from './lib/stores/connections.svelte'
  import { sessions } from './lib/stores/sessions.svelte'
  import { theme } from './lib/stores/theme.svelte'
  import { ui } from './lib/stores/ui.svelte'

  theme.init()

  onMount(() => {
    void connections.load()
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

<div class="app">
  {#if sessions.view === 'connections' || !sessions.active}
    <Welcome />
  {/if}
  <Workspace visible={sessions.view === 'workspace' && !!sessions.active} />
</div>

{#if ui.exportRequest}
  {@const r = ui.exportRequest}
  <ExportDialog session={r.session} source={r.source} name={r.name} onClose={() => (ui.exportRequest = null)} />
{/if}
{#if ui.importRequest}
  {@const r = ui.importRequest}
  <ImportDialog session={r.session} table={r.table} onClose={() => (ui.importRequest = null)} />
{/if}
{#if ui.quickOpen}
  <QuickOpen onClose={() => (ui.quickOpen = false)} />
{/if}
{#if ui.shortcutsHelp}
  <ShortcutsHelp onClose={() => (ui.shortcutsHelp = false)} />
{/if}
{#if ui.about}
  <AboutDialog onClose={() => (ui.about = false)} />
{/if}
<Toasts />

<style>
  .app {
    height: 100%;
  }
</style>
