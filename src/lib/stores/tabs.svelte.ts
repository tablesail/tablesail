import type { Filter, TableRef } from '../types'
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
}

export type Tab = TableTab | QueryTab

function sameTable(a: TableRef, b: TableRef): boolean {
  return a.database === b.database && (a.schema ?? null) === (b.schema ?? null) && a.name === b.name
}

/** What survives a restart: open tabs per saved connection (query text included). */
type SavedTab =
  | { kind: 'table'; table: TableRef; title: string; filters?: Filter[]; preview?: boolean }
  | { kind: 'query'; database: string; sql: string; title: string }

interface SavedTabs {
  tabs: SavedTab[]
  active: number
}

const SAVE_KEY = (connectionId: string) => `tablesoft.tabs.${connectionId}`
const SAVE_DELAY_MS = 400

class TabsStore {
  list = $state<Tab[]>([])
  /** active tab id per session */
  activeBySession = $state<Record<string, string>>({})

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
        : { kind: 'query', database: t.database, sql: t.sql, title: t.title },
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
          created.push({ id: `q${this.seq++}`, kind: 'query', sessionId, database: t.database, sql: t.sql, title: t.title })
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
      title: table.name,
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

  /** Turns a preview tab into a regular one. */
  keep(id: string) {
    const tab = this.list.find((t) => t.id === id)
    if (tab?.kind === 'table' && tab.preview) tab.preview = false
  }

  newQuery(sessionId: string, database: string, sql = '', opts: { run?: boolean } = {}): QueryTab {
    const n = (this.queryCount[sessionId] = (this.queryCount[sessionId] ?? 0) + 1)
    const tab: QueryTab = { id: `q${this.seq++}`, kind: 'query', sessionId, database, sql, title: `Query ${n}`, runOnOpen: opts.run || undefined }
    this.list.push(tab)
    this.select(tab.id)
    return tab
  }

  close(id: string) {
    const i = this.list.findIndex((t) => t.id === id)
    if (i < 0) return
    const tab = this.list[i]
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
    delete this.activeBySession[sessionId]
    delete this.queryCount[sessionId]
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
