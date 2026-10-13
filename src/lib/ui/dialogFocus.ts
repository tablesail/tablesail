// Focus handling for dialogs, palettes and popovers (Svelte action):
// - on open, focus the element marked [autofocus], else the first field, else the first button
//   (the HTML `autofocus` attribute is ignored when something else, like the grid, has focus)
// - Tab and Shift-Tab stay inside the dialog
// - on close, focus returns to whatever had it before (the grid, the editor…)

const FOCUSABLE =
  'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), [href], [tabindex]:not([tabindex="-1"]), [contenteditable="true"]'

function focusables(node: HTMLElement): HTMLElement[] {
  return [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement)
}

export function dialogFocus(node: HTMLElement) {
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null

  const frame = requestAnimationFrame(() => {
    if (node.contains(document.activeElement)) return
    const target =
      node.querySelector<HTMLElement>('[autofocus]') ??
      node.querySelector<HTMLElement>('input:not([disabled]):not([type="hidden"]):not([type="checkbox"]), textarea:not([disabled]), select:not([disabled])') ??
      focusables(node)[0] ??
      node
    target.focus()
  })

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Tab') return
    const items = focusables(node)
    if (items.length === 0) {
      e.preventDefault()
      return
    }
    const first = items[0]
    const last = items[items.length - 1]
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault()
      first.focus()
    }
  }

  node.addEventListener('keydown', onKeydown)

  return {
    destroy() {
      cancelAnimationFrame(frame)
      node.removeEventListener('keydown', onKeydown)
      // Give focus back a frame later: restoring it while the closing key (Enter) is still being
      // dispatched would let the browser "press" the restored button and reopen the dialog.
      requestAnimationFrame(() => {
        const active = document.activeElement
        // Only if nothing else took the focus meanwhile (another dialog, a click elsewhere).
        if (previous?.isConnected && (!active || active === document.body || !active.isConnected)) {
          previous.focus({ preventScroll: true })
        }
      })
    },
  }
}
