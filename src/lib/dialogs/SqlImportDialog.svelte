<!--
  Runs a .sql dump (picked beforehand) against a database as a background job: options first,
  then progress, errors and warnings. The schema is refreshed when the job ends, even partway.
-->
<script lang="ts">
  import { onDestroy } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import JobStatus from './JobStatus.svelte'
  import { JobPoller } from './job.svelte'
  import { api, errorMessage } from '../api'
  import { sessions } from '../stores/sessions.svelte'
  import type { SessionInfo } from '../types'
  import { basename } from '../util/format'

  interface Props {
    session: SessionInfo
    /** the .sql file to run */
    path: string
    onClose: () => void
  }

  let { session, path, onClose }: Props = $props()

  // svelte-ignore state_referenced_locally
  let database = $state(sessions.database(session.sessionId))
  let stopOnError = $state(true)
  let starting = $state(false)
  let startError = $state<string | null>(null)

  const job = new JobPoller()
  onDestroy(() => job.dispose())

  let busy = $derived(starting || job.running)
  let isProd = $derived(session.config.env === 'production')
  let databases = $derived(session.databases.includes(database) || !database ? session.databases : [database, ...session.databases])
  let warning = $derived(job.progress?.warning ?? null)

  // Whatever the dump did (all of it, or part of it before an error), show it in the sidebar.
  let refreshed = false
  $effect(() => {
    if (!job.finished || refreshed) return
    refreshed = true
    void sessions.refreshSchema(session.sessionId, database)
  })

  async function start() {
    starting = true
    startError = null
    try {
      const id = await api.sqlImportStart(session.sessionId, { path, database: database || null, stopOnError })
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

<Modal title="Import SQL dump" onClose={close} width="520px" dismissable={!busy}>
  <div class="form">
    <div class="file">
      <span class="muted">File</span>
      <span class="name" title={path}>{basename(path)}</span>
    </div>

    <label>
      Database
      <select bind:value={database} disabled={busy || job.finished} aria-label="Target database">
        {#each databases as db (db)}
          <option value={db}>{db || '(default)'}</option>
        {/each}
      </select>
    </label>

    <label class="inline">
      <input type="checkbox" bind:checked={stopOnError} disabled={busy || job.finished} />
      Stop on first error
    </label>
    <div class="muted hint">
      {stopOnError
        ? 'The import stops at the first failing statement; statements that already ran stay applied.'
        : 'Failing statements are skipped and counted; the rest of the file still runs.'}
    </div>

    {#if isProd}
      <div class="prod" role="note">
        <b>{session.config.name}</b> is a production connection. The statements in this file will run against it.
      </div>
    {/if}

    {#if startError}<div class="error-text" role="alert">{startError}</div>{/if}
    <JobStatus {job} verb="Executed" unit="statements" />
    {#if job.progress?.error && job.progress.rows > 0}
      <div class="muted">
        {job.progress.rows.toLocaleString('en-US')} statement{job.progress.rows === 1 ? '' : 's'} ran before it stopped and stay applied.
      </div>
    {/if}
    {#if warning}<div class="warn" role="status">{warning}</div>{/if}
  </div>

  {#snippet footer()}
    {#if job.running}
      <button class="danger" onclick={() => job.cancel()}>Cancel import</button>
    {:else}
      <!-- Not while the job is being created: closing now would leave it running with nobody to track or cancel it. -->
      <button onclick={close} disabled={starting}>{job.finished ? 'Close' : 'Cancel'}</button>
      <button class={isProd ? 'danger' : 'primary'} onclick={start} disabled={starting || job.finished}>
        {starting ? 'Starting…' : isProd ? 'Import into production' : 'Import'}
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
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .name {
    font-family: var(--mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .hint {
    margin-top: -6px;
    font-size: 11px;
  }

  .prod {
    padding: 7px 9px;
    border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
    border-radius: 6px;
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: 12px;
  }

  .warn {
    color: var(--warning);
    user-select: text;
    -webkit-user-select: text;
    word-break: break-word;
  }
</style>
