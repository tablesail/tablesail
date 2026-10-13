<!-- CodeMirror host. The CodeMirror bundle loads on first mount (dynamic import). -->
<script lang="ts">
  import { onMount } from 'svelte'
  import type { EditorHandle, SchemaConfig } from './cm'

  interface Props {
    value: string
    schema: SchemaConfig
    onChange: (text: string) => void
    onRun: () => void
    onRunAll: () => void
  }

  let { value, schema, onChange, onRun, onRunAll }: Props = $props()

  let host = $state<HTMLDivElement>()
  let handle: EditorHandle | null = null
  let loading = $state(true)

  onMount(() => {
    let disposed = false
    void import('./cm').then(({ createEditor }) => {
      if (disposed || !host) return
      handle = createEditor({
        parent: host,
        doc: value,
        schema,
        onChange: (t) => onChange(t),
        onRun: () => onRun(),
        onRunAll: () => onRunAll(),
      })
      loading = false
    })
    return () => {
      disposed = true
      handle?.destroy()
      handle = null
    }
  })

  // Autocomplete schema / dialect follow the snapshot and engine.
  $effect(() => {
    const cfg = schema
    handle?.setSchema({ engine: cfg.engine, columns: cfg.columns, defaultSchema: cfg.defaultSchema })
  })

  export function getText(): string {
    return handle?.getText() ?? value
  }
  export function getSelection() {
    return handle?.getSelection() ?? null
  }
  export function getCursor(): number {
    return handle?.getCursor() ?? 0
  }
  export function setText(text: string) {
    handle?.setText(text)
  }
  export function focus() {
    handle?.focus()
  }
</script>

<div class="editor" bind:this={host}>
  {#if loading}<div class="loading muted">Loading editor…</div>{/if}
</div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }

  .loading {
    padding: 10px;
  }
</style>
