import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryStorage, installStorage } from '../../test/helpers'
import type { Filter, TableRef } from '../types'

/**
 * A fresh tabs store (and the svelte runtime it runs on) per call. `flush` runs pending
 * effects; the store's debounced save is driven with fake timers.
 */
async function fresh(initial: Record<string, unknown> = {}, storage?: MemoryStorage) {
  vi.resetModules()
  const store = storage ?? installStorage(initial)
  if (storage) vi.stubGlobal('localStorage', storage)
  const { flushSync } = await import('svelte')
  const { tabs } = await import('./tabs.svelte')
  /** Runs pending effects, then lets the 400 ms debounce elapse. */
  const settleSave = () => {
    flushSync()
    vi.advanceTimersByTime(400)
  }
  return { tabs, storage: store, flush: flushSync, settleSave }
}

const T = (name: string, schema: string | null = null, database = 'db'): TableRef => ({ database, schema, name })
const KEY = (conn: string) => `tablesoft.tabs.${conn}`

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('tabs: opening tables', () => {
  it('opens a table tab, selects it, and titles it after the table', async () => {
    const { tabs } = await fresh()
    const tab = tabs.openTable('s1', T('users', 'public'))
    expect(tab).toMatchObject({ kind: 'table', sessionId: 's1', title: 'users', table: { name: 'users', schema: 'public' } })
    expect(tab.filters).toBeUndefined()
    expect(tab.preview).toBeUndefined()
    expect(tabs.list).toHaveLength(1)
    expect(tabs.active('s1')?.id).toBe(tab.id)
  })

  it('copies the table ref (later mutation of the argument does not leak in)', async () => {
    const { tabs } = await fresh()
    const ref = T('users')
    const tab = tabs.openTable('s1', ref)
    ref.name = 'changed'
    expect(tab.table.name).toBe('users')
  })

  it('opening an already-open table selects the existing tab instead of adding one', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('users'))
    tabs.openTable('s1', T('orders'))
    expect(tabs.active('s1')?.title).toBe('orders')
    const again = tabs.openTable('s1', T('users'))
    expect(again.id).toBe(a.id)
    expect(tabs.list).toHaveLength(2)
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('treats a missing schema and a null schema as the same table', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', { database: 'db', name: 'users' })
    const b = tabs.openTable('s1', T('users', null))
    expect(b.id).toBe(a.id)
    expect(tabs.list).toHaveLength(1)
  })

  it('different schemas, databases or names are different tabs', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users', 'public'))
    tabs.openTable('s1', T('users', 'audit'))
    tabs.openTable('s1', T('users', 'public', 'other_db'))
    tabs.openTable('s1', T('Users', 'public'))
    expect(tabs.list).toHaveLength(4)
  })

  it('the same table in different sessions is not shared', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('users'))
    const b = tabs.openTable('s2', T('users'))
    expect(a.id).not.toBe(b.id)
    expect(tabs.active('s1')?.id).toBe(a.id)
    expect(tabs.active('s2')?.id).toBe(b.id)
  })

  it('ids are unique', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    tabs.newQuery('s1', 'db')
    expect(new Set(tabs.list.map((t) => t.id)).size).toBe(3)
  })
})

describe('tabs: filters', () => {
  const f: Filter[] = [{ column: 'user_id', op: 'eq', value: '7' }]

  it('a filtered open creates its own tab carrying the filters', async () => {
    const { tabs } = await fresh()
    const plain = tabs.openTable('s1', T('orders'))
    const filtered = tabs.openTable('s1', T('orders'), f)
    expect(filtered.id).not.toBe(plain.id)
    expect(filtered.filters).toEqual(f)
    expect(tabs.list).toHaveLength(2)
    expect(tabs.active('s1')?.id).toBe(filtered.id)
  })

  it('a filtered open never reuses an existing tab, even an identical filtered one', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('orders'), f)
    tabs.openTable('s1', T('orders'), f)
    expect(tabs.list).toHaveLength(2)
  })

  it('an unfiltered open reuses the unfiltered tab, not a filtered one', async () => {
    const { tabs } = await fresh()
    const filtered = tabs.openTable('s1', T('orders'), f)
    const plain = tabs.openTable('s1', T('orders'))
    expect(plain.id).not.toBe(filtered.id)
    const again = tabs.openTable('s1', T('orders'))
    expect(again.id).toBe(plain.id)
  })

  it('an empty filter list counts as no filters', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('orders'))
    const b = tabs.openTable('s1', T('orders'), [])
    expect(b.id).toBe(a.id)
    expect(tabs.list).toHaveLength(1)
    expect(a.filters).toBeUndefined()
  })
})

describe('tabs: preview tabs', () => {
  it('opens a preview tab flagged as such', async () => {
    const { tabs } = await fresh()
    const t = tabs.openTable('s1', T('users'), undefined, { preview: true })
    expect(t.preview).toBe(true)
    expect(tabs.active('s1')?.id).toBe(t.id)
  })

  it('the next preview replaces the previous one in place (same position, new id)', async () => {
    const { tabs } = await fresh()
    const keep = tabs.openTable('s1', T('keep'))
    const p1 = tabs.openTable('s1', T('a'), undefined, { preview: true })
    tabs.newQuery('s1', 'db')
    const p2 = tabs.openTable('s1', T('b'), undefined, { preview: true })
    expect(p2.id).not.toBe(p1.id)
    expect(tabs.list.map((t) => t.title)).toEqual(['keep', 'b', 'Query 1'])
    expect(tabs.list.some((t) => t.id === p1.id)).toBe(false)
    expect(tabs.list[0].id).toBe(keep.id)
    expect(tabs.active('s1')?.id).toBe(p2.id)
  })

  it('a preview does not replace regular tabs', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'), undefined, { preview: true })
    tabs.openTable('s1', T('c'), undefined, { preview: true })
    expect(tabs.list.map((t) => t.title)).toEqual(['a', 'c'])
  })

  it('a preview in another session is left alone', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'), undefined, { preview: true })
    tabs.openTable('s2', T('b'), undefined, { preview: true })
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['a'])
    expect(tabs.forSession('s2').map((t) => t.title)).toEqual(['b'])
  })

  it('at most one preview tab exists per session', async () => {
    const { tabs } = await fresh()
    for (const n of ['a', 'b', 'c', 'd']) tabs.openTable('s1', T(n), undefined, { preview: true })
    expect(tabs.forSession('s1').filter((t) => t.kind === 'table' && t.preview)).toHaveLength(1)
    expect(tabs.list).toHaveLength(1)
  })

  it('previewing a table that is already open just selects it, leaving its flag as it was', async () => {
    const { tabs } = await fresh()
    const regular = tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    const same = tabs.openTable('s1', T('a'), undefined, { preview: true })
    expect(same.id).toBe(regular.id)
    expect(same.preview).toBeFalsy()
    expect(tabs.list).toHaveLength(2)
    expect(tabs.active('s1')?.id).toBe(regular.id)
  })

  it('previewing the table that is currently the preview keeps it as the preview', async () => {
    const { tabs } = await fresh()
    const p = tabs.openTable('s1', T('a'), undefined, { preview: true })
    const again = tabs.openTable('s1', T('a'), undefined, { preview: true })
    expect(again.id).toBe(p.id)
    expect(again.preview).toBe(true)
  })

  it('opening an open preview table without preview (double-click) promotes it', async () => {
    const { tabs } = await fresh()
    const p = tabs.openTable('s1', T('a'), undefined, { preview: true })
    const kept = tabs.openTable('s1', T('a'))
    expect(kept.id).toBe(p.id)
    expect(kept.preview).toBe(false)
    // a later preview now adds a tab instead of replacing it
    tabs.openTable('s1', T('b'), undefined, { preview: true })
    expect(tabs.list.map((t) => t.title)).toEqual(['a', 'b'])
  })

  it('keep() turns a preview tab into a regular one', async () => {
    const { tabs } = await fresh()
    const p = tabs.openTable('s1', T('a'), undefined, { preview: true })
    tabs.keep(p.id)
    expect(tabs.list[0]).toMatchObject({ kind: 'table', preview: false })
    tabs.openTable('s1', T('b'), undefined, { preview: true })
    expect(tabs.list.map((t) => t.title)).toEqual(['a', 'b'])
  })

  it('keep() ignores unknown ids, regular tabs and query tabs', async () => {
    const { tabs } = await fresh()
    const regular = tabs.openTable('s1', T('a'))
    const q = tabs.newQuery('s1', 'db')
    expect(() => {
      tabs.keep('nope')
      tabs.keep(regular.id)
      tabs.keep(q.id)
    }).not.toThrow()
    expect(tabs.list).toHaveLength(2)
    expect(tabs.list[0]).not.toHaveProperty('preview', true)
  })

  it('a preview with filters is allowed and replaceable', async () => {
    const { tabs } = await fresh()
    const f: Filter[] = [{ column: 'id', op: 'eq', value: '1' }]
    const p = tabs.openTable('s1', T('a'), f, { preview: true })
    expect(p.filters).toEqual(f)
    expect(p.preview).toBe(true)
    tabs.openTable('s1', T('b'), undefined, { preview: true })
    expect(tabs.list.map((t) => t.title)).toEqual(['b'])
  })

  it('replacing the active preview moves the selection to the new one', async () => {
    const { tabs } = await fresh()
    const p1 = tabs.openTable('s1', T('a'), undefined, { preview: true })
    expect(tabs.activeBySession.s1).toBe(p1.id)
    const p2 = tabs.openTable('s1', T('b'), undefined, { preview: true })
    expect(tabs.activeBySession.s1).toBe(p2.id)
  })
})

describe('tabs: query tabs', () => {
  it('creates numbered query tabs per session', async () => {
    const { tabs } = await fresh()
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 1')
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 2')
    expect(tabs.newQuery('s2', 'db').title).toBe('Query 1')
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 3')
  })

  it('stores the database and starter SQL, and selects the tab', async () => {
    const { tabs } = await fresh()
    const q = tabs.newQuery('s1', 'analytics', 'SELECT 1;')
    expect(q).toMatchObject({ kind: 'query', sessionId: 's1', database: 'analytics', sql: 'SELECT 1;' })
    expect(tabs.active('s1')?.id).toBe(q.id)
  })

  it('SQL defaults to empty', async () => {
    const { tabs } = await fresh()
    expect(tabs.newQuery('s1', 'db').sql).toBe('')
  })

  it('numbering does not reuse a closed tab\'s number', async () => {
    const { tabs } = await fresh()
    const a = tabs.newQuery('s1', 'db')
    tabs.close(a.id)
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 2')
  })
})

describe('tabs: selecting and querying', () => {
  it('active() is null for no session or no tabs', async () => {
    const { tabs } = await fresh()
    expect(tabs.active(null)).toBeNull()
    expect(tabs.active('s1')).toBeNull()
  })

  it('select() switches the active tab within its session only', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    const other = tabs.openTable('s2', T('c'))
    tabs.select(a.id)
    expect(tabs.active('s1')?.id).toBe(a.id)
    expect(tabs.active('s2')?.id).toBe(other.id)
  })

  it('select() with an unknown id is ignored', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('a'))
    tabs.select('ghost')
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('forSession() lists a session\'s tabs in order', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.openTable('s2', T('x'))
    tabs.openTable('s1', T('b'))
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['a', 'b'])
    expect(tabs.forSession('nobody')).toEqual([])
  })
})

describe('tabs: closing', () => {
  async function three() {
    const ctx = await fresh()
    const a = ctx.tabs.openTable('s1', T('a'))
    const b = ctx.tabs.openTable('s1', T('b'))
    const c = ctx.tabs.openTable('s1', T('c'))
    return { ...ctx, a, b, c }
  }

  it('closing the active tab selects the tab to its right', async () => {
    const { tabs, a, b, c } = await three()
    tabs.select(b.id)
    tabs.close(b.id)
    expect(tabs.active('s1')?.id).toBe(c.id)
    expect(tabs.list.map((t) => t.id)).toEqual([a.id, c.id])
  })

  it('closing the last (active) tab selects the one to its left', async () => {
    const { tabs, b, c } = await three()
    tabs.close(c.id)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })

  it('closing the first active tab selects the next one', async () => {
    const { tabs, a, b } = await three()
    tabs.select(a.id)
    tabs.close(a.id)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })

  it('closing an inactive tab keeps the selection', async () => {
    const { tabs, a, b, c } = await three()
    tabs.select(c.id)
    tabs.close(a.id)
    expect(tabs.active('s1')?.id).toBe(c.id)
    expect(tabs.list.map((t) => t.id)).toEqual([b.id, c.id])
  })

  it('closing the only tab leaves the session with no active tab', async () => {
    const { tabs } = await fresh()
    const only = tabs.openTable('s1', T('a'))
    tabs.close(only.id)
    expect(tabs.list).toEqual([])
    expect(tabs.active('s1')).toBeNull()
    expect('s1' in tabs.activeBySession).toBe(false)
  })

  it('"next" is chosen among the same session\'s tabs, ignoring other sessions', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('a'))
    tabs.openTable('s2', T('x'))
    const b = tabs.openTable('s1', T('b'))
    tabs.select(a.id)
    tabs.close(a.id)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })

  it('does not touch other sessions\' selection', async () => {
    const { tabs } = await fresh()
    const x = tabs.openTable('s2', T('x'))
    const a = tabs.openTable('s1', T('a'))
    tabs.close(a.id)
    expect(tabs.active('s2')?.id).toBe(x.id)
  })

  it('closing an unknown id is a no-op', async () => {
    const { tabs, a, b, c } = await three()
    tabs.close('ghost')
    expect(tabs.list.map((t) => t.id)).toEqual([a.id, b.id, c.id])
  })

  it('can close query tabs too', async () => {
    const { tabs } = await fresh()
    const t = tabs.openTable('s1', T('a'))
    const q = tabs.newQuery('s1', 'db')
    tabs.close(q.id)
    expect(tabs.active('s1')?.id).toBe(t.id)
  })
})

describe('tabs: removeSession', () => {
  it('drops all of the session\'s tabs and selection', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.newQuery('s1', 'db')
    const x = tabs.openTable('s2', T('x'))
    tabs.removeSession('s1')
    expect(tabs.list.map((t) => t.id)).toEqual([x.id])
    expect(tabs.active('s1')).toBeNull()
    expect(tabs.active('s2')?.id).toBe(x.id)
  })

  it('restarts the "Query N" numbering', async () => {
    const { tabs } = await fresh()
    tabs.newQuery('s1', 'db')
    tabs.newQuery('s1', 'db')
    tabs.removeSession('s1')
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 1')
  })

  it('is harmless for an unknown session', async () => {
    const { tabs } = await fresh()
    expect(() => tabs.removeSession('ghost')).not.toThrow()
  })
})

describe('tabs: cycle', () => {
  async function three() {
    const ctx = await fresh()
    const a = ctx.tabs.openTable('s1', T('a'))
    const b = ctx.tabs.openTable('s1', T('b'))
    const c = ctx.tabs.openTable('s1', T('c'))
    return { ...ctx, a, b, c }
  }

  it('moves forward and wraps around', async () => {
    const { tabs, a, b, c } = await three()
    tabs.select(a.id)
    tabs.cycle('s1', 1)
    expect(tabs.active('s1')?.id).toBe(b.id)
    tabs.cycle('s1', 1)
    expect(tabs.active('s1')?.id).toBe(c.id)
    tabs.cycle('s1', 1)
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('moves backward and wraps around', async () => {
    const { tabs, a, b, c } = await three()
    tabs.select(a.id)
    tabs.cycle('s1', -1)
    expect(tabs.active('s1')?.id).toBe(c.id)
    tabs.cycle('s1', -1)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })

  it('does nothing with fewer than two tabs', async () => {
    const { tabs } = await fresh()
    const only = tabs.openTable('s1', T('a'))
    tabs.cycle('s1', 1)
    expect(tabs.active('s1')?.id).toBe(only.id)
    tabs.cycle('empty', -1)
    expect(tabs.active('empty')).toBeNull()
  })

  it('only cycles within the given session', async () => {
    const { tabs, a, b } = await three()
    const x = tabs.openTable('s2', T('x'))
    tabs.select(b.id)
    tabs.select(a.id)
    tabs.cycle('s1', 1)
    expect(tabs.active('s2')?.id).toBe(x.id)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })
})

describe('tabs: selectIndex', () => {
  async function five() {
    const ctx = await fresh()
    const made = ['a', 'b', 'c', 'd', 'e'].map((n) => ctx.tabs.openTable('s1', T(n)))
    return { ...ctx, made }
  }

  it('selects the nth tab (1-based)', async () => {
    const { tabs, made } = await five()
    tabs.selectIndex('s1', 1)
    expect(tabs.active('s1')?.id).toBe(made[0].id)
    tabs.selectIndex('s1', 3)
    expect(tabs.active('s1')?.id).toBe(made[2].id)
  })

  it('9 selects the last tab, like browsers', async () => {
    const { tabs, made } = await five()
    tabs.selectIndex('s1', 9)
    expect(tabs.active('s1')?.id).toBe(made[4].id)
  })

  it('an index past the end is ignored (except 9)', async () => {
    const { tabs, made } = await five()
    tabs.select(made[1].id)
    tabs.selectIndex('s1', 7)
    expect(tabs.active('s1')?.id).toBe(made[1].id)
  })

  it('0 and negative indices are ignored', async () => {
    const { tabs, made } = await five()
    tabs.select(made[1].id)
    tabs.selectIndex('s1', 0)
    tabs.selectIndex('s1', -1)
    expect(tabs.active('s1')?.id).toBe(made[1].id)
  })

  it('9 on a session with no tabs is a no-op', async () => {
    const { tabs } = await fresh()
    expect(() => tabs.selectIndex('none', 9)).not.toThrow()
  })
})

describe('tabs: restore', () => {
  const saved = (over: Record<string, unknown> = {}) => ({
    tabs: [
      { kind: 'table', table: T('users', 'public'), title: 'users' },
      { kind: 'query', database: 'app', sql: 'SELECT 1', title: 'Query 1' },
      { kind: 'query', database: 'app', sql: 'SELECT 2', title: 'Query 3' },
    ],
    active: 1,
    ...over,
  })

  it('does nothing when nothing was saved, but starts tracking the session', async () => {
    const { tabs, settleSave, storage } = await fresh()
    tabs.restore('s1', 'conn-1')
    expect(tabs.list).toEqual([])
    tabs.openTable('s1', T('a'))
    settleSave()
    expect(storage.json<{ tabs: unknown[] }>(KEY('conn-1'))?.tabs).toHaveLength(1)
  })

  it('reopens the saved tabs in order with fresh ids', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved() })
    tabs.restore('s1', 'conn-1')
    const list = tabs.forSession('s1')
    expect(list.map((t) => t.title)).toEqual(['users', 'Query 1', 'Query 3'])
    expect(list.map((t) => t.kind)).toEqual(['table', 'query', 'query'])
    expect(list.every((t) => t.sessionId === 's1')).toBe(true)
    expect(new Set(list.map((t) => t.id)).size).toBe(3)
  })

  it('restores table and query details', async () => {
    const f: Filter[] = [{ column: 'id', op: 'gt', value: '5' }]
    const data = {
      tabs: [
        { kind: 'table', table: T('orders', 'sales'), title: 'orders', filters: f, preview: true },
        { kind: 'query', database: 'dw', sql: 'SELECT *\nFROM x', title: 'Query 1' },
      ],
      active: 0,
    }
    const { tabs } = await fresh({ [KEY('c')]: data })
    tabs.restore('s1', 'c')
    const [t, q] = tabs.forSession('s1')
    expect(t).toMatchObject({ kind: 'table', table: T('orders', 'sales'), filters: f, preview: true })
    expect(q).toMatchObject({ kind: 'query', database: 'dw', sql: 'SELECT *\nFROM x' })
  })

  it('selects the saved active tab', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved({ active: 2 }) })
    tabs.restore('s1', 'conn-1')
    expect(tabs.active('s1')?.title).toBe('Query 3')
  })

  it('clamps an out-of-range active index to the last tab', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved({ active: 99 }) })
    tabs.restore('s1', 'conn-1')
    expect(tabs.active('s1')?.title).toBe('Query 3')
  })

  it('a missing active index selects the first tab', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: { tabs: saved().tabs } })
    tabs.restore('s1', 'conn-1')
    expect(tabs.active('s1')?.title).toBe('users')
  })

  it('continues "Query N" numbering after the highest restored number', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved() }) // has "Query 1" and "Query 3"
    tabs.restore('s1', 'conn-1')
    expect(tabs.newQuery('s1', 'app').title).toBe('Query 4')
  })

  it('renamed query titles do not affect numbering', async () => {
    const data = { tabs: [{ kind: 'query', database: 'd', sql: '', title: 'My report' }], active: 0 }
    const { tabs } = await fresh({ [KEY('c')]: data })
    tabs.restore('s1', 'c')
    expect(tabs.newQuery('s1', 'd').title).toBe('Query 1')
  })

  it('skips malformed saved entries', async () => {
    const data = {
      tabs: [
        { kind: 'table', table: { database: 'd' }, title: 'no name' },
        { kind: 'table', title: 'no table' },
        { kind: 'query', database: 'd', title: 'Query 1' }, // no sql
        { kind: 'query', database: 'd', sql: 42, title: 'Query 2' },
        { kind: 'mystery' },
        { kind: 'table', table: T('ok'), title: 'ok' },
      ],
      active: 0,
    }
    const { tabs } = await fresh({ [KEY('c')]: data })
    expect(() => tabs.restore('s1', 'c')).not.toThrow()
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['ok'])
  })

  it('ignores corrupt or unrelated stored data', async () => {
    for (const raw of ['{{{', 'null', '[]', '{"tabs":"x"}', '{}']) {
      const { tabs } = await fresh({ [KEY('c')]: raw })
      expect(() => tabs.restore('s1', 'c')).not.toThrow()
      expect(tabs.list).toEqual([])
    }
  })

  it('does not overwrite tabs that are already open for the session', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved() })
    const existing = tabs.openTable('s1', T('already'))
    tabs.restore('s1', 'conn-1')
    expect(tabs.forSession('s1').map((t) => t.id)).toEqual([existing.id])
  })

  it('is a no-op the second time for the same session and connection', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved() })
    tabs.restore('s1', 'conn-1')
    tabs.restore('s1', 'conn-1')
    expect(tabs.forSession('s1')).toHaveLength(3)
  })

  it('restores only the matching connection\'s tabs', async () => {
    const { tabs } = await fresh({
      [KEY('c1')]: { tabs: [{ kind: 'table', table: T('one'), title: 'one' }], active: 0 },
      [KEY('c2')]: { tabs: [{ kind: 'table', table: T('two'), title: 'two' }], active: 0 },
    })
    tabs.restore('s1', 'c1')
    tabs.restore('s2', 'c2')
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['one'])
    expect(tabs.forSession('s2').map((t) => t.title)).toEqual(['two'])
  })

  it('restored tabs behave like any other (open dedupes against them)', async () => {
    const { tabs } = await fresh({ [KEY('conn-1')]: saved() })
    tabs.restore('s1', 'conn-1')
    tabs.openTable('s1', T('users', 'public'))
    expect(tabs.forSession('s1')).toHaveLength(3)
  })
})

describe('tabs: persistence', () => {
  it('saves nothing for sessions that were never restored (not tracked)', async () => {
    const { tabs, settleSave, storage } = await fresh()
    tabs.openTable('s1', T('a'))
    settleSave()
    expect([...storage.data.keys()].filter((k) => k.startsWith('tablesoft.tabs.'))).toEqual([])
  })

  it('writes the tabs 400 ms after a change, not before', async () => {
    const { tabs, flush, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    vi.advanceTimersByTime(400)
    storage.data.clear()
    tabs.openTable('s1', T('a'))
    flush()
    vi.advanceTimersByTime(399)
    expect(storage.getItem(KEY('c1'))).toBeNull()
    vi.advanceTimersByTime(1)
    expect(storage.json(KEY('c1'))).toMatchObject({ tabs: [{ kind: 'table', title: 'a' }] })
  })

  it('debounces a burst of changes into one write', async () => {
    const { tabs, flush, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    vi.advanceTimersByTime(400)
    const spy = vi.spyOn(storage, 'setItem')
    spy.mockClear()
    for (const n of ['a', 'b', 'c']) {
      tabs.openTable('s1', T(n))
      flush()
      vi.advanceTimersByTime(100)
    }
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(400)
    const writes = spy.mock.calls.filter(([k]) => k === KEY('c1'))
    expect(writes).toHaveLength(1)
    expect(JSON.parse(writes[0][1]).tabs.map((t: { title: string }) => t.title)).toEqual(['a', 'b', 'c'])
  })

  it('serialises table tabs without ids and with filters and preview', async () => {
    const f: Filter[] = [{ column: 'id', op: 'eq', value: '1' }]
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.openTable('s1', T('orders', 'sales'), f, { preview: true })
    settleSave()
    const data = storage.json<{ tabs: Record<string, unknown>[]; active: number }>(KEY('c1'))!
    expect(data.active).toBe(0)
    expect(data.tabs[0]).toEqual({ kind: 'table', table: T('orders', 'sales'), title: 'orders', filters: f, preview: true })
    expect(data.tabs[0]).not.toHaveProperty('id')
    expect(data.tabs[0]).not.toHaveProperty('sessionId')
  })

  it('serialises query tabs with their SQL', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.newQuery('s1', 'app', 'SELECT 1;')
    settleSave()
    expect(storage.json<{ tabs: unknown[] }>(KEY('c1'))?.tabs).toEqual([
      { kind: 'query', database: 'app', sql: 'SELECT 1;', title: 'Query 1' },
    ])
  })

  it('saves edits made to the SQL text of an existing tab', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.newQuery('s1', 'app', 'SELECT 1;')
    settleSave()
    const live = tabs.forSession('s1')[0]
    if (live.kind !== 'query') throw new Error('expected a query tab')
    live.sql = 'SELECT 2;'
    settleSave()
    expect(storage.json<{ tabs: { sql: string }[] }>(KEY('c1'))?.tabs[0].sql).toBe('SELECT 2;')
  })

  it('saves the promotion of a preview tab', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    const p = tabs.openTable('s1', T('a'), undefined, { preview: true })
    settleSave()
    expect(storage.json<{ tabs: { preview?: boolean }[] }>(KEY('c1'))?.tabs[0].preview).toBe(true)
    tabs.keep(p.id)
    settleSave()
    expect(storage.json<{ tabs: { preview?: boolean }[] }>(KEY('c1'))?.tabs[0].preview).toBeFalsy()
  })

  it('saves the active tab index', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    const a = tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    settleSave()
    expect(storage.json<{ active: number }>(KEY('c1'))?.active).toBe(1)
    tabs.select(a.id)
    settleSave()
    expect(storage.json<{ active: number }>(KEY('c1'))?.active).toBe(0)
  })

  it('saves closing a tab', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    const a = tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    settleSave()
    tabs.close(a.id)
    settleSave()
    expect(storage.json<{ tabs: { title: string }[] }>(KEY('c1'))?.tabs.map((t) => t.title)).toEqual(['b'])
  })

  it('saves an empty list after the last tab is closed (so it does not come back)', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    const a = tabs.openTable('s1', T('a'))
    settleSave()
    tabs.close(a.id)
    settleSave()
    expect(storage.json(KEY('c1'))).toEqual({ tabs: [], active: 0 })
  })

  it('keeps each session\'s tabs under its own connection key', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    tabs.restore('s2', 'c2')
    flush()
    tabs.openTable('s1', T('one'))
    tabs.openTable('s2', T('two'))
    settleSave()
    expect(storage.json<{ tabs: { title: string }[] }>(KEY('c1'))?.tabs.map((t) => t.title)).toEqual(['one'])
    expect(storage.json<{ tabs: { title: string }[] }>(KEY('c2'))?.tabs.map((t) => t.title)).toEqual(['two'])
  })

  it('removeSession saves synchronously, before the debounce', async () => {
    const { tabs, flush, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.openTable('s1', T('a'))
    tabs.newQuery('s1', 'app', 'SELECT 1')
    // no flush, no timers: the debounced save has not run
    expect(storage.getItem(KEY('c1'))).toBeNull()
    tabs.removeSession('s1')
    expect(storage.json<{ tabs: { title: string }[] }>(KEY('c1'))?.tabs.map((t) => t.title)).toEqual(['a', 'Query 1'])
  })

  it('an empty list is never written after removeSession (the debounce cannot clobber it)', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.openTable('s1', T('a'))
    settleSave()
    tabs.openTable('s1', T('b'))
    flush() // schedules a debounced save of [a, b]
    tabs.removeSession('s1')
    flush()
    vi.advanceTimersByTime(2000)
    flush()
    vi.advanceTimersByTime(2000)
    expect(storage.json<{ tabs: { title: string }[] }>(KEY('c1'))?.tabs.map((t) => t.title)).toEqual(['a', 'b'])
  })

  it('a removed session is no longer tracked: later tabs under the same id are not saved', async () => {
    const { tabs, flush, settleSave, storage } = await fresh()
    tabs.restore('s1', 'c1')
    flush()
    tabs.openTable('s1', T('a'))
    settleSave()
    tabs.removeSession('s1')
    storage.data.clear()
    tabs.openTable('s1', T('stray'))
    settleSave()
    expect(storage.getItem(KEY('c1'))).toBeNull()
  })

  it('removeSession on an untracked session writes nothing', async () => {
    const { tabs, storage } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.removeSession('s1')
    expect(storage.data.size).toBe(0)
  })

  it('survives a full restart: close the session, start a new store, restore', async () => {
    const first = await fresh()
    first.tabs.restore('s1', 'c1')
    first.flush()
    first.tabs.openTable('s1', T('users', 'public'), undefined, { preview: true })
    first.tabs.openTable('s1', T('orders', 'public'))
    first.tabs.newQuery('s1', 'app', 'SELECT * FROM orders;')
    first.tabs.newQuery('s1', 'app', 'SELECT 2;')
    first.tabs.select(first.tabs.forSession('s1')[1].id) // 'orders'
    first.settleSave()
    first.tabs.removeSession('s1')

    const second = await fresh({}, first.storage)
    second.tabs.restore('sess-new', 'c1')
    const list = second.tabs.forSession('sess-new')
    expect(list.map((t) => t.title)).toEqual(['users', 'orders', 'Query 1', 'Query 2'])
    expect(list[0]).toMatchObject({ kind: 'table', preview: true })
    expect(list[2]).toMatchObject({ kind: 'query', sql: 'SELECT * FROM orders;' })
    expect(second.tabs.active('sess-new')?.title).toBe('orders')
    expect(second.tabs.newQuery('sess-new', 'app').title).toBe('Query 3')
  })

  it('works when localStorage is unavailable', async () => {
    vi.resetModules()
    vi.stubGlobal('localStorage', undefined)
    const { flushSync } = await import('svelte')
    const { tabs } = await import('./tabs.svelte')
    expect(() => {
      tabs.restore('s1', 'c1')
      tabs.openTable('s1', T('a'))
      flushSync()
      vi.advanceTimersByTime(400)
      tabs.removeSession('s1')
    }).not.toThrow()
  })

  it('works when localStorage rejects writes', async () => {
    vi.resetModules()
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem() {
        throw new Error('quota')
      },
    })
    const { flushSync } = await import('svelte')
    const { tabs } = await import('./tabs.svelte')
    expect(() => {
      tabs.restore('s1', 'c1')
      tabs.openTable('s1', T('a'))
      flushSync()
      vi.advanceTimersByTime(400)
    }).not.toThrow()
    expect(tabs.list).toHaveLength(1)
  })
})
