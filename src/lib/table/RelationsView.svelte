<!-- Relations tab: this table's columns and each table its foreign keys point to. -->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import type { TableRef, TableStructure } from '../types'

  interface Props {
    structure: TableStructure
    onOpen: (table: TableRef) => void
  }

  let { structure, onOpen }: Props = $props()

  let fkCols = $derived(new Set(structure.foreignKeys.flatMap((f) => f.columns)))

  function qualified(t: TableRef) {
    return t.schema ? `${t.schema}.${t.name}` : t.name
  }
</script>

<div class="relations">
  <div class="rhead">
    <b>Relationships</b>
    <span class="muted">
      {structure.foreignKeys.length === 0
        ? 'No foreign keys on this table'
        : `${structure.foreignKeys.length} foreign key${structure.foreignKeys.length === 1 ? '' : 's'}`}
    </span>
  </div>

  <div class="diagram">
    <div class="card self">
      <header><Icon name="table" size={13} /> {structure.table.name}</header>
      {#each structure.columns as c (c.name)}
        <div class="col">
          {#if c.isPrimaryKey}<Icon name="key" size={12} />{:else if fkCols.has(c.name)}<span class="fk">↗</span>{:else}<span class="sp"></span>{/if}
          <span class="cname">{c.name}</span>
          <span class="type">{c.dataType}</span>
        </div>
      {/each}
    </div>

    {#if structure.foreignKeys.length > 0}
      <div class="links">
        {#each structure.foreignKeys as fk (fk.name)}
          <div class="link">
            <div class="arrow">
              <span class="via">{fk.columns.join(', ')}</span>
              <span class="line"></span>
              <span class="head">⟶</span>
            </div>
            <button class="card target" onclick={() => onOpen(fk.referenced)} title="Open {qualified(fk.referenced)}">
              <header><Icon name="table" size={13} /> {qualified(fk.referenced)}</header>
              {#each fk.referencedColumns as rc (rc)}
                <div class="col"><Icon name="key" size={12} /><span class="cname">{rc}</span></div>
              {/each}
              <div class="rule">
                {#if fk.onDelete}on delete {fk.onDelete.toLowerCase()}{/if}{#if fk.onDelete && fk.onUpdate} · {/if}{#if fk.onUpdate}on update {fk.onUpdate.toLowerCase()}{/if}
              </div>
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .relations {
    height: 100%;
    overflow: auto;
  }

  .rhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 25px 24px 15px;
    font-size: 14px;
  }

  .rhead .muted {
    font-size: 11px;
  }

  .diagram {
    display: flex;
    align-items: flex-start;
    gap: 0;
    padding: 10px 24px 30px;
  }

  .card {
    display: block;
    width: 250px;
    flex: none;
    overflow: hidden;
    padding: 0;
    min-height: 0;
    text-align: left;
    border: 1px solid var(--border-input);
    border-radius: 10px;
    background: var(--bg-input);
    box-shadow: none;
    color: var(--fg-cell);
  }

  .card header {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 13px 15px;
    font-weight: 750;
    color: var(--fg);
    background: var(--row-selected);
  }

  .target:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--bg-input);
  }

  .col {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 15px;
    border-top: 1px solid var(--border);
    font-size: 11px;
  }

  .col :global(svg) {
    color: var(--warning);
  }

  .fk {
    width: 12px;
    color: var(--info);
    text-align: center;
  }

  .sp {
    width: 12px;
  }

  .cname {
    flex: 1;
    font-family: var(--mono);
    color: var(--mono-fg);
  }

  .type {
    padding: 2px 5px;
    border-radius: 4px;
    font-size: 9px;
    color: var(--fg-muted);
    background: var(--bg-chip);
    border: 1px solid var(--border-input);
  }

  .rule {
    padding: 9px 15px;
    border-top: 1px solid var(--border);
    font-size: 10px;
    color: var(--fg-faint);
  }

  .links {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .link {
    display: flex;
    align-items: flex-start;
  }

  .arrow {
    position: relative;
    width: 150px;
    margin-top: 20px;
    display: flex;
    align-items: center;
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .arrow {
    color: color-mix(in srgb, var(--accent) 60%, #2d3a12);
  }

  .via {
    position: absolute;
    top: -16px;
    left: 18px;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--fg-muted);
  }

  .line {
    flex: 1;
    height: 1.5px;
    margin-left: 10px;
    background: currentColor;
    opacity: 0.6;
  }

  .head {
    font-size: 18px;
    line-height: 1;
    margin-right: 6px;
  }
</style>
