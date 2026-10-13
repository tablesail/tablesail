<script lang="ts">
  import { onMount } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import { api, errorMessage } from '../api'
  import type { CoreInfo } from '../types'
  import { formatBytes } from '../util/format'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  type Phase =
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'current' }
    | { kind: 'available'; version: string; notes: string }
    | { kind: 'downloading'; done: number; total: number | null }
    | { kind: 'installed' }
    | { kind: 'unavailable'; message: string }
    | { kind: 'error'; message: string }

  let info = $state<CoreInfo | null>(null)
  let phase = $state<Phase>({ kind: 'idle' })
  // The Update handle from plugin-updater; typed loosely to keep the plugin out of the main bundle.
  let update: { version: string; downloadAndInstall: (cb: (e: any) => void) => Promise<void> } | null = null

  onMount(async () => {
    try {
      info = await api.appInfo()
    } catch {
      // version stays unknown
    }
  })

  function isNotConfigured(msg: string): boolean {
    return /pubkey|endpoint|not configured|updater.*(config|plugin)|plugin.*updater|not allowed|not found|unsupported/i.test(msg)
  }

  async function check() {
    phase = { kind: 'checking' }
    try {
      const { check } = await import('@tauri-apps/plugin-updater')
      const result = await check()
      if (!result) {
        phase = { kind: 'current' }
        return
      }
      update = result
      phase = { kind: 'available', version: result.version, notes: result.body ?? '' }
    } catch (e) {
      const message = errorMessage(e)
      phase = isNotConfigured(message)
        ? { kind: 'unavailable', message: 'Updates are not configured for this build.' }
        : { kind: 'error', message: `Could not check for updates: ${message}` }
    }
  }

  async function install() {
    if (!update) return
    let done = 0
    let total: number | null = null
    phase = { kind: 'downloading', done, total }
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === 'Started') total = event.data.contentLength ?? null
        else if (event.event === 'Progress') done += event.data.chunkLength
        phase = { kind: 'downloading', done, total }
      })
      phase = { kind: 'installed' }
    } catch (e) {
      phase = { kind: 'error', message: `Update failed: ${errorMessage(e)}` }
    }
  }

  async function restart() {
    try {
      const { relaunch } = await import('@tauri-apps/plugin-process')
      await relaunch()
    } catch (e) {
      phase = { kind: 'error', message: `Could not restart: ${errorMessage(e)}` }
    }
  }
</script>

<Modal title="About TableSoft" {onClose} width="440px">
  <div class="about">
    <div class="ver"><b>TableSoft</b> {info ? `v${info.version}` : ''}</div>
    <div class="muted">A fast, free, open-source database client.</div>
    {#if info}<div class="muted">Engines: {info.engines.join(', ')}</div>{/if}

    <div class="update">
      {#if phase.kind === 'idle'}
        <button onclick={check}>Check for updates</button>
      {:else if phase.kind === 'checking'}
        <span class="muted">Checking…</span>
      {:else if phase.kind === 'current'}
        <span class="ok">You are on the latest version.</span>
        <button onclick={check}>Check again</button>
      {:else if phase.kind === 'available'}
        <div>Version <b>{phase.version}</b> is available.</div>
        {#if phase.notes}<pre>{phase.notes}</pre>{/if}
        <button class="primary" onclick={install}>Download and install</button>
      {:else if phase.kind === 'downloading'}
        <div>
          Downloading… {formatBytes(phase.done)}{phase.total ? ` of ${formatBytes(phase.total)}` : ''}
        </div>
        <progress max={phase.total ?? undefined} value={phase.total ? phase.done : undefined}></progress>
      {:else if phase.kind === 'installed'}
        <div class="ok">Update installed.</div>
        <button class="primary" onclick={restart}>Restart now</button>
      {:else if phase.kind === 'unavailable'}
        <span class="muted">{phase.message}</span>
      {:else}
        <span class="error-text" role="alert">{phase.message}</span>
        <button onclick={check}>Retry</button>
      {/if}
    </div>
  </div>

  {#snippet footer()}
    <button onclick={onClose} disabled={phase.kind === 'downloading'}>Close</button>
  {/snippet}
</Modal>

<style>
  .about {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ver {
    font-size: 16px;
  }

  .update {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }

  .ok {
    color: var(--success);
  }

  pre {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    font-family: inherit;
    width: 100%;
    padding: 6px 8px;
    background: var(--bg-sidebar);
    border-radius: 5px;
  }

  progress {
    width: 100%;
  }
</style>
