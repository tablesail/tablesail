import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Page, Row } from '../types'
import { Pager } from './pager'

const PAGE = 10

/** A row's single cell holds its absolute index, so tests can tell which page they got. */
function makePage(offset: number, limit: number, total = Infinity): Page {
  const n = Math.max(0, Math.min(limit, total - offset))
  const rows: Row[] = Array.from({ length: n }, (_, i) => [offset + i])
  return { columns: [], rows, offset, query: 'q', elapsedMs: 1 }
}

function setup(opts: { maxPages?: number; total?: number } = {}) {
  const load = vi.fn((offset: number, limit: number) => Promise.resolve(makePage(offset, limit, opts.total)))
  const pager = new Pager({ pageSize: PAGE, maxPages: opts.maxPages ?? 100, load })
  return { pager, load }
}

/** Loader whose promises are resolved/rejected by the test. */
function manual() {
  const calls: { offset: number; limit: number; resolve: (p: Page) => void; reject: (e: unknown) => void }[] = []
  const load = vi.fn(
    (offset: number, limit: number) =>
      new Promise<Page>((resolve, reject) => {
        calls.push({ offset, limit, resolve, reject })
      }),
  )
  return { load, calls }
}

describe('Pager: basic loading', () => {
  it('starts empty', () => {
    const { pager } = setup()
    expect(pager.loadedPages).toBe(0)
    expect(pager.busy).toBe(false)
    expect(pager.get(0)).toBeUndefined()
    expect(pager.has(0)).toBe(false)
    expect(pager.pageSize).toBe(PAGE)
  })

  it('loads the page covering a row and serves its rows', async () => {
    const { pager, load } = setup()
    await pager.ensure(0, 5)
    expect(load).toHaveBeenCalledTimes(1)
    expect(load).toHaveBeenCalledWith(0, PAGE)
    expect(pager.get(0)).toEqual([0])
    expect(pager.get(9)).toEqual([9])
    expect(pager.has(9)).toBe(true)
    expect(pager.get(10)).toBeUndefined()
    expect(pager.loadedPages).toBe(1)
  })

  it('loads every page in a range, once each, at page-aligned offsets', async () => {
    const { pager, load } = setup()
    await pager.ensure(5, 35) // rows 5..34 -> pages 0..3
    expect(load.mock.calls).toEqual([[0, PAGE], [10, PAGE], [20, PAGE], [30, PAGE]])
    expect(pager.get(34)).toEqual([34])
  })

  it('the end of the range is exclusive', async () => {
    const { pager, load } = setup()
    await pager.ensure(0, 10) // rows 0..9 -> page 0 only
    expect(load).toHaveBeenCalledTimes(1)
    await pager.ensure(10, 20)
    expect(load).toHaveBeenCalledTimes(2)
    expect(load).toHaveBeenLastCalledWith(10, PAGE)
  })

  it('does not reload pages it already has', async () => {
    const { pager, load } = setup()
    await pager.ensure(0, 30)
    load.mockClear()
    await pager.ensure(0, 30)
    await pager.ensure(12, 18)
    expect(load).not.toHaveBeenCalled()
  })

  it('only fetches the missing pages of a range', async () => {
    const { pager, load } = setup()
    await pager.ensure(10, 20)
    load.mockClear()
    await pager.ensure(0, 30)
    expect(load.mock.calls.map((c) => c[0])).toEqual([0, 20])
  })

  it('negative starts are clamped to the first page', async () => {
    const { pager, load } = setup()
    await pager.ensure(-50, 5)
    expect(load).toHaveBeenCalledTimes(1)
    expect(load).toHaveBeenCalledWith(0, PAGE)
  })

  it('an empty range past page 0 loads nothing', async () => {
    const { pager, load } = setup()
    await pager.ensure(50, 50)
    expect(load).not.toHaveBeenCalled()
  })

  it('a short final page is served up to its last row', async () => {
    const { pager } = setup({ total: 25 })
    await pager.ensure(20, 30)
    expect(pager.get(24)).toEqual([24])
    expect(pager.get(25)).toBeUndefined()
  })

  it('row indices beyond the loaded rows of a page are undefined, not wrong rows', async () => {
    const { pager } = setup({ total: 3 })
    await pager.ensure(0, 10)
    expect(pager.get(2)).toEqual([2])
    expect(pager.get(3)).toBeUndefined()
  })

  it('has() is page-granular', async () => {
    const { pager } = setup()
    await pager.ensure(10, 11)
    expect(pager.has(10)).toBe(true)
    expect(pager.has(19)).toBe(true)
    expect(pager.has(9)).toBe(false)
    expect(pager.has(20)).toBe(false)
  })
})

describe('Pager: callbacks', () => {
  it('onPage receives the page and its index', async () => {
    const { pager } = setup()
    const seen: [number, number][] = []
    pager.onPage = (page, idx) => seen.push([idx, page.rows.length])
    await pager.ensure(0, 25)
    expect(seen.sort((a, b) => a[0] - b[0])).toEqual([[0, 10], [1, 10], [2, 10]])
  })

  it('the page is already readable inside onPage', async () => {
    const { pager } = setup()
    let readable: unknown
    pager.onPage = (_p, idx) => {
      readable = pager.get(idx * PAGE)
    }
    await pager.ensure(0, 5)
    expect(readable).toEqual([0])
  })
})

describe('Pager: de-duplicated requests', () => {
  it('concurrent ensure() calls share one request per page', async () => {
    const { load, calls } = manual()
    const pager = new Pager({ pageSize: PAGE, maxPages: 10, load })
    const a = pager.ensure(0, 10)
    const b = pager.ensure(0, 10)
    expect(load).toHaveBeenCalledTimes(1)
    expect(pager.busy).toBe(true)
    calls[0].resolve(makePage(0, PAGE))
    await Promise.all([a, b])
    expect(pager.busy).toBe(false)
    expect(pager.get(3)).toEqual([3])
  })

  it('an overlapping range only requests the pages not already in flight', async () => {
    const { load, calls } = manual()
    const pager = new Pager({ pageSize: PAGE, maxPages: 10, load })
    const a = pager.ensure(0, 20) // pages 0,1
    const b = pager.ensure(10, 30) // pages 1,2 -> only 2 is new
    expect(load.mock.calls.map((c) => c[0])).toEqual([0, 10, 20])
    for (const c of calls) c.resolve(makePage(c.offset, c.limit))
    await Promise.all([a, b])
    expect(pager.loadedPages).toBe(3)
  })

  it('the second caller waits for the in-flight page', async () => {
    const { load, calls } = manual()
    const pager = new Pager({ pageSize: PAGE, maxPages: 10, load })
    void pager.ensure(0, 10)
    let done = false
    const second = pager.ensure(0, 10).then(() => (done = true))
    await Promise.resolve()
    expect(done).toBe(false)
    calls[0].resolve(makePage(0, PAGE))
    await second
    expect(done).toBe(true)
  })

  it('covers() counts loaded and in-flight pages', async () => {
    const { load, calls } = manual()
    const pager = new Pager({ pageSize: PAGE, maxPages: 10, load })
    expect(pager.covers(0, 10)).toBe(false)
    const p = pager.ensure(0, 10)
    expect(pager.covers(0, 10)).toBe(true) // in flight
    expect(pager.covers(0, 11)).toBe(false) // page 1 is neither
    calls[0].resolve(makePage(0, PAGE))
    await p
    expect(pager.covers(0, 10)).toBe(true) // loaded
    expect(pager.covers(5, 8)).toBe(true)
    expect(pager.covers(0, 20)).toBe(false)
  })

  it('covers() clamps a negative start', async () => {
    const { pager } = setup()
    await pager.ensure(0, 10)
    expect(pager.covers(-100, 5)).toBe(true)
  })
})

describe('Pager: LRU eviction', () => {
  it('keeps at most maxPages once the viewport has moved on', async () => {
    const { pager } = setup({ maxPages: 3 })
    for (let p = 0; p < 10; p++) await pager.ensure(p * PAGE, (p + 1) * PAGE)
    expect(pager.loadedPages).toBeLessThanOrEqual(3)
  })

  it('never evicts the pages around the current viewport', async () => {
    const { pager } = setup({ maxPages: 3 })
    for (let p = 0; p < 6; p++) await pager.ensure(p * PAGE, (p + 1) * PAGE)
    // viewing page 5: pages 4..6 are protected, so 5 and 4 must still be there
    expect(pager.has(5 * PAGE)).toBe(true)
    expect(pager.has(4 * PAGE)).toBe(true)
  })

  it('evicts the oldest pages first', async () => {
    const { pager } = setup({ maxPages: 3 })
    for (let p = 0; p < 6; p++) await pager.ensure(p * PAGE, (p + 1) * PAGE)
    expect(pager.has(0)).toBe(false)
    expect(pager.has(1 * PAGE)).toBe(false)
  })

  it('reading a page makes it recently used', async () => {
    const { pager } = setup({ maxPages: 3 })
    await pager.ensure(0, 10)
    await pager.ensure(10, 20)
    await pager.ensure(20, 30)
    pager.get(0) // touch page 0: now page 1 is the oldest
    await pager.ensure(60, 70) // viewport far away so nothing nearby is protected
    expect(pager.loadedPages).toBe(3)
    expect(pager.has(0)).toBe(true)
    expect(pager.has(10)).toBe(false)
  })

  it('a page read after a newer page arrived is newer than it (regression: lastTouched shortcut)', async () => {
    const { pager } = setup({ maxPages: 2 })
    await pager.ensure(0, 10)
    pager.get(0) // page 0 is now the most recently used
    await pager.ensure(60, 70) // page 6 arrives later; order is [0, 6]
    pager.get(0) // page 0 is read again, after page 6 arrived: order must become [6, 0]
    await pager.ensure(100, 110) // page 10 arrives; the oldest unprotected page goes
    expect(pager.has(0)).toBe(true)
    expect(pager.has(60)).toBe(false)
  })

  it('an evicted page is fetched again when asked for', async () => {
    const { pager, load } = setup({ maxPages: 2 })
    for (let p = 0; p < 5; p++) await pager.ensure(p * PAGE, (p + 1) * PAGE)
    expect(pager.has(0)).toBe(false)
    load.mockClear()
    await pager.ensure(0, 10)
    expect(load).toHaveBeenCalledWith(0, PAGE)
    expect(pager.get(0)).toEqual([0])
  })

  it('may exceed maxPages while every page is inside the protected viewport window', async () => {
    const { pager } = setup({ maxPages: 2 })
    await pager.ensure(0, 50) // 5 pages visible at once
    expect(pager.loadedPages).toBe(5)
  })

  it('stays correct after evicting the most recently touched page', async () => {
    const { pager } = setup({ maxPages: 2 })
    await pager.ensure(0, 10)
    pager.get(0)
    for (let p = 5; p < 9; p++) await pager.ensure(p * PAGE, (p + 1) * PAGE)
    expect(pager.has(0)).toBe(false)
    expect(pager.get(0)).toBeUndefined()
    await pager.ensure(0, 10)
    expect(pager.get(0)).toEqual([0])
  })
})

describe('Pager: failures', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2025-01-01T00:00:00Z'))
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  function flaky(failures: number) {
    let n = 0
    const load = vi.fn(async (offset: number, limit: number) => {
      if (n++ < failures) throw new Error(`boom ${n}`)
      return makePage(offset, limit)
    })
    return { load, pager: new Pager({ pageSize: PAGE, maxPages: 10, load }) }
  }

  it('reports the error with the page index and still resolves ensure()', async () => {
    const { pager } = flaky(1)
    const errors: [unknown, number][] = []
    pager.onError = (e, p) => errors.push([e, p])
    await expect(pager.ensure(20, 30)).resolves.toBeUndefined()
    expect(errors).toHaveLength(1)
    expect((errors[0][0] as Error).message).toBe('boom 1')
    expect(errors[0][1]).toBe(2)
    expect(pager.has(20)).toBe(false)
    expect(pager.busy).toBe(false)
  })

  it('does not retry a failed page for 3 seconds', async () => {
    const { pager, load } = flaky(1)
    await pager.ensure(0, 10)
    expect(load).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(2999)
    await pager.ensure(0, 10)
    expect(load).toHaveBeenCalledTimes(1)
  })

  it('retries after 3 seconds', async () => {
    const { pager, load } = flaky(1)
    await pager.ensure(0, 10)
    vi.advanceTimersByTime(3000)
    await pager.ensure(0, 10)
    expect(load).toHaveBeenCalledTimes(2)
    expect(pager.get(0)).toEqual([0])
  })

  it('clearFailures() allows an immediate retry', async () => {
    const { pager, load } = flaky(1)
    await pager.ensure(0, 10)
    pager.clearFailures()
    await pager.ensure(0, 10)
    expect(load).toHaveBeenCalledTimes(2)
    expect(pager.has(0)).toBe(true)
  })

  it('a failed page does not block its healthy neighbours', async () => {
    const load = vi.fn(async (offset: number, limit: number) => {
      if (offset === 10) throw new Error('bad page')
      return makePage(offset, limit)
    })
    const pager = new Pager({ pageSize: PAGE, maxPages: 10, load })
    await pager.ensure(0, 30)
    expect(pager.has(0)).toBe(true)
    expect(pager.has(10)).toBe(false)
    expect(pager.has(20)).toBe(true)
  })

  it('a successful load clears the failure record', async () => {
    const { pager, load } = flaky(1)
    await pager.ensure(0, 10) // fails
    vi.advanceTimersByTime(3000)
    await pager.ensure(0, 10) // succeeds
    pager.get(0)
    load.mockClear()
    await pager.ensure(0, 10)
    expect(load).not.toHaveBeenCalled()
  })

  it('works without error/page callbacks installed', async () => {
    const { pager } = flaky(1)
    await expect(pager.ensure(0, 10)).resolves.toBeUndefined()
  })

  it('a rejected load is not left in flight (a retry after the cool-down can run)', async () => {
    const { pager } = flaky(1)
    await pager.ensure(0, 10)
    expect(pager.busy).toBe(false)
    expect(pager.covers(0, 10)).toBe(false)
  })
})
