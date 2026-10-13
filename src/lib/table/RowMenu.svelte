<!--
  Small context menu at a screen position. Closes on outside click, Escape, blur or resize.
-->
<script lang="ts">
  import { menuNav } from '../ui/menuNav'
  import type { RowMenuItem } from './menu'

  interface Props {
    x: number
    y: number
    items: RowMenuItem[]
    onClose: () => void
  }

  let { x, y, items, onClose }: Props = $props()

  let el = $state<HTMLDivElement>()
  let left = $state(0)
  let top = $state(0)

  $effect(() => {
    left = x
    top = y
    if (!el) return
    const r = el.getBoundingClientRect()
    left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4))
    top = Math.max(4, Math.min(y, window.innerHeight - r.height - 4))
    el.focus()
  })

  function run(item: RowMenuItem) {
    if (item.disabled || item.separator) return
    onClose()
    item.onclick?.()
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation()
      onClose()
    }
  }
</script>

<svelte:window onkeydown={onKeydown} onblur={onClose} onresize={onClose} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="overlay"
  onmousedown={onClose}
  oncontextmenu={(e) => {
    e.preventDefault()
    onClose()
  }}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="menu"
    role="menu"
    tabindex="-1"
    bind:this={el}
    use:menuNav={onClose}
    style:left="{left}px"
    style:top="{top}px"
    onmousedown={(e) => e.stopPropagation()}
    oncontextmenu={(e) => e.preventDefault()}
  >
    {#each items as item, i (i)}
      {#if item.separator}
        <div class="sep"></div>
      {:else}
        <button
          class="item"
          class:danger={item.danger}
          role="menuitem"
          disabled={item.disabled}
          onclick={() => run(item)}
        >
          <span>{item.label}</span>
          {#if item.hint}<span class="hint">{item.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
  }

  .menu {
    position: fixed;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elev, var(--bg));
    border: 1px solid var(--border);
    border-radius: 7px;
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.28);
    outline: none;
  }

  .item {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    width: 100%;
    padding: 4px 10px;
    border: none;
    border-radius: 4px;
    background: transparent;
    text-align: left;
  }

  .item:hover:not(:disabled),
  .item:focus-visible:not(:disabled) {
    background: var(--accent);
    color: var(--accent-fg, #fff);
    outline: none;
  }

  .item.danger {
    color: var(--danger);
  }

  .item.danger:hover:not(:disabled),
  .item.danger:focus-visible:not(:disabled) {
    color: #fff;
    background: var(--danger);
  }

  .hint {
    opacity: 0.6;
    font-size: 11px;
  }

  .sep {
    height: 1px;
    margin: 4px 2px;
    background: var(--border);
  }
</style>
