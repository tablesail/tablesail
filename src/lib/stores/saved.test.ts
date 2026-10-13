import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'
import type { SavedQuery } from './saved.svelte'

const KEY = 'tablesoft.savedQueries'

async function fresh(initial: Record<string, unknown> = {}) {
  vi.resetModules()
  const storage = installStorage(initial)
  const { saved } = await import('./saved.svelte')
  return { saved, storage }
}

const q = (over: Partial<Omit<SavedQuery, 'id' | 'savedAt'>> = {}) => ({
  name: 'Active users',
  sql: 'SELECT * FROM users WHERE active',
  connectionId: 'c1' as string | null,
  database: 'app' as string | null,
  ...over,
})

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2025-03-01T10:00:00Z'))
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('saved queries', () => {
  it('starts empty', async () => {
    const { saved } = await fresh()
    expect(saved.list).toEqual([])
  })

  it('adds a query and returns it with an id and a timestamp', async () => {
    const { saved } = await fresh()
    const item = saved.add(q())
    expect(item).toMatchObject({ ...q(), savedAt: Date.now() })
    expect(item.id).toMatch(/^s[0-9a-z]+$/)
    expect(saved.list).toHaveLength(1)
    expect(saved.list[0]).toMatchObject({ id: item.id, name: 'Active users' })
  })

  it('puts the newest first', async () => {
    const { saved } = await fresh()
    saved.add(q({ name: 'a' }))
    vi.advanceTimersByTime(5)
    saved.add(q({ name: 'b' }))
    expect(saved.list.map((x) => x.name)).toEqual(['b', 'a'])
  })

  it('gives unique ids even when saved in the same millisecond (regression)', async () => {
    const { saved } = await fresh()
    const ids = Array.from({ length: 20 }, (_, i) => saved.add(q({ name: `n${i}` })).id)
    expect(new Set(ids).size).toBe(20)
  })

  it('renaming one of two same-millisecond saves does not touch the other (regression)', async () => {
    const { saved } = await fresh()
    const a = saved.add(q({ name: 'a' }))
    const b = saved.add(q({ name: 'b' }))
    saved.rename(a.id, 'renamed')
    expect(saved.list.find((x) => x.id === a.id)?.name).toBe('renamed')
    expect(saved.list.find((x) => x.id === b.id)?.name).toBe('b')
  })

  it('allows queries not tied to a connection', async () => {
    const { saved } = await fresh()
    const item = saved.add(q({ connectionId: null, database: null }))
    expect(item.connectionId).toBeNull()
    expect(item.database).toBeNull()
  })

  it('renames by id', async () => {
    const { saved } = await fresh()
    const a = saved.add(q({ name: 'a' }))
    saved.add(q({ name: 'b' }))
    saved.rename(a.id, 'alpha')
    expect(saved.list.map((x) => x.name).sort()).toEqual(['alpha', 'b'])
    expect(saved.list.find((x) => x.id === a.id)?.sql).toBe(q().sql)
  })

  it('renaming an unknown id changes nothing', async () => {
    const { saved } = await fresh()
    saved.add(q({ name: 'a' }))
    saved.rename('nope', 'x')
    expect(saved.list.map((x) => x.name)).toEqual(['a'])
  })

  it('removes by id', async () => {
    const { saved } = await fresh()
    const a = saved.add(q({ name: 'a' }))
    const b = saved.add(q({ name: 'b' }))
    saved.remove(a.id)
    expect(saved.list.map((x) => x.id)).toEqual([b.id])
  })

  it('removing an unknown id changes nothing', async () => {
    const { saved } = await fresh()
    saved.add(q())
    saved.remove('nope')
    expect(saved.list).toHaveLength(1)
  })
})

describe('saved queries persistence', () => {
  it('writes on add, rename and remove', async () => {
    const { saved, storage } = await fresh()
    const a = saved.add(q({ name: 'a' }))
    expect(storage.json<SavedQuery[]>(KEY)?.map((x) => x.name)).toEqual(['a'])
    saved.rename(a.id, 'z')
    expect(storage.json<SavedQuery[]>(KEY)?.map((x) => x.name)).toEqual(['z'])
    saved.remove(a.id)
    expect(storage.json<SavedQuery[]>(KEY)).toEqual([])
  })

  it('loads saved queries at startup', async () => {
    const stored: SavedQuery[] = [{ id: 's1', name: 'Q', sql: 'SELECT 1', connectionId: null, database: null, savedAt: 1 }]
    const { saved } = await fresh({ [KEY]: stored })
    expect(saved.list).toEqual(stored)
  })

  it('survives an app restart', async () => {
    const { saved, storage } = await fresh()
    saved.add(q({ name: 'keep me' }))
    vi.resetModules()
    vi.stubGlobal('localStorage', storage)
    const again = (await import('./saved.svelte')).saved
    expect(again.list.map((x) => x.name)).toEqual(['keep me'])
  })

  it('starts empty with corrupt data', async () => {
    const { saved } = await fresh({ [KEY]: 'not json' })
    expect(saved.list).toEqual([])
  })
})
