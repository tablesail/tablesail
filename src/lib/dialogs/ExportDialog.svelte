<script lang="ts">
  import { onDestroy } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import JobStatus from './JobStatus.svelte'
  import { JobPoller } from './job.svelte'
  import { api, errorMessage } from '../api'
  import type { ExportFormat, ExportSource, SessionInfo } from '../types'
  import { pickSavePath } from '../util/dialogs'

  interface Props {
    session: SessionInfo
    source: ExportSource
    /** base name for the default file */
    name?: string
    onClose: () => void
  }

  let { session, source, name = 'export', onClose }: Props = $props()

  type Kind = ExportFormat['kind']
  const FORMATS: { kind: Kind; label: string; ext: string; filter: string }[] = [
    { kind: 'csv', label: 'CSV', ext: 'csv', filter: 'CSV' },
    { kind: 'json', label: 'JSON', ext: 'json', filter: 'JSON' },
    { kind: 'jsonLines', label: 'JSON Lines', ext: 'jsonl', filter: 'JSON Lines' },
    { kind: 'sql', label: 'SQL INSERT', ext: 'sql', filter: 'SQL' },
  ]
  const DELIMS = [
    { value: ',', label: 'Comma (,)' },
    { value: ';', label: 'Semicolon (;)' },
    { value: '\t', label: 'Tab' },
    { value: '|', label: 'Pipe (|)' },
  ]

  let kind = $state<Kind>('csv')
  let delimiter = $state(',')
  let header = $state(true)
  let nullAs = $state('')
  let pretty = $state(true)
  // svelte-ignore state_referenced_locally
  let tableName = $state(source.kind === 'table' ? source.request.table.name : 'table_name')
  let rowsPerInsert = $state(100)
  let path = $state('')
  let startError = $state<string | null>(null)
  let starting = $state(false)

  const job = new JobPoller()
  onDestroy(() => job.dispose())

  let fmt = $derived(FORMATS.find((f) => f.kind === kind)!)
  let busy = $derived(starting || job.running)

  function buildFormat(): ExportFormat {
    switch (kind) {
      case 'csv':
        return { kind, delimiter, header, nullAs }
      case 'json':
        return { kind, pretty }
      case 'jsonLines':
        return { kind }
      case 'sql':
        return { kind, table: tableName.trim() || 'table_name', rowsPerInsert: Math.max(1, rowsPerInsert || 1) }
    }
  }

  async function browse() {
    const p = await pickSavePath({
      title: 'Export to file',
      defaultPath: path || `${name}.${fmt.ext}`,
      filters: [{ name: fmt.filter, extensions: [fmt.ext] }],
    })
    if (p) path = p
    return p
  }

  async function start() {
    startError = null
    const target = path || (await browse())
    if (!target) return
    starting = true
    try {
      const id = await api.exportStart(session.sessionId, source, buildFormat(), target)
      starting = false
      await job.track(id)
    } catch (e) {
      startError = errorMessage(e)
    } finally {
      starting = false
    }
  }

  async function close() {
    if (job.running) await job.cancel()
    onClose()
  }
</script>

<Modal title="Export" onClose={close} width="520px" dismissable={!busy}>
  <div class="form">
    <div class="src muted">
      {#if source.kind === 'table'}
        Table <b>{source.request.table.schema ? source.request.table.schema + '.' : ''}{source.request.table.name}</b>
        {#if source.request.filters.length || source.request.rawWhere}(with the current filter){/if}
      {:else}
        Query results
      {/if}
    </div>

    <label>
      Format
      <select bind:value={kind} disabled={busy} onchange={() => (path = '')}>
        {#each FORMATS as f (f.kind)}
          <option value={f.kind}>{f.label}</option>
        {/each}
      </select>
    </label>

    {#if kind === 'csv'}
      <div class="row">
        <label>
          Delimiter
          <select bind:value={delimiter} disabled={busy}>
            {#each DELIMS as d (d.value)}
              <option value={d.value}>{d.label}</option>
            {/each}
          </select>
        </label>
        <label>
          NULL as
          <input type="text" bind:value={nullAs} disabled={busy} placeholder="(empty)" />
        </label>
      </div>
      <label class="inline"><input type="checkbox" bind:checked={header} disabled={busy} /> Header row</label>
    {:else if kind === 'json'}
      <label class="inline"><input type="checkbox" bind:checked={pretty} disabled={busy} /> Pretty-print</label>
    {:else if kind === 'sql'}
      <div class="row">
        <label>
          Table name
          <input type="text" bind:value={tableName} disabled={busy} />
        </label>
        <label>
          Rows per INSERT
          <input type="number" min="1" max="10000" bind:value={rowsPerInsert} disabled={busy} />
        </label>
      </div>
    {/if}

    <label>
      File
      <div class="file">
        <input type="text" bind:value={path} disabled={busy} placeholder={`${name}.${fmt.ext}`} />
        <button onclick={browse} disabled={busy}>Browse…</button>
      </div>
    </label>

    {#if startError}<div class="error-text">{startError}</div>{/if}
    <JobStatus {job} verb="Exported" />
  </div>

  {#snippet footer()}
    {#if job.running}
      <button class="danger" onclick={() => job.cancel()}>Cancel export</button>
    {:else}
      <button onclick={close}>{job.finished ? 'Close' : 'Cancel'}</button>
      <button class="primary" onclick={start} disabled={starting}>{job.finished ? 'Export again' : 'Export'}</button>
    {/if}
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

  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .file {
    display: flex;
    gap: 6px;
  }

  .file input {
    flex: 1;
  }
</style>
