// Keyboard handling for popup menus (Svelte action), used by ContextMenu and RowMenu:
// - ArrowDown / ArrowUp move between the enabled items (wrapping), Home / End jump to the ends;
//   with the focus still on the menu itself the first ArrowDown lands on the first item
// - Enter / Space activate the focused item (they are real buttons); Tab closes the menu
// - on close, focus returns to whatever had it before (the grid, a sidebar row…)

const ITEMS = '[role="menuitem"]:not(:disabled)'

export function menuNav(node: HTMLElement, onClose: () => void) {
  let close = onClose
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null

  function onKeydown(e: KeyboardEvent) {
    const items = [...node.querySelectorAll<HTMLElement>(ITEMS)]
    const i = items.indexOf(document.activeElement as HTMLElement)
    let next: HTMLElement | undefined
    if (e.key === 'ArrowDown') next = items[i < 0 ? 0 : (i + 1) % items.length]
    else if (e.key === 'ArrowUp') next = items[i < 0 ? items.length - 1 : (i - 1 + items.length) % items.length]
    else if (e.key === 'Home') next = items[0]
    else if (e.key === 'End') next = items[items.length - 1]
    else if (e.key === 'Tab') {
      e.preventDefault()
      close()
      return
    } else return
    e.preventDefault()
    next?.focus()
  }

  node.addEventListener('keydown', onKeydown)

  return {
    update(fn: () => void) {
      close = fn
    },
    destroy() {
      node.removeEventListener('keydown', onKeydown)
      // A frame later, and only if nothing else took the focus meanwhile (a dialog the item opened, a click elsewhere).
      requestAnimationFrame(() => {
        const active = document.activeElement
        if (previous?.isConnected && (!active || active === document.body || !active.isConnected)) {
          previous.focus({ preventScroll: true })
        }
      })
    },
  }
}
