<!--
  Context menu at a screen position. Items with `separator` render a rule.
  Closes on outside click, Escape, blur or scroll.
-->
<script lang="ts">
  export interface MenuItem {
    label?: string
    onclick?: () => void
    danger?: boolean
    disabled?: boolean
    separator?: boolean
    hint?: string
  }

  interface Props {
    x: number
    y: number
    items: MenuItem[]
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

  function run(item: MenuItem) {
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
></div>
<div class="menu" role="menu" tabindex="-1" bind:this={el} style:left="{left}px" style:top="{top}px">
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

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
  }

  .menu {
    position: fixed;
    z-index: 201;
    min-width: 180px;
    padding: 4px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 7px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
    outline: none;
  }

  .item {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 16px;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
  }

  .item:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-fg);
  }

  .item.danger {
    color: var(--danger);
  }

  .item.danger:hover:not(:disabled) {
    background: var(--danger);
    color: #fff;
  }

  .hint {
    opacity: 0.6;
    font-size: 11px;
  }

  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 2px;
  }
</style>
