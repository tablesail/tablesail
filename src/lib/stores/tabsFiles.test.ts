import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage, type MemoryStorage } from '../../test/helpers'
import type { QueryTab } from './tabs.svelte'

// File-backed query tabs, the manual-transaction fields, and the close guard that protects them.

async function fresh(initial: Record<string, unknown> = {}) {
  vi.resetModules()
  const storage = installStorage(initial)
  const { flushSync } = await import('svelte')
  const mod = await import('./tabs.svelte')
  /** Runs pending effects, then lets the 400 ms save debounce elapse. */
  const settleSave = () => {
    flushSync()
    vi.advanceTimersByTime(400)
  }
  return { ...mod, storage, settleSave }
}

const KEY = (conn: string) => `tablesail.tabs.${conn}`
const queryOf = (t: unknown) => t as QueryTab
// newQuery hands back the plain object it pushed; later updates are visible through the store's reactive copy.
const live = <T extends { id: string }>(tabs: { list: { id: string }[] }, t: T) => tabs.list.find((x) => x.id === t.id) as unknown as QueryTab

// The first import compiles the svelte runtime and the store, which is slow on a busy machine.
beforeAll(async () => {
  await fresh()
  vi.unstubAllGlobals()
}, 60_000)

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('newQuery with a file', () => {
  it('titles the tab with the file name and starts it saved', async () => {
    const { tabs, queryDirty } = await fresh()
    const tab = tabs.newQuery('s1', 'db', 'SELECT 1;', { file: '/home/me/queries/report.sql' })
    expect(tab).toMatchObject({ title: 'report.sql', file: '/home/me/queries/report.sql', sql: 'SELECT 1;', savedSql: 'SELECT 1;' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('knows Windows paths too', async () => {
    const { tabs } = await fresh()
    expect(tabs.newQuery('s1', 'db', '', { file: 'C:\\sql\\dump.sql' }).title).toBe('dump.sql')
  })

  it('does not use up a "Query N" number', async () => {
    const { tabs } = await fresh()
    tabs.newQuery('s1', 'db', 'x', { file: '/a.sql' })
    expect(tabs.newQuery('s1', 'db').title).toBe('Query 1')
  })

  it('a tab without a file is never dirty', async () => {
    const { tabs, queryDirty } = await fresh()
    const tab = live(tabs, tabs.newQuery('s1', 'db', 'SELECT 1'))
    tabs.updateQuery(tab.id, { sql: 'SELECT 2' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('becomes dirty when the text differs from what was saved, and clean again when it matches', async () => {
    const { tabs, queryDirty } = await fresh()
    const tab = live(tabs, tabs.newQuery('s1', 'db', 'SELECT 1', { file: '/a.sql' }))
    tabs.updateQuery(tab.id, { sql: 'SELECT 12' })
    expect(queryDirty(tab)).toBe(true)
    tabs.updateQuery(tab.id, { sql: 'SELECT 1' })
    expect(queryDirty(tab)).toBe(false)
    tabs.updateQuery(tab.id, { sql: 'SELECT 3' })
    tabs.updateQuery(tab.id, { savedSql: 'SELECT 3' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('table tabs are never dirty', async () => {
    const { tabs, queryDirty } = await fresh()
    expect(queryDirty(tabs.openTable('s1', { database: 'db', schema: null, name: 'users' }))).toBe(false)
  })
})

describe('close guard', () => {
  it('closes a plain query tab at once', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db', 'SELECT 1')
    tabs.close(tab.id)
    expect(tabs.list).toEqual([])
    expect(tabs.closeRequest).toBeNull()
  })

  it('closes a saved file tab at once', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db', 'SELECT 1', { file: '/a.sql' })
    tabs.close(tab.id)
    expect(tabs.list).toEqual([])
  })

  it('keeps a file tab with unsaved edits, selects it and asks', async () => {
    const { tabs } = await fresh()
    const a = tabs.newQuery('s1', 'db', 'SELECT 1', { file: '/a.sql' })
    tabs.newQuery('s1', 'db')
    tabs.updateQuery(a.id, { sql: 'SELECT 2' })
    tabs.close(a.id)
    expect(tabs.list).toHaveLength(2)
    expect(tabs.closeRequest).toBe(a.id)
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('keeps a tab with an open transaction and asks', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db', 'UPDATE t SET a = 1')
    tabs.updateQuery(tab.id, { txId: 'tx1', txStatements: 1 })
    tabs.close(tab.id)
    expect(tabs.list).toHaveLength(1)
    expect(tabs.closeRequest).toBe(tab.id)
  })

  it('force closes it anyway and clears the request', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db', 'x', { file: '/a.sql' })
    tabs.updateQuery(tab.id, { sql: 'y' })
    tabs.close(tab.id)
    tabs.close(tab.id, true)
    expect(tabs.list).toEqual([])
    expect(tabs.closeRequest).toBeNull()
  })

  it('closes normally once the transaction has ended', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db')
    tabs.updateQuery(tab.id, { txId: 'tx1' })
    tabs.updateQuery(tab.id, { txId: null })
    tabs.close(tab.id)
    expect(tabs.list).toEqual([])
  })

  it('removing the session drops the pending request without asking', async () => {
    const { tabs } = await fresh()
    const tab = tabs.newQuery('s1', 'db')
    tabs.updateQuery(tab.id, { txId: 'tx1' })
    tabs.close(tab.id)
    tabs.removeSession('s1')
    expect(tabs.list).toEqual([])
    expect(tabs.closeRequest).toBeNull()
  })

  it('needsCloseConfirm covers exactly dirty files and open transactions', async () => {
    const { tabs, needsCloseConfirm } = await fresh()
    const table = live(tabs, tabs.openTable('s1', { database: 'db', schema: null, name: 'users' }))
    const plain = live(tabs, tabs.newQuery('s1', 'db', 'x'))
    const file = live(tabs, tabs.newQuery('s1', 'db', 'x', { file: '/a.sql' }))
    expect([table, plain, file].map((t) => needsCloseConfirm(t))).toEqual([false, false, false])
    tabs.updateQuery(file.id, { sql: 'y' })
    tabs.updateQuery(plain.id, { txId: 'tx9' })
    expect([table, plain, file].map((t) => needsCloseConfirm(t))).toEqual([false, true, true])
  })
})

describe('persistence of files and transaction mode', () => {
  const saved = (storage: MemoryStorage) => storage.json<{ tabs: Record<string, unknown>[] }>(KEY('c1'))!.tabs

  it('saves the file, the dirty flag and the manual toggle, but never the transaction', async () => {
    const { tabs, storage, settleSave } = await fresh()
    tabs.restore('s1', 'c1')
    const clean = tabs.newQuery('s1', 'db', 'SELECT 1', { file: '/a.sql' })
    const dirty = tabs.newQuery('s1', 'db', 'SELECT 2', { file: '/b.sql' })
    tabs.updateQuery(dirty.id, { sql: 'SELECT 22' })
    const manual = tabs.newQuery('s1', 'db', 'UPDATE t SET a = 1')
    tabs.updateQuery(manual.id, { manual: true, txId: 'tx1', txStatements: 4 })
    settleSave()
    const [a, b, c] = saved(storage)
    expect(a).toMatchObject({ kind: 'query', file: '/a.sql', title: 'a.sql' })
    expect(a.dirty).toBeUndefined()
    expect(b).toMatchObject({ file: '/b.sql', sql: 'SELECT 22', dirty: true })
    expect(c).toMatchObject({ manual: true })
    expect(JSON.stringify(storage.data.get(KEY('c1')))).not.toContain('tx1')
    expect(Object.keys(c)).not.toContain('txId')
    expect(live(tabs, clean).txId).toBeUndefined()
  })

  it('restores a clean file tab as saved and a dirty one as unsaved', async () => {
    const { tabs, queryDirty } = await fresh({
      [KEY('c1')]: {
        tabs: [
          { kind: 'query', database: 'db', sql: 'SELECT 1', title: 'a.sql', file: '/a.sql' },
          { kind: 'query', database: 'db', sql: 'SELECT 22', title: 'b.sql', file: '/b.sql', dirty: true },
          { kind: 'query', database: 'db', sql: 'x', title: 'Query 2', manual: true },
        ],
        active: 0,
      },
    })
    tabs.restore('s1', 'c1')
    const [a, b, c] = tabs.forSession('s1').map(queryOf)
    expect(a).toMatchObject({ file: '/a.sql', savedSql: 'SELECT 1' })
    expect(queryDirty(a)).toBe(false)
    expect(b.file).toBe('/b.sql')
    expect(queryDirty(b)).toBe(true)
    expect(c.manual).toBe(true)
    expect(c.file).toBeUndefined()
    expect([a, b, c].every((t) => !t.txId)).toBe(true)
  })

  it('keeps the CRLF line ending of a file across a restart (and drops it without a file)', async () => {
    const { tabs, storage, settleSave } = await fresh()
    tabs.restore('s1', 'c1')
    tabs.newQuery('s1', 'db', 'a\nb', { file: '/win.sql', eol: '\r\n' })
    tabs.newQuery('s1', 'db', 'x', { eol: '\r\n' })
    settleSave()
    const [win, none] = saved(storage)
    expect(win).toMatchObject({ file: '/win.sql', eol: '\r\n' })
    expect(none.eol).toBeUndefined()

    const again = await fresh({ [KEY('c1')]: storage.json(KEY('c1')) })
    again.tabs.restore('s1', 'c1')
    const [restored] = again.tabs.forSession('s1').map(queryOf)
    expect(restored).toMatchObject({ file: '/win.sql', eol: '\r\n', sql: 'a\nb' })
    expect(again.queryDirty(restored)).toBe(false)
  })

  it('tabs saved before files existed still restore', async () => {
    const { tabs, queryDirty } = await fresh({
      [KEY('c1')]: { tabs: [{ kind: 'query', database: 'db', sql: 'SELECT 1', title: 'Query 1' }], active: 0 },
    })
    tabs.restore('s1', 'c1')
    const [q] = tabs.forSession('s1').map(queryOf)
    expect(q).toMatchObject({ sql: 'SELECT 1', title: 'Query 1' })
    expect(q.file).toBeUndefined()
    expect(queryDirty(q)).toBe(false)
  })

  it('ignores a file that is not a string', async () => {
    const { tabs } = await fresh({
      [KEY('c1')]: { tabs: [{ kind: 'query', database: 'db', sql: 'x', title: 'Query 1', file: 42 }], active: 0 },
    })
    tabs.restore('s1', 'c1')
    expect(queryOf(tabs.list[0]).file).toBeUndefined()
  })
})
