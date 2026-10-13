import type { Filter, TableRef } from '../types'

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
}

export type Tab = TableTab | QueryTab

function sameTable(a: TableRef, b: TableRef): boolean {
  return a.database === b.database && (a.schema ?? null) === (b.schema ?? null) && a.name === b.name
}

class TabsStore {
  list = $state<Tab[]>([])
  /** active tab id per session */
  activeBySession = $state<Record<string, string>>({})

  private seq = 1
  private queryCount: Record<string, number> = {}

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

  newQuery(sessionId: string, database: string, sql = ''): QueryTab {
    const n = (this.queryCount[sessionId] = (this.queryCount[sessionId] ?? 0) + 1)
    const tab: QueryTab = { id: `q${this.seq++}`, kind: 'query', sessionId, database, sql, title: `Query ${n}` }
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
