import { sessions } from '../stores/sessions.svelte'
import { tabs } from '../stores/tabs.svelte'
import { theme } from '../stores/theme.svelte'
import { ui } from '../stores/ui.svelte'
import { isMac } from '../util/format'

/**
 * Global shortcuts. Registered on the capture phase so they win over CodeMirror's own
 * bindings (for instance Mod-/ would otherwise toggle a comment). Returns true when handled.
 */
export function handleShortcut(e: KeyboardEvent): boolean {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const key = e.key
  const lower = key.toLowerCase()
  const sid = sessions.activeId
  const inWorkspace = sessions.view === 'workspace' && sid !== null

  // Ctrl-Tab / Ctrl-Shift-Tab cycle tabs on every platform.
  if (e.ctrlKey && key === 'Tab') {
    if (inWorkspace) tabs.cycle(sid, e.shiftKey ? -1 : 1)
    return true
  }

  if (!mod || e.altKey) return false

  if (lower === 'k' && !e.shiftKey) {
    ui.palette = !ui.palette
    return true
  }
  if (lower === 'l' && e.shiftKey) {
    theme.toggle()
    return true
  }
  if (key === '/' || key === '?') {
    ui.shortcutsHelp = !ui.shortcutsHelp
    return true
  }
  if (!inWorkspace) return false

  if (lower === 't' && !e.shiftKey) {
    tabs.newQuery(sid, sessions.database(sid))
    return true
  }
  if (lower === 'w' && !e.shiftKey) {
    const active = tabs.active(sid)
    if (active) tabs.close(active.id)
    return true
  }
  if (lower === 'p' && !e.shiftKey) {
    ui.palette = true
    return true
  }
  if (/^[1-9]$/.test(key) && !e.shiftKey) {
    tabs.selectIndex(sid, Number(key))
    return true
  }
  return false
}
