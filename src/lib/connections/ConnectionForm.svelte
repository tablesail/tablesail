<!-- Create / edit a saved connection. Secrets are sent to the keychain and never stored in the config. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import FilePick from './FilePick.svelte'
  import SshFields from './SshFields.svelte'
  import { api, errorMessage } from '../api'
  import { connections } from '../stores/connections.svelte'
  import type { ConnectionConfig, Engine, Secrets, SshConfig, SslMode, TestResult } from '../types'
  import { DEFAULT_PORT, ENGINE_LABEL, ENV_TAGS, defaultUser, newConfig } from '../util/env'
  import { basename } from '../util/format'

  interface Props {
    initial: ConnectionConfig | null
    onClose: () => void
    onSaved?: (config: ConnectionConfig) => void
  }

  let { initial, onClose, onSaved }: Props = $props()

  const engines: Engine[] = ['postgres', 'mysql', 'mariadb', 'sqlite', 'mongodb']
  const sslModes: { value: SslMode; label: string }[] = [
    { value: 'disable', label: 'Disable' },
    { value: 'prefer', label: 'Prefer' },
    { value: 'require', label: 'Require' },
    { value: 'verifyCa', label: 'Verify CA' },
    { value: 'verifyFull', label: 'Verify full (CA + host name)' },
  ]

  function blankSsh(): SshConfig {
    return { host: '', port: 22, user: '', auth: { kind: 'agent' }, jump: null }
  }

  // svelte-ignore state_referenced_locally
  const start: ConnectionConfig = initial ? structuredClone($state.snapshot(initial)) : newConfig()
  let cfg = $state<ConnectionConfig>(start)
  let dbPassword = $state('')
  let sshPassword = $state('')
  let sshPassphrase = $state('')
  let jumpPassword = $state('')
  let jumpPassphrase = $state('')
  let useSsh = $state(!!start.ssh)
  let useJump = $state(!!start.ssh?.jump)
  let customColor = $state(!!start.color)
  let colorValue = $state(start.color || '#2f7de1')

  let testing = $state(false)
  let testResult = $state<TestResult | null>(null)
  let testError = $state<string | null>(null)
  let saving = $state(false)
  let saveError = $state<string | null>(null)

  let isSqlite = $derived(cfg.engine === 'sqlite')
  let isMongo = $derived(cfg.engine === 'mongodb')
  let sslOn = $derived(cfg.ssl.mode !== 'disable')

  function changeEngine(next: Engine) {
    const prev = cfg.engine
    if (cfg.port === DEFAULT_PORT[prev] || !cfg.port) cfg.port = DEFAULT_PORT[next]
    if (cfg.user === defaultUser(prev)) cfg.user = defaultUser(next)
    if (next !== 'sqlite' && !cfg.host) cfg.host = '127.0.0.1'
    cfg.engine = next
  }

  function toggleSsh(on: boolean) {
    useSsh = on
    if (on && !cfg.ssh) cfg.ssh = blankSsh()
  }

  function toggleJump(on: boolean) {
    useJump = on
    if (on && cfg.ssh && !cfg.ssh.jump) cfg.ssh.jump = blankSsh()
  }

  /** Builds the config to send: drops disabled sections, keeps only what applies to the engine. */
  function buildConfig(): ConnectionConfig {
    const out: ConnectionConfig = structuredClone($state.snapshot(cfg))
    out.name = out.name.trim()
    if (!out.name) {
      out.name = isSqlite && out.filePath ? basename(out.filePath) : out.database || out.host || 'Untitled'
    }
    out.color = customColor ? colorValue : null
    if (isSqlite) {
      out.host = ''
      out.port = 0
      out.user = ''
      out.ssh = null
      out.ssl = { mode: 'disable' }
    } else {
      out.filePath = null
      if (!isMongo) out.url = null
      else out.url = out.url?.trim() || null
      if (!useSsh || !out.ssh) out.ssh = null
      else if (!useJump) out.ssh.jump = null
    }
    return out
  }

  /** Only fields the user typed; undefined leaves the stored secret untouched. */
  function buildSecrets(): Secrets | null {
    const s: Secrets = {}
    if (dbPassword && !isSqlite) s.dbPassword = dbPassword
    if (useSsh && !isSqlite) {
      if (sshPassword) s.sshPassword = sshPassword
      if (sshPassphrase) s.sshPassphrase = sshPassphrase
      if (useJump) {
        if (jumpPassword) s.jumpPassword = jumpPassword
        if (jumpPassphrase) s.jumpPassphrase = jumpPassphrase
      }
    }
    return Object.keys(s).length ? s : null
  }

  function validate(): string | null {
    if (isSqlite) return cfg.filePath ? null : 'Choose a SQLite database file.'
    if (isMongo && cfg.url?.trim()) return null
    if (!cfg.host.trim()) return 'Host is required.'
    if (useSsh && cfg.ssh && !cfg.ssh.host.trim()) return 'SSH host is required.'
    if (useSsh && useJump && cfg.ssh?.jump && !cfg.ssh.jump.host.trim()) return 'Jump host is required.'
    return null
  }

  async function test() {
    const problem = validate()
    testResult = null
    testError = problem
    if (problem) return
    testing = true
    try {
      testResult = await api.connectionTest(buildConfig(), buildSecrets())
    } catch (e) {
      testError = errorMessage(e)
    } finally {
      testing = false
    }
  }

  async function save() {
    const problem = validate()
    if (problem) {
      saveError = problem
      return
    }
    saving = true
    saveError = null
    try {
      const saved = await connections.save(buildConfig(), buildSecrets())
      onSaved?.(saved)
      onClose()
    } catch (e) {
      saveError = errorMessage(e)
    } finally {
      saving = false
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      void save()
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<Modal title={initial ? 'Edit connection' : 'New connection'} {onClose} width="560px" dismissable={!saving}>
  <div class="form">
    <div class="row two">
      <label>
        Name
        <input type="text" bind:value={cfg.name} placeholder="My database" />
      </label>
      <label>
        Engine
        <select value={cfg.engine} onchange={(e) => changeEngine(e.currentTarget.value as Engine)}>
          {#each engines as eng (eng)}
            <option value={eng}>{ENGINE_LABEL[eng]}</option>
          {/each}
        </select>
      </label>
    </div>

    {#if isSqlite}
      <div class="field">
        Database file
        <FilePick
          bind:value={cfg.filePath}
          placeholder="/path/to/database.db"
          title="Open SQLite database"
          filters={[{ name: 'SQLite', extensions: ['db', 'sqlite', 'sqlite3', 'db3', 's3db'] }]}
        />
      </div>
    {:else}
      {#if isMongo}
        <label>
          Connection URL <span class="opt">optional, overrides host and port</span>
          <input type="text" bind:value={cfg.url} placeholder="mongodb://host:27017/?replicaSet=rs0" />
        </label>
      {/if}
      <div class="row hostport">
        <label>
          Host
          <input type="text" bind:value={cfg.host} placeholder="127.0.0.1" />
        </label>
        <label>
          Port
          <input type="number" min="1" max="65535" bind:value={cfg.port} />
        </label>
      </div>
      <div class="row two">
        <label>
          User
          <input type="text" bind:value={cfg.user} autocomplete="off" />
        </label>
        <label>
          Password
          <input
            type="password"
            bind:value={dbPassword}
            autocomplete="new-password"
            placeholder={initial ? 'Leave blank to keep the saved one' : ''}
          />
        </label>
      </div>
      <label>
        Database <span class="opt">optional</span>
        <input type="text" bind:value={cfg.database} />
      </label>
    {/if}

    <div class="row two">
      <label>
        Environment
        <select bind:value={cfg.env}>
          {#each ENV_TAGS as tag (tag)}
            <option value={tag}>{tag[0].toUpperCase() + tag.slice(1)}</option>
          {/each}
        </select>
      </label>
      <div class="field">
        Color
        <div class="color">
          <label class="inline">
            <input type="checkbox" bind:checked={customColor} /> Custom
          </label>
          <input type="color" bind:value={colorValue} disabled={!customColor} />
        </div>
      </div>
    </div>

    <label class="inline">
      <input type="checkbox" bind:checked={cfg.readOnly} />
      Read-only connection (blocks every statement that is not a plain read)
    </label>

    {#if !isSqlite}
      <details open={cfg.ssl.mode !== 'prefer' && cfg.ssl.mode !== 'disable'}>
        <summary>SSL</summary>
        <div class="section">
          <label>
            Mode
            <select bind:value={cfg.ssl.mode}>
              {#each sslModes as m (m.value)}
                <option value={m.value}>{m.label}</option>
              {/each}
            </select>
          </label>
          {#if sslOn}
            <div class="field">
              CA certificate
              <FilePick bind:value={cfg.ssl.caCert} title="Choose CA certificate" />
            </div>
            <div class="field">
              Client certificate
              <FilePick bind:value={cfg.ssl.clientCert} title="Choose client certificate" />
            </div>
            <div class="field">
              Client key
              <FilePick bind:value={cfg.ssl.clientKey} title="Choose client key" />
            </div>
          {/if}
        </div>
      </details>

      <details open={useSsh}>
        <summary>SSH tunnel</summary>
        <div class="section">
          <label class="inline">
            <input type="checkbox" checked={useSsh} onchange={(e) => toggleSsh(e.currentTarget.checked)} />
            Connect through an SSH tunnel
          </label>
          {#if useSsh && cfg.ssh}
            <SshFields bind:ssh={cfg.ssh} bind:password={sshPassword} bind:passphrase={sshPassphrase} />
            <label class="inline">
              <input type="checkbox" checked={useJump} onchange={(e) => toggleJump(e.currentTarget.checked)} />
              Use a jump host
            </label>
            {#if useJump && cfg.ssh.jump}
              <div class="jump">
                <SshFields bind:ssh={cfg.ssh.jump} bind:password={jumpPassword} bind:passphrase={jumpPassphrase} />
              </div>
            {/if}
          {/if}
        </div>
      </details>
    {/if}

    {#if testResult}
      <div class="notice ok">
        Connected. Server {testResult.serverVersion} · {testResult.elapsedMs} ms
      </div>
    {:else if testError}
      <div class="notice bad">{testError}</div>
    {/if}
    {#if saveError}
      <div class="notice bad">{saveError}</div>
    {/if}
  </div>

  {#snippet footer()}
    <button onclick={test} disabled={testing || saving}>{testing ? 'Testing…' : 'Test'}</button>
    <span class="spacer"></span>
    <button onclick={onClose} disabled={saving}>Cancel</button>
    <button class="primary" onclick={save} disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  label,
  .field {
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

  .row {
    display: grid;
    gap: 10px;
  }

  .row.two {
    grid-template-columns: 1fr 1fr;
  }

  .row.hostport {
    grid-template-columns: 1fr 100px;
  }

  .opt {
    opacity: 0.7;
  }

  .color {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 27px;
  }

  input[type='color'] {
    width: 40px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border);
    background: none;
  }

  details {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 10px;
  }

  summary {
    cursor: pointer;
    font-weight: 600;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 0 4px;
  }

  .jump {
    border-left: 2px solid var(--border);
    padding-left: 10px;
  }

  .notice {
    padding: 6px 10px;
    border-radius: 6px;
    user-select: text;
    -webkit-user-select: text;
    word-break: break-word;
  }

  .notice.ok {
    background: color-mix(in srgb, var(--success) 15%, transparent);
    color: var(--success);
  }

  .notice.bad {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
  }

  .spacer {
    flex: 1;
  }
</style>
