import { openSqlFile, saveTab, saveTabAs } from '../editor/sqlFiles'
import { finishTx } from '../editor/transaction'
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
  if (key === ',') {
    ui.settings = true
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
  // ⌘R reloads the open table (TableView handles that); elsewhere it refreshes the schema.
  if (lower === 'r' && !e.shiftKey && tabs.active(sid)?.kind !== 'table') {
    void sessions.refreshSchema(sid, sessions.database(sid))
    return true
  }
  if (lower === 'p' && !e.shiftKey) {
    ui.palette = true
    return true
  }

  // SQL files and manual transactions. Only claimed where they apply, so ⌘S still commits a table's
  // pending changes (TableView). On a query tab ⌘S writes its file, or asks where to put one.
  // A held key (auto-repeat) is swallowed rather than stacking dialogs / repeating the action.
  const tab = tabs.active(sid)
  const query = tab?.kind === 'query' ? tab : null
  if (lower === 'o' && !e.shiftKey) {
    const session = sessions.active
    if (!session) return false
    if (!e.repeat) void openSqlFile(session)
    return true
  }
  if (lower === 's' && query) {
    if (!e.repeat) void (e.shiftKey ? saveTabAs(query) : saveTab(query))
    return true
  }
  // ⌘⇧C is "copy with headers" while a data grid has the focus (a query result), so it must never commit from there.
  const copyInGrid = lower === 'c' && !!(e.target as Element | null)?.closest?.('[role="grid"]')
  if (e.shiftKey && (lower === 'c' || lower === 'r') && query?.txId && !copyInGrid) {
    if (!e.repeat) void finishTx(query, lower === 'c' ? 'commit' : 'rollback')
    return true
  }
  if (/^[1-9]$/.test(key) && !e.shiftKey) {
    tabs.selectIndex(sid, Number(key))
    return true
  }
  return false
}
