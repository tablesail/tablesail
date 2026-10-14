<script lang="ts">
  import type { JobPoller } from './job.svelte'
  import { formatBytes, formatCount, formatDuration } from '../util/format'

  interface Props {
    job: JobPoller
    verb: string
    /** what the job counts; "rows" for exports and CSV imports */
    unit?: string
  }

  let { job, verb, unit = 'rows' }: Props = $props()
  let p = $derived(job.progress)
</script>

{#if p}
  <div class="job">
    <div class="bar" class:indeterminate={job.running}>
      <div class="fill" class:full={p.done}></div>
    </div>
    <div class="line">
      {#if p.error}
        <span class="error-text" role="alert">{p.error}</span>
      {:else if p.cancelled}
        <span class="warn">Cancelled after {formatCount(p.rows)} {unit}.</span>
      {:else if p.done}
        <span class="ok">{verb} {formatCount(p.rows)} {unit}{p.bytes ? ` · ${formatBytes(p.bytes)}` : ''} in {formatDuration(job.elapsedMs)}.</span>
      {:else}
        <span>{formatCount(p.rows)} {unit}{p.bytes ? ` · ${formatBytes(p.bytes)}` : ''} · {formatDuration(job.elapsedMs)}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .job {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
    position: relative;
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    width: 0;
    background: var(--accent);
  }

  .fill.full {
    width: 100%;
    background: var(--success);
  }

  .indeterminate .fill {
    width: 30%;
    animation: slide 1.1s ease-in-out infinite;
  }

  @keyframes slide {
    from {
      left: -30%;
    }
    to {
      left: 100%;
    }
  }

  .ok {
    color: var(--success);
  }

  .warn {
    color: var(--warning);
  }

  .line {
    user-select: text;
    -webkit-user-select: text;
    word-break: break-word;
  }
</style>
