import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryStorage, installStorage } from '../../test/helpers'
import type { HistoryEntry } from './history.svelte'

const KEY = 'tablesoft.history'

async function fresh(initial: Record<string, unknown> = {}) {
  vi.resetModules()
  const storage = installStorage(initial)
  const { history } = await import('./history.svelte')
  return { history, storage }
}

const entry = (over: Partial<Omit<HistoryEntry, 'id' | 'at'>> = {}) => ({
  sql: 'SELECT 1',
  connectionId: 'c1',
  connectionName: 'Local',
  database: 'app',
  elapsedMs: 12,
  ok: true,
  ...over,
})

/** `n` stored entries, newest first, named old0 (newest) .. old<n-1> (oldest). */
function seeded(n: number): HistoryEntry[] {
  return Array.from({ length: n }, (_, i) => ({
    id: n - i,
    sql: `old${i}`,
    connectionId: 'c1',
    connectionName: 'Local',
    database: 'app',
    at: n - i,
    elapsedMs: 1,
    ok: true,
  }))
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2025-03-01T10:00:00Z'))
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('history', () => {
  it('starts empty without stored data', async () => {
    const { history } = await fresh()
    expect(history.entries).toEqual([])
  })

  it('adds an entry with an id and a timestamp', async () => {
    const { history } = await fresh()
    history.add(entry())
    expect(history.entries).toHaveLength(1)
    const [e] = history.entries
    expect(e.sql).toBe('SELECT 1')
    expect(e.at).toBe(Date.now())
    expect(typeof e.id).toBe('number')
    expect(e).toMatchObject({ connectionId: 'c1', connectionName: 'Local', database: 'app', elapsedMs: 12, ok: true })
  })

  it('trims the SQL', async () => {
    const { history } = await fresh()
    history.add(entry({ sql: '  \n SELECT 2;  \n' }))
    expect(history.entries[0].sql).toBe('SELECT 2;')
  })

  it('ignores empty and whitespace-only SQL', async () => {
    const { history, storage } = await fresh()
    history.add(entry({ sql: '' }))
    history.add(entry({ sql: '   \n\t ' }))
    expect(history.entries).toEqual([])
    expect(storage.getItem(KEY)).toBeNull()
  })

  it('keeps the newest entry first', async () => {
    const { history } = await fresh()
    history.add(entry({ sql: 'first' }))
    vi.advanceTimersByTime(1000)
    history.add(entry({ sql: 'second' }))
    vi.advanceTimersByTime(1000)
    history.add(entry({ sql: 'third' }))
    expect(history.entries.map((e) => e.sql)).toEqual(['third', 'second', 'first'])
    expect(history.entries[0].at).toBeGreaterThan(history.entries[2].at)
  })

  it('gives distinct ids to entries added in the same millisecond (regression: Date.now() + Math.random() collided)', async () => {
    const { history } = await fresh()
    for (let i = 0; i < 300; i++) history.add(entry({ sql: `q${i}` }))
    expect(new Set(history.entries.map((e) => e.id)).size).toBe(history.entries.length)
  })

  it('remove() by id only removes that one entry even for same-millisecond entries', async () => {
    const { history } = await fresh()
    for (let i = 0; i < 20; i++) history.add(entry({ sql: `q${i}` }))
    history.remove(history.entries[7].id)
    expect(history.entries).toHaveLength(19)
  })

  it('ids keep increasing when the clock does not move or steps back', async () => {
    const { history } = await fresh()
    history.add(entry({ sql: 'a' }))
    vi.setSystemTime(new Date('2025-03-01T09:00:00Z')) // an hour earlier
    history.add(entry({ sql: 'b' }))
    expect(history.entries[0].id).toBeGreaterThan(history.entries[1].id)
  })

  it('records failures and missing timings', async () => {
    const { history } = await fresh()
    history.add(entry({ ok: false, elapsedMs: null }))
    expect(history.entries[0]).toMatchObject({ ok: false, elapsedMs: null })
  })

  it('keeps duplicates (every run is an entry)', async () => {
    const { history } = await fresh()
    history.add(entry())
    history.add(entry())
    expect(history.entries).toHaveLength(2)
  })

  it('caps the list at 500, dropping the oldest', async () => {
    const { history } = await fresh({ [KEY]: seeded(499) })
    history.add(entry({ sql: 'newest' }))
    expect(history.entries).toHaveLength(500)
    history.add(entry({ sql: 'newer still' }))
    expect(history.entries).toHaveLength(500)
    expect(history.entries[0].sql).toBe('newer still')
    expect(history.entries[1].sql).toBe('newest')
    expect(history.entries[499].sql).toBe('old497') // old498, the oldest, fell off the end
  })

  it('a first-time overflow of many entries still ends at the cap', async () => {
    const { history } = await fresh()
    for (let i = 0; i < 40; i++) history.add(entry({ sql: `q${i}` }))
    expect(history.entries).toHaveLength(40)
  })

  it('removes one entry by id', async () => {
    const { history } = await fresh()
    history.add(entry({ sql: 'a' }))
    history.add(entry({ sql: 'b' }))
    history.add(entry({ sql: 'c' }))
    const middle = history.entries[1]
    history.remove(middle.id)
    expect(history.entries.map((e) => e.sql)).toEqual(['c', 'a'])
  })

  it('removing an unknown id changes nothing', async () => {
    const { history } = await fresh()
    history.add(entry())
    history.remove(-1)
    expect(history.entries).toHaveLength(1)
  })

  it('clear empties the list', async () => {
    const { history } = await fresh()
    history.add(entry())
    history.add(entry())
    history.clear()
    expect(history.entries).toEqual([])
  })
})

describe('history persistence', () => {
  it('writes to localStorage on add, remove and clear', async () => {
    const { history, storage } = await fresh()
    history.add(entry({ sql: 'a' }))
    expect(storage.json<HistoryEntry[]>(KEY)?.map((e) => e.sql)).toEqual(['a'])
    history.add(entry({ sql: 'b' }))
    expect(storage.json<HistoryEntry[]>(KEY)?.map((e) => e.sql)).toEqual(['b', 'a'])
    history.remove(history.entries[0].id)
    expect(storage.json<HistoryEntry[]>(KEY)?.map((e) => e.sql)).toEqual(['a'])
    history.clear()
    expect(storage.json<HistoryEntry[]>(KEY)).toEqual([])
  })

  it('stores the capped list, not the overflow', async () => {
    const { history, storage } = await fresh({ [KEY]: seeded(500) })
    history.add(entry({ sql: 'one more' }))
    const stored = storage.json<HistoryEntry[]>(KEY)!
    expect(stored).toHaveLength(500)
    expect(stored[0].sql).toBe('one more')
  })

  it('loads previously saved entries at startup', async () => {
    const saved: HistoryEntry[] = [
      { id: 2, sql: 'newer', connectionId: 'c', connectionName: 'C', database: 'd', at: 200, elapsedMs: 1, ok: true },
      { id: 1, sql: 'older', connectionId: 'c', connectionName: 'C', database: 'd', at: 100, elapsedMs: null, ok: false },
    ]
    const { history } = await fresh({ [KEY]: saved })
    expect(history.entries.map((e) => e.sql)).toEqual(['newer', 'older'])
  })

  it('a second app start sees what the first one saved', async () => {
    const { history, storage } = await fresh()
    history.add(entry({ sql: 'persisted' }))
    vi.resetModules()
    vi.stubGlobal('localStorage', storage as MemoryStorage)
    const again = (await import('./history.svelte')).history
    expect(again.entries.map((e) => e.sql)).toEqual(['persisted'])
  })

  it('starts empty with corrupt stored data', async () => {
    const { history } = await fresh({ [KEY]: '{{{' })
    expect(history.entries).toEqual([])
    expect(() => history.add(entry())).not.toThrow()
  })

  it('works when storage throws on write', async () => {
    vi.resetModules()
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem() {
        throw new Error('full')
      },
    })
    const { history } = await import('./history.svelte')
    expect(() => history.add(entry())).not.toThrow()
    expect(history.entries).toHaveLength(1)
  })

  it('works without localStorage', async () => {
    vi.resetModules()
    vi.stubGlobal('localStorage', undefined)
    const { history } = await import('./history.svelte')
    history.add(entry())
    expect(history.entries).toHaveLength(1)
  })
})
