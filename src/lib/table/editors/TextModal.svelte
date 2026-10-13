<!--
  Long-text viewer / editor. A truncated cell is fetched in full (fetchFull) before it can
  be edited, so saving never overwrites the stored text with its prefix.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import Modal from '../../ui/Modal.svelte'
  import { errorMessage } from '../../api'
  import { formatCount, isTruncatedValue, valueToEditText } from '../../format'
  import type { CellInput, Value } from '../../types'

  interface Props {
    title: string
    value: Value | undefined
    readOnly: boolean
    nullable: boolean
    fetchFull?: () => Promise<Value>
    onSave: (input: CellInput) => void
    onClose: () => void
  }

  let { title, value, readOnly, nullable, fetchFull, onSave, onClose }: Props = $props()

  const start = untrack(() => ({
    text: valueToEditText(value),
    wasNull: value === null,
    truncated: isTruncatedValue(value),
    canFetch: !!fetchFull,
  }))
  let text = $state(start.text)
  let loading = $state(start.truncated && start.canFetch)
  let loadError = $state<string | null>(null)
  let incomplete = $state(start.truncated)
  let dirty = $state(false)

  onMount(() => {
    if (!start.truncated || !fetchFull) return
    fetchFull().then(
      (full) => {
        text = valueToEditText(full)
        incomplete = false
        loading = false
      },
      (e) => {
        loadError = errorMessage(e)
        loading = false
      },
    )
  })

  const locked = $derived(readOnly || loading || incomplete)

  function save() {
    if (locked || !dirty) return
    onSave({ kind: 'text', value: text })
  }

  function setNull() {
    if (!locked && nullable) onSave({ kind: 'null' })
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      // clipboard unavailable: nothing to do
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault()
      save()
    }
  }
</script>

<Modal {title} {onClose} width="680px">
  {#if loading}
    <p class="muted">Loading the full value…</p>
  {/if}
  {#if loadError}
    <p class="error-text" role="alert">Could not load the full value: {loadError}. Showing the first part only.</p>
  {/if}
  <textarea
    class="text"
    spellcheck="false"
    readonly={locked}
    bind:value={text}
    oninput={() => (dirty = true)}
    onkeydown={onKeydown}
    placeholder={start.wasNull ? 'NULL' : ''}
  ></textarea>
  <div class="status muted">
    {formatCount([...text].length)} characters{readOnly ? ' · read-only' : ''}
  </div>

  {#snippet footer()}
    <button class="ghost" onclick={copy}>Copy</button>
    {#if !readOnly && nullable}
      <button class="ghost" onclick={setNull} disabled={locked}>Set NULL</button>
    {/if}
    <span class="spacer"></span>
    <button onclick={onClose}>{readOnly ? 'Close' : 'Cancel'}</button>
    {#if !readOnly}
      <button class="primary" onclick={save} disabled={locked || !dirty}>Save</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .text {
    width: 100%;
    height: 50vh;
    min-height: 220px;
    resize: vertical;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
  }

  .status {
    margin-top: 6px;
    font-size: 12px;
  }

  .spacer {
    flex: 1;
  }
</style>
