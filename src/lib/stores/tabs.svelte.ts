import type { Filter, TableRef } from '../types'
import { basename } from '../util/format'
import { load, save } from './persist'

export interface TableTab {
  id: string
  kind: 'table'
  sessionId: string
  table: TableRef
  title: string
  /** Filters to apply when the tab first loads (e.g. "View related" from the inspector). */
  filters?: Filter[]
  /**
   * Preview tab (TablePlus-style, shown in italics): the next single-click on another table
   * replaces it. Double-clicking, editing or filtering keeps it.
   */
  preview?: boolean
}

export interface QueryTab {
  id: string
  kind: 'query'
  sessionId: string
  database: string
  sql: string
  title: string
  /** Run the whole text once the editor is ready (e.g. "Run" on an assistant suggestion). */
  runOnOpen?: boolean
  /** The .sql file this tab was opened from / saved to (absolute path). */
  file?: string
  /** The text last read from / written to `file`. `sql !== savedSql` means unsaved changes; undefined = unknown (unsaved). */
  savedSql?: string
  /** Line ending `file` uses when it is CRLF. The editor works in LF; saving converts back, so the file keeps its endings. */
  eol?: '\r\n'
  /** Manual transaction mode ("Manual" in the editor toolbar) instead of auto-commit. */
  manual?: boolean
  /** The open transaction, once the first run in manual mode began one. Runtime only, never persisted. */
  txId?: string | null
  /** Statements that ran in the open transaction. Runtime only. */
  txStatements?: number
  /** A statement is running in this tab (mirrors its runner, so a commit from outside the tab can wait). Runtime only. */
  running?: boolean
}

/** A file-backed query tab with edits not yet written to its file. */
export function queryDirty(tab: Tab): boolean {
  return tab.kind === 'query' && !!tab.file && tab.sql !== tab.savedSql
}

/** A tab that needs an answer before it closes: unsaved file edits or an open transaction. */
export function needsCloseConfirm(tab: Tab): boolean {
  return tab.kind === 'query' && (queryDirty(tab) || !!tab.txId)
}

export type Tab = TableTab | QueryTab

function sameTable(a: TableRef, b: TableRef): boolean {
  return a.database === b.database && (a.schema ?? null) === (b.schema ?? null) && a.name === b.name
}

/** What survives a restart: open tabs per saved connection (query text included). */
type SavedTab =
  | { kind: 'table'; table: TableRef; title: string; filters?: Filter[]; preview?: boolean }
  | { kind: 'query'; database: string; sql: string; title: string; file?: string; eol?: '\r\n'; dirty?: boolean; manual?: boolean }

interface SavedTabs {
  tabs: SavedTab[]
  active: number
}

const SAVE_KEY = (connectionId: string) => `tablesail.tabs.${connectionId}`
const SAVE_DELAY_MS = 400

class TabsStore {
  list = $state<Tab[]>([])
  /** active tab id per session */
  activeBySession = $state<Record<string, string>>({})
  /** A tab whose close is waiting for an answer (see needsCloseConfirm); the workspace shows the prompt. */
  closeRequest = $state<string | null>(null)

  private seq = 1
  private queryCount: Record<string, number> = {}
  /** sessionId → saved connection id, for sessions whose tabs are persisted */
  private connections = $state<Record<string, string>>({})
  private saveTimer: ReturnType<typeof setTimeout> | null = null

  constructor() {
    $effect.root(() => {
      $effect(() => {
        // Track every tab field (including query text) of every persisted session.
        const snapshot = Object.keys(this.connections).map((sid) => [sid, this.serialize(sid)] as const)
        if (this.saveTimer) clearTimeout(this.saveTimer)
        this.saveTimer = setTimeout(() => {
          for (const [sid, data] of snapshot) {
            const conn = this.connections[sid]
            if (conn) save(SAVE_KEY(conn), data)
          }
        }, SAVE_DELAY_MS)
      })
    })
  }

  private serialize(sessionId: string): SavedTabs {
    const list = this.forSession(sessionId)
    const tabs: SavedTab[] = list.map((t) =>
      t.kind === 'table'
        ? { kind: 'table', table: { ...t.table }, title: t.title, filters: t.filters ? [...t.filters] : undefined, preview: t.preview }
        : {
            kind: 'query',
            database: t.database,
            sql: t.sql,
            title: t.title,
            file: t.file,
            eol: t.file ? t.eol : undefined,
            dirty: queryDirty(t) || undefined,
            manual: t.manual || undefined,
          },
    )
    return { tabs, active: Math.max(0, list.findIndex((t) => t.id === this.activeBySession[sessionId])) }
  }

  /** Reopens the tabs last open on this connection, and keeps saving them from now on. */
  restore(sessionId: string, connectionId: string) {
    if (this.connections[sessionId] === connectionId) return
    if (this.forSession(sessionId).length === 0) {
      const saved = load<SavedTabs | null>(SAVE_KEY(connectionId), null)
      const created: Tab[] = []
      for (const t of saved?.tabs ?? []) {
        if (t.kind === 'table' && t.table?.name) {
          created.push({ id: `t${this.seq++}`, kind: 'table', sessionId, table: t.table, title: t.title, filters: t.filters, preview: t.preview })
        } else if (t.kind === 'query' && typeof t.sql === 'string') {
          const n = Number(/^Query (\d+)$/.exec(t.title)?.[1] ?? 0)
          this.queryCount[sessionId] = Math.max(this.queryCount[sessionId] ?? 0, n)
          const file = typeof t.file === 'string' && t.file ? t.file : undefined
          created.push({
            id: `q${this.seq++}`,
            kind: 'query',
            sessionId,
            database: t.database,
            sql: t.sql,
            title: t.title,
            file,
            eol: file && t.eol === '\r\n' ? t.eol : undefined,
            savedSql: file && !t.dirty ? t.sql : undefined,
            manual: t.manual || undefined,
          })
        }
      }
      if (created.length) {
        this.list.push(...created)
        this.select(created[Math.min(saved?.active ?? 0, created.length - 1)].id)
      }
    }
    this.connections[sessionId] = connectionId
  }

  forSession(sessionId: string): Tab[] {
    return this.list.filter((t) => t.sessionId === sessionId)
  }

  active(sessionId: string | null): Tab | null {
    if (!sessionId) return null
    const id = this.activeBySession[sessionId]
    return this.list.find((t) => t.id === id) ?? null
  }

  select(id: string) {
    const tab = this.list.find((t) => t.id === id)
    if (tab) this.activeBySession[tab.sessionId] = id
  }

  openTable(sessionId: string, table: TableRef, filters?: Filter[], opts: { preview?: boolean } = {}): TableTab {
    const existing = this.list.find(
      (t): t is TableTab =>
        t.kind === 'table' && t.sessionId === sessionId && sameTable(t.table, table) && !t.filters?.length && !filters?.length,
    )
    if (existing) {
      if (!opts.preview) existing.preview = false
      this.select(existing.id)
      return existing
    }
    const tab: TableTab = {
      id: `t${this.seq++}`,
      kind: 'table',
      sessionId,
      table: { ...table },
      title: filters?.length ? `${table.name} (filtered)` : table.name,
      filters: filters?.length ? filters : undefined,
      preview: opts.preview || undefined,
    }
    // A new preview takes the old preview's place instead of adding another tab.
    const old = opts.preview
      ? this.list.findIndex((t) => t.kind === 'table' && t.sessionId === sessionId && t.preview)
      : -1
    if (old >= 0) this.list.splice(old, 1, tab)
    else this.list.push(tab)
    this.select(tab.id)
    return tab
  }

  /** Updates a query tab's own fields (the tab objects belong to this store). */
  updateQuery(
    id: string,
    patch: Partial<Pick<QueryTab, 'sql' | 'title' | 'database' | 'runOnOpen' | 'file' | 'savedSql' | 'eol' | 'manual' | 'txId' | 'txStatements' | 'running'>>,
  ) {
    const tab = this.list.find((t) => t.id === id)
    if (tab?.kind === 'query') Object.assign(tab, patch)
  }

  /** Turns a preview tab into a regular one. */
  keep(id: string) {
    const tab = this.list.find((t) => t.id === id)
    if (tab?.kind === 'table' && tab.preview) tab.preview = false
  }

  /** `file`: the tab shows that .sql file (titled with its name) and starts out saved. */
  newQuery(sessionId: string, database: string, sql = '', opts: { run?: boolean; file?: string; eol?: '\r\n' } = {}): QueryTab {
    const n = opts.file ? 0 : (this.queryCount[sessionId] = (this.queryCount[sessionId] ?? 0) + 1)
    const tab: QueryTab = {
      id: `q${this.seq++}`,
      kind: 'query',
      sessionId,
      database,
      sql,
      title: opts.file ? basename(opts.file) : `Query ${n}`,
      runOnOpen: opts.run || undefined,
      file: opts.file,
      eol: opts.file ? opts.eol : undefined,
      savedSql: opts.file ? sql : undefined,
    }
    this.list.push(tab)
    this.select(tab.id)
    return tab
  }

  /**
   * Closes a tab. A query tab with unsaved file edits or an open transaction is not closed
   * but selected and put in `closeRequest` for the user to answer; `force` closes it regardless.
   */
  close(id: string, force = false) {
    const i = this.list.findIndex((t) => t.id === id)
    if (i < 0) return
    const tab = this.list[i]
    if (!force && needsCloseConfirm(tab)) {
      this.select(id)
      this.closeRequest = id
      return
    }
    if (this.closeRequest === id) this.closeRequest = null
    const siblings = this.forSession(tab.sessionId)
    const wasActive = this.activeBySession[tab.sessionId] === id
    this.list.splice(i, 1)
    if (wasActive) {
      const pos = siblings.findIndex((t) => t.id === id)
      const next = siblings[pos + 1] ?? siblings[pos - 1]
      if (next) this.activeBySession[tab.sessionId] = next.id
      else delete this.activeBySession[tab.sessionId]
    }
  }

  removeSession(sessionId: string) {
    // Save now and stop tracking, so the next debounced save can't write an empty list.
    const conn = this.connections[sessionId]
    if (conn) save(SAVE_KEY(conn), this.serialize(sessionId))
    delete this.connections[sessionId]
    this.list = this.list.filter((t) => t.sessionId !== sessionId)
    if (this.closeRequest && !this.list.some((t) => t.id === this.closeRequest)) this.closeRequest = null
    delete this.activeBySession[sessionId]
    delete this.queryCount[sessionId]
  }

  /** Closes every tab (filtered ones too) that shows this table, e.g. after it was dropped. */
  closeTable(sessionId: string, table: TableRef) {
    for (const t of this.tableTabs(sessionId, table)) this.close(t.id)
  }

  /** Points the tabs of a renamed table at its new name (and retitles them). */
  renameTable(sessionId: string, table: TableRef, newName: string) {
    this.remountTable(sessionId, table, (t) => ({
      ...t,
      table: { ...t.table, name: newName },
      title: t.title === table.name || t.title.startsWith(`${table.name} (`) ? newName + t.title.slice(table.name.length) : t.title,
    }))
  }

  /** Reloads the open tabs of a table from the server (e.g. after TRUNCATE). */
  reloadTable(sessionId: string, table: TableRef) {
    this.remountTable(sessionId, table, (t) => ({ ...t }))
  }

  private tableTabs(sessionId: string, table: TableRef): TableTab[] {
    return this.list.filter((t): t is TableTab => t.kind === 'table' && t.sessionId === sessionId && sameTable(t.table, table))
  }

  /** Replaces matching table tabs in place under a new id: the workspace keys its views by tab id, so each one remounts and loads afresh. */
  private remountTable(sessionId: string, table: TableRef, change: (t: TableTab) => TableTab) {
    for (const old of this.tableTabs(sessionId, table)) {
      const i = this.list.findIndex((t) => t.id === old.id)
      if (i < 0) continue
      const changed = change(old)
      const next: TableTab = { ...changed, id: `t${this.seq++}`, table: { ...changed.table }, filters: old.filters ? [...old.filters] : undefined }
      this.list[i] = next
      if (this.activeBySession[sessionId] === old.id) this.activeBySession[sessionId] = next.id
    }
  }

  /** +1 / -1 cycles within the session's tabs. */
  cycle(sessionId: string, dir: 1 | -1) {
    const tabs = this.forSession(sessionId)
    if (tabs.length < 2) return
    const cur = tabs.findIndex((t) => t.id === this.activeBySession[sessionId])
    this.select(tabs[(cur + dir + tabs.length) % tabs.length].id)
  }

  /** 1-based; 9 selects the last tab, as browsers do. */
  selectIndex(sessionId: string, n: number) {
    const tabs = this.forSession(sessionId)
    const tab = n === 9 ? tabs[tabs.length - 1] : tabs[n - 1]
    if (tab) this.select(tab.id)
  }
}

export const tabs = new TabsStore()
