<!-- Mod-P: fuzzy search over every table in the current database snapshot. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import type { TableInfo } from '../types'
  import { fuzzyFilter } from '../util/fuzzy'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  let query = $state('')
  let index = $state(0)
  let listEl = $state<HTMLUListElement>()

  let session = $derived(sessions.active)
  let all = $derived.by<TableInfo[]>(() => {
    if (!session) return []
    const snap = sessions.schemaEntry(session.sessionId, sessions.database(session.sessionId)).snapshot
    return snap ? Object.values(snap.schemas).flat() : []
  })
  let results = $derived(fuzzyFilter(all, query, (t) => (t.schema ? `${t.schema}.${t.name}` : t.name), 50))

  $effect(() => {
    void query
    index = 0
  })

  $effect(() => {
    listEl?.children[index]?.scrollIntoView({ block: 'nearest' })
  })

  function open(t: TableInfo) {
    if (!session) return
    tabs.openTable(session.sessionId, { database: t.database, schema: t.schema ?? null, name: t.name })
    onClose()
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      index = Math.min(results.length - 1, index + 1)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      index = Math.max(0, index - 1)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      if (results[index]) open(results[index])
    }
  }
</script>

<Modal title="Open table" {onClose} width="480px">
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="q"
    type="text"
    placeholder="Type a table name"
    bind:value={query}
    onkeydown={onKeydown}
    autofocus
    spellcheck="false"
  />
  <ul bind:this={listEl}>
    {#each results as t, i (t.schema + '\u0000' + t.name)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <li class:sel={i === index} onclick={() => open(t)} onmousemove={() => (index = i)}>
        <span>{t.name}</span>
        {#if t.schema}<span class="muted">{t.schema}</span>{/if}
        <span class="kind muted">{t.kind}</span>
      </li>
    {:else}
      <li class="muted none">{all.length ? 'No matches.' : 'Schema not loaded yet.'}</li>
    {/each}
  </ul>
</Modal>

<style>
  .q {
    width: 100%;
    margin-bottom: 8px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 340px;
    overflow: auto;
  }

  li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 4px 8px;
    border-radius: 5px;
  }

  li.sel {
    background: var(--accent);
    color: var(--accent-fg);
  }

  li.sel .muted {
    color: inherit;
    opacity: 0.8;
  }

  .kind {
    margin-left: auto;
    font-size: 11px;
  }

  .none {
    justify-content: center;
  }
</style>
