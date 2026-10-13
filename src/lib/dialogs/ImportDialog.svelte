<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import JobStatus from './JobStatus.svelte'
  import { JobPoller } from './job.svelte'
  import { api, errorMessage } from '../api'
  import type { ColumnInfo, CsvPreview, SessionInfo, TableRef } from '../types'
  import { pickFile } from '../util/dialogs'
  import { basename } from '../util/format'

  interface Props {
    session: SessionInfo
    table: TableRef
    onClose: () => void
  }

  let { session, table, onClose }: Props = $props()

  const DELIMS = [
    { value: '', label: 'Auto-detect' },
    { value: ',', label: 'Comma (,)' },
    { value: ';', label: 'Semicolon (;)' },
    { value: '\t', label: 'Tab' },
    { value: '|', label: 'Pipe (|)' },
  ]

  let path = $state('')
  let delimiterChoice = $state('')
  let hasHeader = $state(true)
  let nullAs = $state('')
  let preview = $state<CsvPreview | null>(null)
  let targets = $state<ColumnInfo[]>([])
  let mapping = $state<(string | null)[]>([])
  let loadError = $state<string | null>(null)
  let loading = $state(false)
  let starting = $state(false)

  const job = new JobPoller()
  onDestroy(() => job.dispose())

  let busy = $derived(starting || job.running)

  // The preview's first record is the header row; without a header it is data too.
  let columnCount = $derived(preview?.headers.length ?? 0)
  let shownRows = $derived.by(() => {
    if (!preview) return []
    return hasHeader ? preview.rows : [preview.headers, ...preview.rows]
  })
  let csvNames = $derived(
    preview ? preview.headers.map((h, i) => (hasHeader && h ? h : `Column ${i + 1}`)) : [],
  )
  let mappedCount = $derived(mapping.filter((m) => m).length)

  onMount(async () => {
    try {
      const s = await api.tableStructure(session.sessionId, table)
      targets = s.columns.filter((c) => !c.isGenerated)
    } catch (e) {
      loadError = errorMessage(e)
    }
  })

  function autoMap() {
    if (!preview) return
    const byName = new Map(targets.map((c) => [c.name.toLowerCase(), c.name]))
    const used = new Set<string>()
    mapping = preview.headers.map((h, i) => {
      const match = hasHeader
        ? byName.get(h.trim().toLowerCase())
        : targets[i]?.name
      if (match && !used.has(match)) {
        used.add(match)
        return match
      }
      return null
    })
  }

  async function loadPreview() {
    if (!path) return
    loading = true
    loadError = null
    try {
      preview = await api.csvPreview(path, delimiterChoice || null)
      autoMap()
    } catch (e) {
      preview = null
      loadError = errorMessage(e)
    } finally {
      loading = false
    }
  }

  async function choose() {
    const p = await pickFile({
      title: 'Import CSV',
      filters: [{ name: 'CSV', extensions: ['csv', 'tsv', 'txt'] }],
    })
    if (!p) return
    path = p
    delimiterChoice = ''
    await loadPreview()
  }

  function setMapping(i: number, value: string) {
    mapping[i] = value || null
  }

  async function start() {
    if (!preview) return
    starting = true
    loadError = null
    try {
      const id = await api.importStart(session.sessionId, {
        path,
        table,
        delimiter: preview.delimiter,
        hasHeader,
        mapping: $state.snapshot(mapping),
        nullAs: nullAs === '' ? null : nullAs,
      })
      starting = false
      await job.track(id)
    } catch (e) {
      loadError = errorMessage(e)
    } finally {
      starting = false
    }
  }

  async function close() {
    if (job.running) await job.cancel()
    onClose()
  }
</script>

<Modal title="Import CSV into {table.name}" onClose={close} width="760px" dismissable={!busy}>
  <div class="form">
    <div class="file">
      <input type="text" value={path ? basename(path) : ''} readonly placeholder="No file chosen" title={path} />
      <button onclick={choose} disabled={busy}>Choose CSV…</button>
    </div>

    {#if preview}
      <div class="opts">
        <label>
          Delimiter
          <select
            value={delimiterChoice}
            disabled={busy}
            onchange={(e) => {
              delimiterChoice = e.currentTarget.value
              void loadPreview()
            }}
          >
            {#each DELIMS as d (d.value)}
              <option value={d.value}>{d.label}</option>
            {/each}
          </select>
        </label>
        <label>
          NULL text
          <input type="text" bind:value={nullAs} disabled={busy} placeholder="(none)" />
        </label>
        <label class="inline">
          <input
            type="checkbox"
            checked={hasHeader}
            disabled={busy}
            onchange={(e) => {
              hasHeader = e.currentTarget.checked
              autoMap()
            }}
          />
          First row is a header
        </label>
        <button class="ghost" onclick={autoMap} disabled={busy}>Auto-match columns</button>
      </div>

      <div class="map">
        <table>
          <thead>
            <tr>
              {#each csvNames as name, i (i)}
                <th>
                  <div class="csvname" title={name}>{name}</div>
                  <select value={mapping[i] ?? ''} disabled={busy} aria-label="Target column for {name}" onchange={(e) => setMapping(i, e.currentTarget.value)}>
                    <option value="">(skip)</option>
                    {#each targets as c (c.name)}
                      <option value={c.name}>{c.name}</option>
                    {/each}
                  </select>
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each shownRows as row, r (r)}
              <tr>
                {#each { length: columnCount } as _, i (i)}
                  <td title={row[i]}>{row[i] ?? ''}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="muted">
        Previewing {shownRows.length} rows · {mappedCount} of {columnCount} columns mapped
      </div>
    {:else if loading}
      <div class="muted">Reading file…</div>
    {/if}

    {#if loadError}<div class="error-text" role="alert">{loadError}</div>{/if}
    <JobStatus {job} verb="Imported" />
  </div>

  {#snippet footer()}
    {#if job.running}
      <button class="danger" onclick={() => job.cancel()}>Cancel import</button>
    {:else}
      <button onclick={close}>{job.finished ? 'Close' : 'Cancel'}</button>
      <button class="primary" onclick={start} disabled={!preview || mappedCount === 0 || starting || job.finished}>
        {starting ? 'Starting…' : 'Import'}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }

  .file {
    display: flex;
    gap: 6px;
  }

  .file input {
    flex: 1;
  }

  .opts {
    display: flex;
    gap: 14px;
    align-items: flex-end;
    flex-wrap: wrap;
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
    padding-bottom: 5px;
  }

  .map {
    overflow: auto;
    max-height: 320px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  table {
    border-collapse: collapse;
    min-width: 100%;
  }

  th,
  td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    border-right: 1px solid var(--border);
    text-align: left;
    white-space: nowrap;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: normal;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--bg-sidebar);
    vertical-align: top;
  }

  .csvname {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-bottom: 3px;
  }

  th select {
    width: 100%;
    min-width: 110px;
  }
</style>
