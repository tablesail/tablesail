// Lazy paging of a table: fixed-size pages, an LRU cache and de-duplicated requests.

import type { Page, Row } from '../types'

export interface PagerOptions {
  pageSize: number
  maxPages: number
  load: (offset: number, limit: number) => Promise<Page>
}

/** Failed pages are not retried for this long (ms), so a broken query isn't hammered while scrolling. */
const RETRY_AFTER_MS = 3000

export class Pager {
  readonly pageSize: number
  private readonly maxPages: number
  private readonly loadPage: PagerOptions['load']
  private pages = new Map<number, Row[]>()
  private inflight = new Map<number, Promise<void>>()
  private failed = new Map<number, number>()
  private lastTouched = -1
  private protectLo = 0
  private protectHi = 0

  onPage?: (page: Page, pageIndex: number) => void
  onError?: (error: unknown, pageIndex: number) => void

  constructor(opts: PagerOptions) {
    this.pageSize = opts.pageSize
    this.maxPages = opts.maxPages
    this.loadPage = opts.load
  }

  get loadedPages(): number {
    return this.pages.size
  }

  get busy(): boolean {
    return this.inflight.size > 0
  }

  /** The row at `index`, or undefined while its page is not loaded. */
  get(index: number): Row | undefined {
    const p = Math.floor(index / this.pageSize)
    const rows = this.pages.get(p)
    if (!rows) return undefined
    if (p !== this.lastTouched) {
      this.pages.delete(p)
      this.pages.set(p, rows)
      this.lastTouched = p
    }
    return rows[index - p * this.pageSize]
  }

  has(index: number): boolean {
    return this.pages.has(Math.floor(index / this.pageSize))
  }

  /** True when every page covering [start, end) is loaded or on its way. */
  covers(start: number, end: number): boolean {
    const lo = Math.floor(Math.max(0, start) / this.pageSize)
    const hi = Math.floor(Math.max(0, end - 1) / this.pageSize)
    for (let p = lo; p <= hi; p++) {
      if (!this.pages.has(p) && !this.inflight.has(p)) return false
    }
    return true
  }

  /** Loads the pages covering [start, end) (end exclusive). Resolves when all of them settled. */
  ensure(start: number, end: number): Promise<void> {
    const lo = Math.floor(Math.max(0, start) / this.pageSize)
    const hi = Math.floor(Math.max(0, end - 1) / this.pageSize)
    this.protectLo = lo - 1
    this.protectHi = hi + 1
    const waits: Promise<void>[] = []
    const now = Date.now()
    for (let p = lo; p <= hi; p++) {
      if (this.pages.has(p)) continue
      const running = this.inflight.get(p)
      if (running) {
        waits.push(running)
        continue
      }
      const failedAt = this.failed.get(p)
      if (failedAt !== undefined && now - failedAt < RETRY_AFTER_MS) continue
      waits.push(this.fetch(p))
    }
    return Promise.all(waits).then(() => undefined)
  }

  /** Forget a failed page so the next ensure() retries immediately. */
  clearFailures() {
    this.failed.clear()
  }

  private fetch(p: number): Promise<void> {
    const job = this.loadPage(p * this.pageSize, this.pageSize).then(
      (page) => {
        this.inflight.delete(p)
        this.failed.delete(p)
        this.pages.set(p, page.rows)
        // The new page is now the newest; a page read before it is no longer last in the map.
        this.lastTouched = -1
        this.evict()
        this.onPage?.(page, p)
      },
      (error) => {
        this.inflight.delete(p)
        this.failed.set(p, Date.now())
        this.onError?.(error, p)
      },
    )
    this.inflight.set(p, job)
    return job
  }

  private evict() {
    if (this.pages.size <= this.maxPages) return
    for (const key of this.pages.keys()) {
      if (this.pages.size <= this.maxPages) break
      if (key >= this.protectLo && key <= this.protectHi) continue
      this.pages.delete(key)
      if (key === this.lastTouched) this.lastTouched = -1
    }
  }
}
