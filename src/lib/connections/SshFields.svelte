<!-- Host/port/user/auth fields for one SSH hop (the main tunnel or the jump host). -->
<script lang="ts">
  import type { SshConfig } from '../types'
  import FilePick from './FilePick.svelte'

  interface Props {
    ssh: SshConfig
    password: string
    passphrase: string
  }

  let { ssh = $bindable(), password = $bindable(), passphrase = $bindable() }: Props = $props()

  let authKind = $derived(ssh.auth.kind)

  function setAuth(kind: string) {
    if (kind === 'key') ssh.auth = { kind: 'key', path: '' }
    else if (kind === 'agent') ssh.auth = { kind: 'agent' }
    else ssh.auth = { kind: 'password' }
  }
</script>

<div class="grid">
  <label class="wide">Host <input type="text" bind:value={ssh.host} placeholder="bastion.example.com" /></label>
  <label>Port <input type="number" min="1" max="65535" bind:value={ssh.port} /></label>
  <label>User <input type="text" bind:value={ssh.user} /></label>
  <label class="wide">
    Auth
    <select value={authKind} onchange={(e) => setAuth(e.currentTarget.value)}>
      <option value="password">Password</option>
      <option value="key">Key file</option>
      <option value="agent">SSH agent</option>
    </select>
  </label>
  {#if ssh.auth.kind === 'password'}
    <label class="wide">
      Password
      <input type="password" bind:value={password} placeholder="Leave blank to keep the saved one" />
    </label>
  {:else if ssh.auth.kind === 'key'}
    <div class="wide field">
      Private key
      <FilePick bind:value={ssh.auth.path} placeholder="~/.ssh/id_ed25519" title="Choose SSH private key" />
    </div>
    <label class="wide">
      Key passphrase
      <input type="password" bind:value={passphrase} placeholder="Leave blank if none / keep the saved one" />
    </label>
  {/if}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 10px;
  }

  label,
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--fg-muted);
    font-size: 12px;
  }

  .wide {
    grid-column: 1 / -1;
  }
</style>
