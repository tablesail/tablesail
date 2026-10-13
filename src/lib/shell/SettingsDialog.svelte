<!-- Settings: appearance, AI assistant, shortcuts, updates. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import { theme, type ThemeMode } from '../stores/theme.svelte'
  import { history } from '../stores/history.svelte'
  import { ui } from '../stores/ui.svelte'
  import { assistant, MODELS } from '../stores/assistant.svelte'
  import { errorMessage } from '../api'
  import { onMount } from 'svelte'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  let apiKey = $state('')
  let keyBusy = $state(false)

  onMount(() => void assistant.refreshKey())

  async function saveKey() {
    const key = apiKey.trim()
    if (!key) return
    keyBusy = true
    try {
      await assistant.setKey(key)
      apiKey = ''
      ui.toast('API key saved in Keychain', 'success')
    } catch (e) {
      ui.toast(errorMessage(e), 'error')
    } finally {
      keyBusy = false
    }
  }

  async function removeKey() {
    keyBusy = true
    const typed = apiKey
    try {
      await assistant.setKey('')
      if (apiKey === typed) apiKey = ''
    } catch (e) {
      ui.toast(errorMessage(e), 'error')
    } finally {
      keyBusy = false
    }
  }

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
    <div class="row stack">
      <div>
        <b>AI assistant</b>
        <p class="muted">Your Anthropic API key is kept in the system Keychain and only sent to Anthropic.</p>
      </div>
      <form class="keyline" onsubmit={(e) => { e.preventDefault(); void saveKey() }}>
        <input
          type="password"
          autocomplete="off"
          spellcheck="false"
          aria-label="Anthropic API key"
          placeholder={assistant.hasKey ? 'Key saved in Keychain' : 'sk-ant-…'}
          bind:value={apiKey}
        />
        <button type="submit" class="outline" disabled={keyBusy || !apiKey.trim()}>Save</button>
        <button type="button" class="outline" disabled={keyBusy || !assistant.hasKey} onclick={removeKey}>Remove</button>
      </form>
      <label class="modelline">
        <span class="muted">Model</span>
        <select value={assistant.model} onchange={(e) => assistant.setModel(e.currentTarget.value)}>
          {#each MODELS as m (m.id)}
            <option value={m.id}>{m.label}</option>
          {/each}
          {#if !MODELS.some((m) => m.id === assistant.model)}
            <option value={assistant.model}>{assistant.model}</option>
          {/if}
        </select>
      </label>
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

  .row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }

  .keyline {
    display: flex;
    gap: 6px;
  }

  .keyline input {
    flex: 1;
  }

  .modelline {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
  }

  p {
    margin: 3px 0 0;
    font-size: 11px;
  }
</style>
