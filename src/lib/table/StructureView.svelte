<!--
  Read-only structure of a table: columns, indexes, foreign keys and the DDL.
-->
<script lang="ts">
  import type { TableStructure } from '../types'
  import { highlightSql } from '../util/sqlHighlight'

  interface Props {
    structure: TableStructure
  }

  let { structure }: Props = $props()

  let copied = $state(false)
  let copyTimer: ReturnType<typeof setTimeout> | undefined

  async function copyDdl() {
    if (!structure.ddl) return
    try {
      await navigator.clipboard.writeText(structure.ddl)
      copied = true
      clearTimeout(copyTimer)
      copyTimer = setTimeout(() => (copied = false), 1500)
    } catch {
      // clipboard unavailable: the DDL is selectable
    }
  }

  function qualified(t: { database: string; schema?: string | null; name: string }): string {
    return [t.schema, t.name].filter(Boolean).join('.')
  }
</script>

<div class="structure">
  <section>
    <h3>Columns <span class="count">{structure.columns.length}</span></h3>
    <table>
      <thead>
        <tr>
          <th class="idx">#</th>
          <th>Name</th>
          <th>Type</th>
          <th>Nullable</th>
          <th>Default</th>
          <th>Attributes</th>
          <th>Comment</th>
        </tr>
      </thead>
      <tbody>
        {#each structure.columns as col, i (col.name)}
          <tr>
            <td class="idx">{i + 1}</td>
            <td class="name">{col.name}</td>
            <td class="mono">{col.dataType}</td>
            <td>{col.nullable ? 'YES' : 'NO'}</td>
            <td class="mono">
              {#if col.default === null}<span class="muted">—</span>{:else}{col.default}{/if}
            </td>
            <td>
              {#if col.isPrimaryKey}<span class="pill pk">PK</span>{/if}
              {#if col.isAutoIncrement}<span class="pill">auto</span>{/if}
              {#if col.isGenerated}<span class="pill">generated</span>{/if}
              {#if col.enumValues}<span class="pill" title={col.enumValues.join(', ')}>enum</span>{/if}
            </td>
            <td class="muted">{col.comment ?? ''}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <section>
    <h3>Indexes <span class="count">{structure.indexes.length}</span></h3>
    {#if structure.indexes.length === 0}
      <p class="muted none">No indexes.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Name</th><th>Columns</th><th>Unique</th><th>Method</th></tr>
        </thead>
        <tbody>
          {#each structure.indexes as ix (ix.name)}
            <tr>
              <td class="name">
                {ix.name}
                {#if ix.primary}<span class="pill pk">PRIMARY</span>{/if}
              </td>
              <td class="mono">{ix.columns.join(', ')}</td>
              <td>{ix.unique ? 'YES' : 'NO'}</td>
              <td class="mono">{ix.method ?? ''}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section>
    <h3>Foreign keys <span class="count">{structure.foreignKeys.length}</span></h3>
    {#if structure.foreignKeys.length === 0}
      <p class="muted none">No foreign keys.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Name</th><th>Columns</th><th>References</th><th>On update</th><th>On delete</th></tr>
        </thead>
        <tbody>
          {#each structure.foreignKeys as fk (fk.name)}
            <tr>
              <td class="name">{fk.name}</td>
              <td class="mono">{fk.columns.join(', ')}</td>
              <td class="mono">{qualified(fk.referenced)} ({fk.referencedColumns.join(', ')})</td>
              <td>{fk.onUpdate ?? ''}</td>
              <td>{fk.onDelete ?? ''}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section>
    <h3>
      DDL
      {#if structure.ddl}
        <button class="ghost small" onclick={copyDdl}>{copied ? 'Copied' : 'Copy'}</button>
      {/if}
    </h3>
    {#if structure.ddl}
      <pre class="ddl">{@html highlightSql(structure.ddl)}</pre>
    {:else}
      <p class="muted none">This engine does not provide a DDL statement for the table.</p>
    {/if}
  </section>
</div>

<style>
  .structure {
    height: 100%;
    overflow: auto;
    padding: 12px 16px 24px;
    user-select: text;
    -webkit-user-select: text;
    cursor: auto;
  }

  section {
    margin-bottom: 22px;
  }

  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
  }

  .count {
    font-weight: 400;
  }

  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 12px;
  }

  th,
  td {
    text-align: left;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  th {
    font-weight: 600;
    color: var(--fg-muted);
    background: var(--bg-sidebar);
    position: sticky;
    top: 0;
  }

  td.name {
    font-weight: 600;
  }

  .mono {
    font-family: var(--mono);
    font-size: 11.5px;
  }

  .idx {
    width: 36px;
    text-align: right;
    color: var(--fg-muted);
  }

  .pill {
    display: inline-block;
    margin-right: 4px;
    padding: 0 5px;
    font-size: 10px;
    font-weight: 600;
    line-height: 15px;
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--fg-muted);
  }

  .pill.pk {
    color: var(--accent);
    border-color: var(--accent);
  }

  .none {
    margin: 0;
  }

  .ddl :global(.k) {
    color: var(--syn-keyword);
  }

  .ddl :global(.s) {
    color: var(--syn-string);
  }

  .ddl :global(.n) {
    color: var(--syn-number);
  }

  .ddl :global(.c) {
    color: var(--syn-comment);
  }

  .ddl {
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .small {
    padding: 1px 8px;
    font-size: 11px;
    text-transform: none;
    letter-spacing: 0;
  }
</style>
