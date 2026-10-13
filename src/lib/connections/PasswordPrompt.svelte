<!-- Asked when opening a connection failed on authentication and no password was stored. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import type { ConnectionConfig, Secrets } from '../types'

  interface Props {
    config: ConnectionConfig
    error: string
    busy: boolean
    onSubmit: (secrets: Secrets, remember: boolean) => void
    onClose: () => void
  }

  let { config, error, busy, onSubmit, onClose }: Props = $props()

  let dbPassword = $state('')
  let sshSecret = $state('')
  let remember = $state(false)

  let sshAuth = $derived(config.ssh?.auth.kind)
  let needsDb = $derived(config.engine !== 'sqlite')

  function submit() {
    const s: Secrets = {}
    if (needsDb && dbPassword) s.dbPassword = dbPassword
    if (sshSecret && sshAuth === 'password') s.sshPassword = sshSecret
    if (sshSecret && sshAuth === 'key') s.sshPassphrase = sshSecret
    onSubmit(s, remember)
  }
</script>

<Modal title="Password required" {onClose} width="420px" dismissable={!busy}>
  <form
    class="form"
    onsubmit={(e) => {
      e.preventDefault()
      submit()
    }}
  >
    <div>Enter the credentials for <b>{config.name}</b>.</div>
    {#if error}<div class="error-text">{error}</div>{/if}
    {#if needsDb}
      <label>
        Password for {config.user || 'user'}
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={dbPassword} autofocus />
      </label>
    {/if}
    {#if sshAuth === 'password' || sshAuth === 'key'}
      <label>
        {sshAuth === 'key' ? 'SSH key passphrase' : 'SSH password'}
        <input type="password" bind:value={sshSecret} />
      </label>
    {/if}
    <label class="inline">
      <input type="checkbox" bind:checked={remember} />
      Save in the system keychain
    </label>
    <button type="submit" hidden>Connect</button>
  </form>

  {#snippet footer()}
    <button onclick={onClose} disabled={busy}>Cancel</button>
    <button class="primary" onclick={submit} disabled={busy}>{busy ? 'Connecting…' : 'Connect'}</button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--fg-muted);
    font-size: 12px;
  }

  label.inline {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    color: var(--fg);
    font-size: 13px;
  }
</style>
