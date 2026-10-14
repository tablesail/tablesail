import { api, errorMessage } from '../api'
import type { ColumnMeta, QueryStatus, Row, StatementSpan } from '../types'

export const PAGE = 500
const POLL_MS = 60

interface SetCache {
  rows: (Row | undefined)[]
  done: Set<number>
  pending: Set<number>
  loaded: number
  complete: boolean
  truncated: boolean
  columns: ColumnMeta[]
  failed: string | null
}

const key = (stmt: number, set: number) => `${stmt}:${set}`

/**
 * One query tab's execution state: starts a run, polls its status, and caches result rows
 * in pages of PAGE as the grid asks for them. Rows live in plain arrays (not reactive);
 * `revision` is bumped whenever any of them change.
 */
export class QueryRunner {
  running = $state(false)
  queryId = $state<string | null>(null)
  status = $state.raw<QueryStatus | null>(null)
  statements = $state.raw<StatementSpan[]>([])
  runError = $state<string | null>(null)
  elapsedMs = $state(0)
  revision = $state(0)
  /** increments for every run, so the UI can reset per-run selections */
  runId = $state(0)

  private caches = new Map<string, SetCache>()
  private token = 0
  private startedAt = 0

  /** txId: run inside that manual transaction instead of on the connection pool. */
  async run(sessionId: string, database: string | null, sql: string, txId: string | null = null) {
    await this.close()
    const token = ++this.token
    this.runId++
    this.running = true
    this.runError = null
    this.elapsedMs = 0
    this.startedAt = performance.now()
    let queryId: string
    try {
      const run = await api.queryRun(sessionId, database, sql, txId)
      queryId = run.queryId
      if (token !== this.token) {
        void api.queryClose(queryId).catch(() => {})
        return
      }
      this.queryId = queryId
      this.statements = run.statements
    } catch (e) {
      if (token === this.token) {
        this.runError = errorMessage(e)
        this.running = false
        this.elapsedMs = performance.now() - this.startedAt
      }
      return
    }

    try {
      while (token === this.token) {
        const st = await api.queryStatus(queryId)
        if (token !== this.token) return
        this.apply(st)
        this.elapsedMs = performance.now() - this.startedAt
        if (st.finished) break
        await new Promise((r) => setTimeout(r, POLL_MS))
      }
    } catch (e) {
      if (token === this.token) this.runError = errorMessage(e)
    }
    if (token === this.token) {
      this.running = false
      this.elapsedMs = performance.now() - this.startedAt
    }
  }

  async cancel() {
    if (this.queryId && this.running) {
      try {
        await api.queryCancel(this.queryId)
      } catch (e) {
        this.runError = errorMessage(e)
      }
    }
  }

  /** Frees the backend query and drops cached rows. Safe to call repeatedly. */
  async close() {
    this.token++
    const id = this.queryId
    this.queryId = null
    this.status = null
    this.statements = []
    this.caches.clear()
    this.running = false
    this.revision++
    if (id) {
      try {
        await api.queryClose(id)
      } catch {
        // already gone
      }
    }
  }

  private cache(stmt: number, set: number): SetCache {
    const k = key(stmt, set)
    let c = this.caches.get(k)
    if (!c) {
      c = {
        rows: [],
        done: new Set(),
        pending: new Set(),
        loaded: 0,
        complete: false,
        truncated: false,
        columns: [],
        failed: null,
      }
      this.caches.set(k, c)
    }
    return c
  }

  private apply(st: QueryStatus) {
    this.status = st
    for (const s of st.statements) {
      for (const set of s.sets) {
        const c = this.cache(s.index, set.setIndex)
        const fresh = c.columns.length === 0
        c.columns = set.columns
        c.loaded = Math.max(c.loaded, set.loaded)
        c.complete = c.complete || set.complete
        c.truncated = set.truncated
        if (fresh) this.needRows(s.index, set.setIndex, 0, PAGE)
      }
    }
    this.revision++
  }

  info(stmt: number, set: number) {
    void this.revision
    const c = this.caches.get(key(stmt, set))
    return {
      columns: c?.columns ?? [],
      loaded: c?.loaded ?? 0,
      complete: c?.complete ?? false,
      truncated: c?.truncated ?? false,
      failed: c?.failed ?? null,
    }
  }

  /** Rows the grid should size its scroll area to: one extra page while more may follow. */
  rowCount(stmt: number, set: number): number {
    const i = this.info(stmt, set)
    return i.complete ? i.loaded : i.loaded + PAGE
  }

  getRow = (stmt: number, set: number, index: number): Row | undefined => {
    return this.caches.get(key(stmt, set))?.rows[index]
  }

  needRows(stmt: number, set: number, start: number, end: number) {
    const c = this.cache(stmt, set)
    const last = Math.max(start, end - 1)
    for (let p = Math.floor(start / PAGE); p <= Math.floor(last / PAGE); p++) {
      if (!c.done.has(p) && !c.pending.has(p)) void this.fetchPage(stmt, set, p, c)
    }
  }

  private async fetchPage(stmt: number, set: number, page: number, c: SetCache) {
    const id = this.queryId
    if (!id) return
    const token = this.token
    c.pending.add(page)
    try {
      const r = await api.queryFetch(id, stmt, set, page * PAGE, PAGE)
      if (token !== this.token) return
      for (let i = 0; i < r.rows.length; i++) c.rows[r.offset + i] = r.rows[i]
      c.loaded = Math.max(c.loaded, r.loaded, r.offset + r.rows.length)
      if (r.complete) c.complete = true
      if (r.rows.length >= PAGE || r.complete) c.done.add(page)
      c.failed = null
    } catch (e) {
      if (token !== this.token) return
      c.failed = errorMessage(e)
      c.done.add(page)
    } finally {
      c.pending.delete(page)
      if (token === this.token) this.revision++
    }
  }
}
