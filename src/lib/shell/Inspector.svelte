<!--
  Right-hand Row inspector: every field of the selected row, its foreign-key
  relations (jump to the related row), and the connection's measured latency.
-->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import { formatValue } from '../format'
  import { inspector } from '../stores/inspector.svelte'
  import { pulse } from '../stores/pulse.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import type { Filter, SessionInfo, Value } from '../types'

  interface Props {
    session: SessionInfo
    /** Only rows published by the visible tab are shown. */
    activeTabId: string | null
  }

  let { session, activeTabId }: Props = $props()

  let cur = $derived(
    inspector.current && inspector.current.session.sessionId === session.sessionId && inspector.current.tabId === activeTabId
      ? inspector.current
      : null,
  )

  let title = $derived.by(() => {
    if (!cur) return ''
    const pk = cur.primaryKey
      .map((k) => cur.columns.findIndex((c) => c.name === k))
      .filter((i) => i >= 0)
      .map((i) => formatValue(cur.row[i], cur.columns[i].family).text)
    return pk.length ? `#${pk.join(' · ')}` : `Row ${cur.rowIndex + 1}`
  })

  function valueText(v: Value | undefined, i: number): { text: string; cls: string } {
    if (!cur) return { text: '', cls: '' }
    const f = formatValue(v, cur.columns[i].family)
    return { text: f.text.length > 400 ? f.text.slice(0, 400) + '…' : f.text, cls: f.cls }
  }

  /** Raw text of a value, for building an equality filter. */
  function filterText(v: Value | undefined): string | null {
    if (v === null || v === undefined) return null
    if (typeof v !== 'object') return String(v)
    const o = v as Record<string, unknown>
    for (const k of ['$int', '$dec', '$uuid', '$oid', '$date', '$datetime', '$time', '$other', '$text']) {
      if (typeof o[k] === 'string') return o[k] as string
    }
    return null
  }

  function openRelated(fkIndex: number) {
    if (!cur) return
    const fk = cur.foreignKeys[fkIndex]
    const filters: Filter[] = []
    fk.columns.forEach((col, i) => {
      const ci = cur.columns.findIndex((c) => c.name === col)
      const text = ci >= 0 ? filterText(cur.row[ci]) : null
      if (text !== null) filters.push({ column: fk.referencedColumns[i], op: 'eq', value: text })
    })
    tabs.openTable(cur.session.sessionId, fk.referenced, filters)
  }

  // Latency card
  let samples = $derived(pulse.for(session.sessionId))
  let last = $derived(samples.length ? samples[samples.length - 1] : null)
  let avg = $derived(samples.length > 1 ? samples.slice(0, -1).reduce((a, b) => a + b, 0) / (samples.length - 1) : null)
  let diff = $derived(last !== null && avg ? ((last - avg) / avg) * 100 : null)
  let healthy = $derived(last === null || last < 300)
  let chart = $derived.by(() => {
    const pts = samples.slice(-24)
    if (pts.length < 2) return null
    const max = Math.max(...pts)
    const min = Math.min(...pts)
    const range = Math.max(max - min, max * 0.25, 1)
    const step = 206 / (pts.length - 1)
    const y = (v: number) => 40 - ((v - min) / range) * 32
    const line = pts.map((v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)} ${y(v).toFixed(1)}`).join(' ')
    return { line, area: `${line} L206 48 L0 48Z` }
  })
</script>

<aside class="inspector">
  <div class="ihead">
    <Icon name="panel" size={16} />
    <b>Row inspector</b>
    <button class="x" aria-label="Close inspector" onclick={() => inspector.toggle()}><Icon name="close" size={11} strokeWidth={1.8} /></button>
  </div>

  <div class="scroll">
    {#if cur}
      <div class="intro">
        <div class="overline">Selected record</div>
        <div class="rid" title={title}>{title}</div>
        {#if cur.editable}
          <span class="tag edit"><Icon name="check" size={11} strokeWidth={2} /> Editable</span>
        {:else}
          <span class="tag"><Icon name="shield" size={12} /> Safe to inspect · Read only</span>
        {/if}
        {#if cur.table}<div class="src">{cur.table.schema ? `${cur.table.schema}.` : ''}{cur.table.name}</div>{/if}
      </div>

      <div class="section">
        <div class="stitle">Properties <span class="n">{cur.columns.length}</span></div>
        {#each cur.columns as c, i (c.name + i)}
          {@const v = valueText(cur.row[i], i)}
          <div class="field">
            <span class="label">{c.name} <em>{c.typeName}</em></span>
            <div class="value {v.cls}">{v.text || ' '}</div>
          </div>
        {/each}
      </div>

      {#if cur.foreignKeys.length > 0}
        <div class="section">
          <div class="stitle">Relations <span class="arrow">↗</span></div>
          {#each cur.foreignKeys as fk, i (fk.name)}
            <div class="field">
              <span class="label">{fk.columns.join(', ')} → {fk.referenced.schema ? `${fk.referenced.schema}.` : ''}{fk.referenced.name}</span>
              <button class="related" onclick={() => openRelated(i)}>View related {fk.referenced.name} ↗</button>
            </div>
          {/each}
        </div>
      {/if}
    {:else}
      <div class="empty">
        <Icon name="table" size={22} />
        <p>Select a row to inspect every field, follow its relations and see its full values.</p>
      </div>
    {/if}

    <div class="pulse">
      <div class="phead">
        <span>Connection pulse</span>
        <span class:slow={!healthy}>● {healthy ? 'Healthy' : 'Slow'}</span>
      </div>
      {#if last !== null}
        <div class="big">{Math.round(last)}<small> ms</small></div>
        <div class="pdiff" class:up={diff !== null && diff > 0}>
          {#if diff !== null}{diff > 0 ? '↑' : '↓'} {Math.abs(diff).toFixed(1)}% vs. average{:else}Last round trip{/if}
        </div>
        {#if chart}
          <svg viewBox="0 0 206 48" preserveAspectRatio="none" aria-hidden="true">
            <path d={chart.area} class="area" />
            <path d={chart.line} class="line" />
          </svg>
        {/if}
      {:else}
        <div class="pdiff">Latency appears after the first query.</div>
      {/if}
    </div>
  </div>
</aside>

<style>
  .inspector {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
  }

  .ihead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 19px 18px 16px;
    border-bottom: 1px solid var(--border);
  }

  .ihead > :global(svg) {
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .ihead > :global(svg) {
    color: color-mix(in srgb, var(--accent) 60%, #2d3a12);
  }

  .ihead b {
    flex: 1;
    font-size: 12px;
    font-weight: 690;
  }

  .x {
    width: 22px;
    min-width: 22px;
    height: 22px;
    min-height: 22px;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .scroll {
    flex: 1;
    overflow: auto;
  }

  .intro {
    padding: 18px 18px 20px;
    border-bottom: 1px solid var(--border);
  }

  .rid {
    margin: 10px 0;
    font-size: 19px;
    font-weight: 710;
    letter-spacing: -0.035em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
    -webkit-user-select: text;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 10px;
    color: var(--success);
    background: color-mix(in srgb, var(--success) 14%, transparent);
  }

  .tag.edit {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
  }

  .src {
    margin-top: 10px;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--fg-faint);
  }

  .section {
    padding: 20px 18px 6px;
  }

  .stitle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
    font-size: 10px;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--fg-muted);
  }

  .n {
    color: var(--fg-faint);
  }

  .arrow {
    color: var(--accent);
  }

  .field {
    margin-bottom: 16px;
  }

  .label {
    display: block;
    margin-bottom: 6px;
    font-size: 10px;
    color: var(--fg-faint);
  }

  .label em {
    font-style: normal;
    opacity: 0.7;
  }

  .value {
    font-size: 11px;
    color: var(--fg);
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }

  .value.mono,
  .value.num {
    font-family: var(--mono);
    font-size: 10.5px;
  }

  .value.mono {
    color: var(--mono-fg);
  }

  .value.null,
  .value.missing {
    color: var(--fg-faint);
  }

  .value.bool.yes {
    color: var(--success);
  }

  .related {
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
    color: var(--info);
  }

  .related:hover:not(:disabled) {
    background: none;
    text-decoration: underline;
  }

  .empty {
    padding: 40px 24px;
    text-align: center;
    color: var(--fg-faint);
  }

  .empty :global(svg) {
    margin: 0 auto 12px;
  }

  .empty p {
    margin: 0;
    font-size: 11px;
    line-height: 1.6;
  }

  .pulse {
    margin: 16px;
    padding: 15px 14px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--success) 6%, var(--bg-input));
    border: 1px solid color-mix(in srgb, var(--success) 22%, var(--border));
  }

  .phead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 11px;
    font-weight: 700;
  }

  .phead span + span {
    font-size: 9px;
    font-weight: 500;
    color: var(--success);
  }

  .phead .slow {
    color: var(--warning) !important;
  }

  .big {
    margin-top: 11px;
    font-size: 22px;
    font-weight: 760;
    letter-spacing: -0.045em;
  }

  .big small {
    font-size: 11px;
    font-weight: 400;
    color: var(--fg-muted);
  }

  .pdiff {
    font-size: 10px;
    color: var(--success);
  }

  .pdiff.up {
    color: var(--warning);
  }

  svg {
    display: block;
    width: 100%;
    height: 48px;
    margin-top: 7px;
    overflow: visible;
  }

  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  :global(:root[data-theme='light']) .line {
    stroke: color-mix(in srgb, var(--accent) 70%, #2d5a12);
  }

  .area {
    fill: var(--accent);
    opacity: 0.08;
  }
</style>
