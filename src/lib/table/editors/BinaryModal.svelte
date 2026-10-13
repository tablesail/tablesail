<!--
  Binary preview: size, hex dump of the loaded bytes and, when the data is an image, a
  thumbnail. "Load full value" fetches the complete cell (fetchFull).
-->
<script lang="ts">
  import { untrack } from 'svelte'
  import Modal from '../../ui/Modal.svelte'
  import { errorMessage } from '../../api'
  import { base64ToBytes, bytesToHex, formatBytes, isTruncatedValue } from '../../format'
  import type { Value } from '../../types'

  interface Props {
    title: string
    value: Value | undefined
    nullable: boolean
    readOnly: boolean
    fetchFull?: () => Promise<Value>
    onSetNull: () => void
    onClose: () => void
  }

  let { title, value, nullable, readOnly, fetchFull, onSetNull, onClose }: Props = $props()

  /** Largest dump rendered; the rest is only counted. */
  const DUMP_LIMIT = 64 * 1024

  let current = $state<Value | undefined>(untrack(() => value))
  let loading = $state(false)
  let loadError = $state<string | null>(null)

  const info = $derived.by(() => {
    const v = current
    if (v === null || v === undefined) return { bytes: new Uint8Array(0), total: 0, partial: false, isNull: v === null }
    if (typeof v === 'object' && '$bytes' in v) {
      const bytes = base64ToBytes(v.$bytes)
      return { bytes, total: v.len, partial: isTruncatedValue(v), isNull: false }
    }
    // Not bytes (a text column holding binary, say): show its UTF-8.
    const text = typeof v === 'object' ? JSON.stringify(v) : String(v)
    const bytes = new TextEncoder().encode(text)
    return { bytes, total: bytes.length, partial: false, isNull: false }
  })

  const shown = $derived(Math.min(info.bytes.length, DUMP_LIMIT))

  const dump = $derived.by(() => {
    const b = info.bytes
    const lines: string[] = []
    for (let i = 0; i < shown; i += 16) {
      const row = b.subarray(i, Math.min(i + 16, shown))
      let hex = ''
      let ascii = ''
      for (let j = 0; j < 16; j++) {
        if (j < row.length) {
          hex += row[j].toString(16).padStart(2, '0') + ' '
          ascii += row[j] >= 32 && row[j] < 127 ? String.fromCharCode(row[j]) : '.'
        } else {
          hex += '   '
        }
        if (j === 7) hex += ' '
      }
      lines.push(`${i.toString(16).padStart(8, '0')}  ${hex} |${ascii}|`)
    }
    return lines.join('\n')
  })

  function mimeOf(b: Uint8Array): string | null {
    if (b.length > 8 && b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return 'image/png'
    if (b.length > 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return 'image/jpeg'
    if (b.length > 6 && b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x38) return 'image/gif'
    if (b.length > 12 && b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45)
      return 'image/webp'
    return null
  }

  const imageUrl = $derived.by(() => {
    const mime = mimeOf(info.bytes)
    if (!mime || info.partial) return null
    return URL.createObjectURL(new Blob([info.bytes as BlobPart], { type: mime }))
  })

  $effect(() => {
    const url = imageUrl
    return () => {
      if (url) URL.revokeObjectURL(url)
    }
  })

  async function load() {
    if (!fetchFull) return
    loading = true
    loadError = null
    try {
      current = await fetchFull()
    } catch (e) {
      loadError = errorMessage(e)
    } finally {
      loading = false
    }
  }

  async function copyHex() {
    try {
      await navigator.clipboard.writeText('0x' + bytesToHex(info.bytes))
    } catch {
      // clipboard unavailable: nothing to do
    }
  }
</script>

<Modal {title} {onClose} width="720px">
  {#if info.isNull}
    <p class="muted">NULL</p>
  {:else}
    <p class="meta">
      <strong>{formatBytes(info.total)}</strong> ({info.total.toLocaleString('en-US')} bytes)
      {#if info.partial}
        <span class="muted"> · loaded {formatBytes(info.bytes.length)}</span>
      {/if}
      {#if info.bytes.length > shown}
        <span class="muted"> · dump shows the first {formatBytes(shown)}</span>
      {/if}
    </p>
    {#if loadError}<p class="error-text">{loadError}</p>{/if}
    {#if imageUrl}
      <img class="preview" src={imageUrl} alt="Stored data" />
    {/if}
    <pre class="dump">{dump}</pre>
  {/if}

  {#snippet footer()}
    {#if info.partial && fetchFull}
      <button onclick={load} disabled={loading}>{loading ? 'Loading…' : 'Load full value'}</button>
    {/if}
    {#if !info.isNull}<button class="ghost" onclick={copyHex}>Copy hex</button>{/if}
    {#if !readOnly && nullable && !info.isNull}
      <button class="ghost" onclick={onSetNull}>Set NULL</button>
    {/if}
    <span class="spacer"></span>
    <button onclick={onClose}>Close</button>
  {/snippet}
</Modal>

<style>
  .meta {
    margin: 0 0 8px;
  }

  .dump {
    margin: 0;
    padding: 8px;
    max-height: 48vh;
    overflow: auto;
    font-family: var(--mono);
    font-size: 11.5px;
    line-height: 1.5;
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 5px;
    user-select: text;
    -webkit-user-select: text;
  }

  .preview {
    display: block;
    max-width: 100%;
    max-height: 220px;
    margin: 0 0 8px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: repeating-conic-gradient(rgb(128 128 128 / 0.18) 0% 25%, transparent 0% 50%) 50% / 16px 16px;
  }

  .spacer {
    flex: 1;
  }
</style>
