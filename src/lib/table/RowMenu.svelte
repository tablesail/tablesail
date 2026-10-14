<!--
  Small context menu at a screen position. Closes on outside click, Escape, blur or resize.
  An item with `submenu` opens a flyout beside it: on hover or click, with ArrowRight / Enter from the
  keyboard; ArrowLeft or Escape closes just the flyout.
-->
<script lang="ts">
  import { tick } from 'svelte'
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

  // The open flyout: which item owns it and where it goes.
  let subIndex = $state<number | null>(null)
  let subEl = $state<HTMLDivElement>()
  let subLeft = $state(0)
  let subTop = $state(0)
  let subAnchor: DOMRect | null = null
  let closeTimer: ReturnType<typeof setTimeout> | undefined
  const sub = $derived(subIndex === null ? null : items[subIndex])

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
    if (item.disabled || item.separator || item.submenu) return
    onClose()
    item.onclick?.()
  }

  // Place the flyout to the right of its item, or to the left when there is no room.
  $effect(() => {
    const a = subAnchor
    if (!subEl || !a) return
    const r = subEl.getBoundingClientRect()
    subLeft = a.right + r.width + 4 > window.innerWidth ? Math.max(4, a.left - r.width + 2) : a.right - 2
    subTop = Math.max(4, Math.min(a.top - 5, window.innerHeight - r.height - 4))
  })

  function openSub(i: number, target: HTMLElement, focusFirst = false) {
    clearTimeout(closeTimer)
    if (items[i].disabled) return
    subAnchor = target.getBoundingClientRect()
    subLeft = subAnchor.right - 2
    subTop = subAnchor.top - 5
    subIndex = i
    if (focusFirst) {
      void tick().then(() => subEl?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus())
    }
  }

  /** Closes the flyout; with `refocus` the keyboard goes back to the item that owned it. */
  function closeSub(refocus = false) {
    clearTimeout(closeTimer)
    const owner = subIndex === null ? null : el?.querySelectorAll<HTMLElement>('[role="menuitem"]')[ownerPosition(subIndex)]
    subIndex = null
    subAnchor = null
    if (refocus) owner?.focus()
  }

  // Index among the menu's buttons (separators are not menuitems).
  function ownerPosition(itemIndex: number): number {
    return items.slice(0, itemIndex).filter((x) => !x.separator).length
  }

  // Moving the pointer to another item closes the flyout after a moment, so a diagonal move into it still works.
  function leaveSub() {
    clearTimeout(closeTimer)
    closeTimer = setTimeout(() => closeSub(), 160)
  }

  function onItemKeydown(e: KeyboardEvent, i: number, item: RowMenuItem) {
    if (item.submenu && !item.disabled && (e.key === 'ArrowRight' || e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault()
      e.stopPropagation()
      openSub(i, e.currentTarget as HTMLElement, true)
    }
  }

  function onSubKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft') {
      e.preventDefault()
      e.stopPropagation()
      closeSub(true)
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation()
      if (subIndex === null) {
        onClose()
      } else {
        closeSub(subEl?.contains(document.activeElement) ?? false)
      }
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
      {:else if item.submenu}
        <button
          class="item"
          class:open={subIndex === i}
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={subIndex === i}
          disabled={item.disabled}
          onclick={(e) => openSub(i, e.currentTarget)}
          onmouseenter={(e) => openSub(i, e.currentTarget)}
          onkeydown={(e) => onItemKeydown(e, i, item)}
        >
          <span>{item.label}</span>
          <span class="chev" aria-hidden="true">▸</span>
        </button>
      {:else}
        <button
          class="item"
          class:danger={item.danger}
          role="menuitem"
          disabled={item.disabled}
          onclick={() => run(item)}
          onmouseenter={() => subIndex !== null && leaveSub()}
          onfocus={() => subIndex !== null && closeSub()}
        >
          <span>{item.label}</span>
          {#if item.hint}<span class="hint">{item.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>

  {#if sub?.submenu}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="menu"
      role="menu"
      aria-label={sub.label}
      tabindex="-1"
      bind:this={subEl}
      use:menuNav={onClose}
      style:left="{subLeft}px"
      style:top="{subTop}px"
      onmousedown={(e) => e.stopPropagation()}
      onmouseenter={() => clearTimeout(closeTimer)}
      oncontextmenu={(e) => e.preventDefault()}
      onkeydown={onSubKeydown}
    >
      {#each sub.submenu as item, i (i)}
        {#if item.separator}
          <div class="sep"></div>
        {:else}
          <button class="item" class:danger={item.danger} role="menuitem" disabled={item.disabled} onclick={() => run(item)}>
            <span>{item.label}</span>
            {#if item.hint}<span class="hint">{item.hint}</span>{/if}
          </button>
        {/if}
      {/each}
    </div>
  {/if}
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
  .item:focus-visible:not(:disabled),
  .item.open:not(:disabled) {
    background: var(--accent);
    color: var(--accent-fg, #fff);
    outline: none;
  }

  .chev {
    opacity: 0.7;
    font-size: 10px;
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
