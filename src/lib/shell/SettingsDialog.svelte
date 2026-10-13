<!-- Settings: appearance, shortcuts, updates. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import { theme, type ThemeMode } from '../stores/theme.svelte'
  import { history } from '../stores/history.svelte'
  import { ui } from '../stores/ui.svelte'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  const MODES: { mode: ThemeMode; label: string }[] = [
    { mode: 'system', label: 'System' },
    { mode: 'light', label: 'Light' },
    { mode: 'dark', label: 'Dark' },
  ]
</script>

<Modal title="Settings" {onClose} width="460px">
  <div class="rows">
    <div class="row">
      <div>
        <b>Appearance</b>
        <p class="muted">Follow the system, or pick a theme.</p>
      </div>
      <div class="seg">
        {#each MODES as m (m.mode)}
          <button class:on={theme.mode === m.mode} onclick={() => theme.set(m.mode)}>{m.label}</button>
        {/each}
      </div>
    </div>
    <div class="row">
      <div>
        <b>Query history</b>
        <p class="muted">{history.entries.length} queries kept on this computer.</p>
      </div>
      <button class="outline" disabled={history.entries.length === 0} onclick={() => history.clear()}>Clear history</button>
    </div>
    <div class="row">
      <div>
        <b>Keyboard shortcuts</b>
        <p class="muted">Every shortcut in one place.</p>
      </div>
      <button class="outline" onclick={() => { onClose(); ui.shortcutsHelp = true }}>Show</button>
    </div>
    <div class="row">
      <div>
        <b>Updates</b>
        <p class="muted">Version and update check.</p>
      </div>
      <button class="outline" onclick={() => { onClose(); ui.about = true }}>About…</button>
    </div>
  </div>
  {#snippet footer()}
    <button class="primary" onclick={onClose}>Done</button>
  {/snippet}
</Modal>

<style>
  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
  }

  .row:last-child {
    border-bottom: none;
  }

  p {
    margin: 3px 0 0;
    font-size: 11px;
  }
</style>
