<!--
  JSON viewer / editor for one cell. Pretty-prints, validates while typing and saves
  a { kind: 'json' } input. Read-only when the cell is not editable. A truncated value
  is fetched in full first (fetchFull).
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import Modal from '../../ui/Modal.svelte'
  import { errorMessage } from '../../api'
  import { isTruncatedValue } from '../../format'
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

  function asText(v: Value | undefined): string {
    if (v === null || v === undefined) return ''
    if (typeof v === 'object' && '$json' in v) return JSON.stringify(v.$json, null, 2) ?? ''
    if (typeof v === 'object' && '$text' in v) return pretty(v.$text)
    if (typeof v === 'string') return pretty(v)
    return JSON.stringify(v, null, 2) ?? ''
  }

  function pretty(s: string): string {
    try {
      return JSON.stringify(JSON.parse(s), null, 2)
    } catch {
      return s
    }
  }

  const start = untrack(() => ({ value, truncated: isTruncatedValue(value), canFetch: !!fetchFull }))
  let text = $state(asText(start.value))
  let loading = $state(start.truncated && start.canFetch)
  let loadError = $state<string | null>(null)
  // A truncated prefix can't be saved: it would overwrite the stored document.
  let incomplete = $state(start.truncated)

  onMount(() => {
    if (!start.truncated || !fetchFull) return
    fetchFull().then(
      (full) => {
        text = asText(full)
        incomplete = false
        loading = false
      },
      (e) => {
        loadError = errorMessage(e)
        loading = false
      },
    )
  })

  const parsed = $derived.by(() => {
    if (text.trim() === '') return { ok: true as const, empty: true as const, value: null }
    try {
      return { ok: true as const, empty: false as const, value: JSON.parse(text) as unknown }
    } catch (e) {
      return { ok: false as const, error: e instanceof Error ? e.message : String(e) }
    }
  })

  const locked = $derived(readOnly || loading || incomplete)

  function format() {
    if (parsed.ok && !parsed.empty) text = JSON.stringify(parsed.value, null, 2)
  }

  function minify() {
    if (parsed.ok && !parsed.empty) text = JSON.stringify(parsed.value)
  }

  function save() {
    if (!parsed.ok || locked) return
    if (parsed.empty) {
      if (nullable) onSave({ kind: 'null' })
      return
    }
    onSave({ kind: 'json', value: parsed.value })
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
    <p class="error-text">Could not load the full value: {loadError}</p>
  {/if}
  <textarea
    class="json"
    spellcheck="false"
    readonly={locked}
    bind:value={text}
    onkeydown={onKeydown}
    placeholder={nullable ? 'NULL' : ''}
  ></textarea>
  <div class="status">
    {#if !parsed.ok}
      <span class="error-text">Invalid JSON: {parsed.error}</span>
    {:else if parsed.empty}
      <span class="muted">{nullable ? 'Empty: saves as NULL' : 'Empty'}</span>
    {:else}
      <span class="ok">Valid JSON</span>
    {/if}
    {#if readOnly}<span class="muted">Read-only</span>{/if}
  </div>

  {#snippet footer()}
    <button class="ghost" onclick={copy}>Copy</button>
    <button class="ghost" onclick={format} disabled={!parsed.ok || parsed.empty}>Format</button>
    <button class="ghost" onclick={minify} disabled={!parsed.ok || parsed.empty}>Minify</button>
    <span class="spacer"></span>
    <button onclick={onClose}>{locked ? 'Close' : 'Cancel'}</button>
    {#if !readOnly}
      <button class="primary" onclick={save} disabled={!parsed.ok || locked}>Save</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .json {
    width: 100%;
    height: 50vh;
    min-height: 220px;
    resize: vertical;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    tab-size: 2;
    white-space: pre;
  }

  .status {
    margin-top: 6px;
    display: flex;
    gap: 12px;
    font-size: 12px;
  }

  .ok {
    color: var(--success);
  }

  .spacer {
    flex: 1;
  }
</style>
