<!--
  Console pane: every statement the app sent to a server, newest at the bottom, each
  under a `-- timestamp` comment line (like TablePlus). Self-contained: polls the
  backend while mounted.
  <ConsoleLog onClose={() => (ui.console = false)} />
-->
<script lang="ts" module>
  import { highlightSql } from '../util/sqlHighlight'

  const pad = (n: number, width: number) => String(n).padStart(width, '0')

  /** Local `YYYY-MM-DD HH:MM:SS.mmmm` (milliseconds plus a zero, the way TablePlus prints it). */
  function stamp(atMs: number): string {
    const d = new Date(atMs)
    return (
      `${d.getFullYear()}-${pad(d.getMonth() + 1, 2)}-${pad(d.getDate(), 2)} ` +
      `${pad(d.getHours(), 2)}:${pad(d.getMinutes(), 2)}:${pad(d.getSeconds(), 2)}.${pad(d.getMilliseconds(), 3)}0`
    )
  }
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { api } from '../api'
  import type { LogEntry } from '../types'
  import Icon from '../ui/Icon.svelte'

  interface Props {
    onClose?: () => void
  }

  let { onClose }: Props = $props()

  const MAX_ENTRIES = 2000
  const POLL_MS = 700
  const HIGHLIGHT_KEY = 'tablesoft.console.highlight'

  function readHighlight(): boolean {
    try {
      return localStorage.getItem(HIGHLIGHT_KEY) !== '0'
    } catch {
      return true
    }
  }

  let entries = $state.raw<LogEntry[]>([])
  let highlight = $state(readHighlight())
  let scroller: HTMLDivElement | undefined = $state()
  let lastSeq = 0
  let atBottom = true

  function setHighlight(on: boolean) {
    highlight = on
    try {
      localStorage.setItem(HIGHLIGHT_KEY, on ? '1' : '0')
    } catch {
      // ignore: persistence is a convenience
    }
  }

  let polling = false
  async function poll() {
    if (polling) return
    polling = true
    try {
      const fresh = await api.queryLogSince(lastSeq)
      if (fresh.length) {
        lastSeq = fresh[fresh.length - 1].seq
        const all = entries.length ? entries.concat(fresh) : fresh
        entries = all.length > MAX_ENTRIES ? all.slice(all.length - MAX_ENTRIES) : all
      }
    } catch {
      // the backend may be busy or restarting; the next tick tries again
    } finally {
      polling = false
    }
  }

  onMount(() => {
    void poll()
    const timer = setInterval(poll, POLL_MS)
    return () => clearInterval(timer)
  })

  function onScroll() {
    if (scroller) atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 24
  }

  // Follow new output, but only while the reader hasn't scrolled up.
  $effect(() => {
    void entries
    if (!atBottom) return
    void tick().then(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight
    })
  })

  async function clear() {
    try {
      await api.queryLogClear()
    } catch {
      // clearing the view still helps
    }
    entries = []
    atBottom = true
  }
</script>

<div class="console">
  <div class="scroll" bind:this={scroller} onscroll={onScroll}>
    {#each entries as e (e.seq)}
      <div class="entry">
        <div class="stamp">-- {stamp(e.atMs)}</div>
        {#if highlight}
          <pre class="sql">{@html highlightSql(e.sql)}</pre>
        {:else}
          <pre class="sql">{e.sql}</pre>
        {/if}
      </div>
    {:else}
      <div class="stamp">-- Statements sent to the server will appear here.</div>
    {/each}
  </div>
  <div class="bar">
    <button class="icon" title="Clear log" aria-label="Clear log" disabled={entries.length === 0} onclick={clear}>
      <Icon name="trash" size={13} />
    </button>
    <label class="toggle">
      <input type="checkbox" checked={highlight} onchange={(e) => setHighlight(e.currentTarget.checked)} />
      Enable syntax highlighting
    </label>
    <span class="spacer"></span>
    {#if onClose}
      <button class="icon" title="Close console" aria-label="Close console" onclick={onClose}>
        <Icon name="close" size={12} />
      </button>
    {/if}
  </div>
</div>

<style>
  .console {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 8px 12px;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.6;
    color: var(--fg-cell);
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }

  .entry {
    margin-bottom: 1.6em;
  }

  .stamp {
    color: var(--fg-faint);
  }

  .sql {
    margin: 0;
    font: inherit;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .sql :global(.k) {
    color: var(--syn-keyword);
  }

  .sql :global(.s) {
    color: var(--syn-string);
  }

  .sql :global(.n) {
    color: var(--syn-number);
  }

  .sql :global(.c) {
    color: var(--syn-comment);
  }

  .bar {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px;
    border-top: 1px solid var(--border);
    background: var(--bg-chrome);
    font-size: 11px;
    color: var(--fg-muted);
  }

  .bar button.icon {
    width: 22px;
    min-width: 22px;
    min-height: 22px;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    cursor: default;
  }

  .toggle input {
    margin: 0;
    accent-color: var(--accent-solid);
  }

  .spacer {
    flex: 1;
  }
</style>
