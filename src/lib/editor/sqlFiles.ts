// Open / save .sql files in query tabs, and pick a dump to import.

import { api, errorMessage } from '../api'
import { sessions } from '../stores/sessions.svelte'
import { tabs, type QueryTab } from '../stores/tabs.svelte'
import { ui } from '../stores/ui.svelte'
import type { Engine, SessionInfo } from '../types'
import { pickFile, pickSavePath, type FileFilter } from '../util/dialogs'
import { basename } from '../util/format'
import { editorRequests } from './requests.svelte'

export const SQL_FILTERS: FileFilter[] = [{ name: 'SQL', extensions: ['sql', 'txt'] }]
const MONGO_FILTERS: FileFilter[] = [{ name: 'Query', extensions: ['json', 'js', 'txt'] }]

/** Extension a new file gets when the user typed none: MongoDB tabs hold shell scripts, the rest SQL. */
const extFor = (engine: Engine | undefined) => (engine === 'mongodb' ? 'js' : 'sql')
const engineOf = (sessionId: string): Engine | undefined => sessions.list.find((s) => s.sessionId === sessionId)?.config.engine

/** Larger files would freeze the editor; they belong in "Import SQL dump…" instead. */
export const MAX_EDIT_CHARS = 5_000_000

/** File name offered by "Save to file…": the file the tab already has, else its title. */
export function suggestFileName(tab: Pick<QueryTab, 'title' | 'file'>, ext = 'sql'): string {
  if (tab.file) return basename(tab.file)
  const name = tab.title.replace(/[\\/:*?"<>|]+/g, '_').trim() || 'query'
  return /\.[a-z0-9]{1,5}$/i.test(name) ? name : `${name}.${ext}`
}

/**
 * What the editor shows for a file's text: no byte-order mark and LF line endings (CodeMirror works in LF),
 * plus the line ending to write back when the file is mostly CRLF.
 */
export function fromFileText(raw: string): { text: string; eol?: '\r\n' } {
  const bare = raw.replace(/^\uFEFF/, '')
  const crlf = bare.split('\r\n').length - 1
  const newlines = bare.split('\n').length - 1
  return { text: bare.replace(/\r\n?/g, '\n'), eol: crlf > 0 && crlf * 2 > newlines ? '\r\n' : undefined }
}

const fileTab = (sessionId: string, path: string) =>
  tabs.forSession(sessionId).find((t): t is QueryTab => t.kind === 'query' && t.file === path)

/** Opens `path` in a query tab (or selects the tab that already has it). Returns null when it could not be opened. */
export async function openSqlPath(sessionId: string, database: string, path: string): Promise<QueryTab | null> {
  const select = (tab: QueryTab) => {
    tabs.select(tab.id)
    return tab
  }
  const open = fileTab(sessionId, path)
  if (open) return select(open)
  try {
    const raw = await api.fileReadText(path)
    if (raw.length > MAX_EDIT_CHARS) {
      ui.toast(`${basename(path)} is too large to edit. Use “Import SQL dump…” to run it instead.`, 'error')
      return null
    }
    // The same file may have been opened again while this read was in flight.
    const raced = fileTab(sessionId, path)
    if (raced) return select(raced)
    const { text, eol } = fromFileText(raw)
    return tabs.newQuery(sessionId, database, text, { file: path, eol })
  } catch (e) {
    ui.toast(`Could not open ${basename(path)}: ${errorMessage(e)}`, 'error')
    return null
  }
}

let picking = false

/** "Open SQL file…": pick a file and open it in a new query tab. */
export async function openSqlFile(session: SessionInfo): Promise<QueryTab | null> {
  if (picking) return null
  picking = true
  let path: string | null
  try {
    path = await pickFile({
      title: 'Open SQL file',
      filters: session.config.engine === 'mongodb' ? MONGO_FILTERS : SQL_FILTERS,
    })
  } finally {
    picking = false
  }
  if (!path) return null
  return openSqlPath(session.sessionId, sessions.database(session.sessionId), path)
}

/** Tabs with a save (or its path dialog) in flight: a second ⌘S meanwhile must not stack another. */
const saving = new Set<string>()

async function writeTab(tab: QueryTab, path: string): Promise<boolean> {
  const text = tab.sql
  try {
    await api.fileWriteText(path, tab.eol ? text.replace(/\n/g, tab.eol) : text)
  } catch (e) {
    ui.toast(`Could not save ${basename(path)}: ${errorMessage(e)}`, 'error')
    return false
  }
  tabs.updateQuery(tab.id, { file: path, savedSql: text, title: basename(path) })
  ui.toast(`Saved ${basename(path)}`, 'success')
  return true
}

async function saveTo(tab: QueryTab, ask: boolean): Promise<boolean> {
  if (saving.has(tab.id)) return false
  saving.add(tab.id)
  try {
    if (tab.file && !ask) return await writeTab(tab, tab.file)
    const ext = extFor(engineOf(tab.sessionId))
    const picked = await pickSavePath({
      title: 'Save to file',
      defaultPath: suggestFileName(tab, ext),
      filters: ext === 'js' ? [{ name: 'Query', extensions: ['js', 'json', 'txt'] }] : [{ name: 'SQL', extensions: ['sql'] }],
    })
    if (!picked) return false
    const path = /\.[^\\/.]+$/.test(basename(picked)) ? picked : `${picked}.${ext}`
    return await writeTab(tab, path)
  } finally {
    saving.delete(tab.id)
  }
}

/** "Save to file…": ask for a path (default name from the tab) and write the tab's text there. */
export const saveTabAs = (tab: QueryTab): Promise<boolean> => saveTo(tab, true)

/** ⌘S: write the tab to its file; a tab without one asks for a path first. */
export const saveTab = (tab: QueryTab): Promise<boolean> => saveTo(tab, false)

/** Why a dump cannot be imported into this connection, or null. */
export function importBlockedReason(session: SessionInfo): string | null {
  if (session.config.engine === 'mongodb') return 'SQL dumps cannot be imported into MongoDB.'
  if (session.config.readOnly) return 'This connection is read only.'
  return null
}

/** "Import SQL dump…": pick a .sql file, then the workspace shows the import options. */
export async function pickSqlImport(session: SessionInfo): Promise<void> {
  const blocked = importBlockedReason(session)
  if (blocked) {
    ui.toast(blocked, 'error')
    return
  }
  const path = await pickFile({ title: 'Import SQL dump', filters: SQL_FILTERS })
  if (path) editorRequests.sqlImport = { sessionId: session.sessionId, path }
}
