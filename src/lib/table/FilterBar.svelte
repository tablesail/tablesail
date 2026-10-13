<!--
  Filter rows (column, operator, value), AND/OR switch and an optional raw WHERE input.
  The owner keeps the state; Enter in any input (or the Apply button) calls onApply.
-->
<script lang="ts">
  import type { FilterOp } from '../types'
  import { FILTER_OPS as OPS, opTakesValue, type FilterRow } from './filters'

  interface Props {
    columns: string[]
    rows: FilterRow[]
    matchAll: boolean
    rawMode: boolean
    rawWhere: string
    /** Placeholder for the raw input (SQL for SQL engines, a JSON filter for Mongo). */
    rawPlaceholder?: string
    onApply: () => void
    onClear: () => void
    onChange: () => void
  }

  let {
    columns,
    rows = $bindable(),
    matchAll = $bindable(),
    rawMode = $bindable(),
    rawWhere = $bindable(),
    rawPlaceholder = "e.g. status = 'active' AND created_at > '2024-01-01'",
    onApply,
    onClear,
    onChange,
  }: Props = $props()

  let nextId = 1000

  export function addRow() {
    rows = [...rows, { id: nextId++, column: columns[0] ?? '', op: 'eq', value: '' }]
    onChange()
  }

  function removeRow(id: number) {
    rows = rows.filter((r) => r.id !== id)
    onChange()
  }

  function noValue(op: FilterOp): boolean {
    return !opTakesValue(op)
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault()
      onApply()
    }
  }
</script>

<div class="filters">
  {#each rows as row (row.id)}
    <div class="frow">
      <select bind:value={row.column} onchange={onChange} aria-label="Column">
        {#each columns as c (c)}
          <option value={c}>{c}</option>
        {/each}
      </select>
      <select bind:value={row.op} onchange={onChange} aria-label="Operator">
        {#each OPS as o (o.op)}
          <option value={o.op}>{o.label}</option>
        {/each}
      </select>
      {#if noValue(row.op)}
        <span class="fill"></span>
      {:else}
        <input
          class="fill"
          type="text"
          spellcheck="false"
          placeholder={row.op === 'in' || row.op === 'notIn' ? 'comma separated values' : 'value'}
          bind:value={row.value}
          oninput={onChange}
          onkeydown={onKey}
        />
      {/if}
      <button class="ghost icon" title="Remove filter" aria-label="Remove filter" onclick={() => removeRow(row.id)}>−</button>
    </div>
  {/each}

  {#if rawMode}
    <div class="frow">
      <span class="where">WHERE</span>
      <input
        class="fill mono"
        type="text"
        spellcheck="false"
        placeholder={rawPlaceholder}
        bind:value={rawWhere}
        oninput={onChange}
        onkeydown={onKey}
      />
    </div>
  {/if}

  <div class="frow controls">
    <button class="ghost" onclick={addRow}>+ Filter</button>
    {#if rows.length > 1}
      <label class="inline">
        Match
        <select bind:value={matchAll} onchange={onChange}>
          <option value={true}>all</option>
          <option value={false}>any</option>
        </select>
      </label>
    {/if}
    <label class="inline">
      <input type="checkbox" bind:checked={rawMode} onchange={onChange} />
      Raw WHERE
    </label>
    <span class="fill"></span>
    <button onclick={onClear}>Clear</button>
    <button class="primary" onclick={onApply}>Apply</button>
  </div>
</div>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-sidebar);
  }

  .frow {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .fill {
    flex: 1;
    min-width: 0;
  }

  select {
    max-width: 200px;
  }

  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }

  .where {
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-muted);
    width: 48px;
  }

  .inline {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--fg-muted);
  }

  .icon {
    padding: 2px 8px;
  }

  .controls {
    margin-top: 2px;
  }
</style>
