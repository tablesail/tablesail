import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'
import type { Filter, TableRef } from '../types'

// closeTable / renameTable / reloadTable: what the sidebar's drop, rename and truncate do to open table tabs.

async function fresh() {
  vi.resetModules()
  installStorage()
  const { tabs } = await import('./tabs.svelte')
  return { tabs }
}

const T = (name: string, schema: string | null = 'public', database = 'db'): TableRef => ({ database, schema, name })
const F = (column: string): Filter => ({ column, op: 'eq', value: '1' })

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

describe('tabs.closeTable', () => {
  it('closes the tab of the table, filtered variants too, and nothing else', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users'))
    tabs.openTable('s1', T('users'), [F('id')])
    tabs.openTable('s1', T('orders'))
    tabs.newQuery('s1', 'db')
    tabs.closeTable('s1', T('users'))
    expect(tabs.list.map((t) => t.title)).toEqual(['orders', 'Query 1'])
  })

  it('leaves same-named tables of another schema, database or session', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users', 'billing'))
    tabs.openTable('s1', T('users', 'public', 'other'))
    tabs.openTable('s2', T('users'))
    tabs.openTable('s1', T('users'))
    tabs.closeTable('s1', T('users'))
    expect(tabs.list).toHaveLength(3)
    expect(tabs.forSession('s2')).toHaveLength(1)
  })

  it('moves the selection to a neighbour when the active tab goes', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.openTable('s1', T('b'))
    const c = tabs.openTable('s1', T('c'))
    expect(tabs.active('s1')?.id).toBe(c.id)
    tabs.closeTable('s1', T('c'))
    expect(tabs.active('s1')?.title).toBe('b')
  })

  it('does nothing when no tab shows the table', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    tabs.closeTable('s1', T('zzz'))
    expect(tabs.list).toHaveLength(1)
  })
})

describe('tabs.renameTable', () => {
  it('retargets and retitles the tab, keeping its position and selection', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    const b = tabs.openTable('s1', T('b'))
    tabs.openTable('s1', T('c'))
    tabs.select(b.id)
    tabs.renameTable('s1', T('b'), 'b2')
    expect(tabs.list.map((t) => t.title)).toEqual(['a', 'b2', 'c'])
    const renamed = tabs.list[1]
    expect(renamed.kind === 'table' && renamed.table).toEqual({ database: 'db', schema: 'public', name: 'b2' })
    expect(tabs.active('s1')?.id).toBe(renamed.id)
  })

  it('gives the tab a fresh id, so the view remounts on the new table', async () => {
    const { tabs } = await fresh()
    const before = tabs.openTable('s1', T('a'))
    tabs.renameTable('s1', T('a'), 'a2')
    expect(tabs.list[0].id).not.toBe(before.id)
  })

  it('keeps filters and the "(filtered)" suffix, and the preview flag', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users'), [F('id')])
    tabs.openTable('s1', T('users'), undefined, { preview: true })
    tabs.renameTable('s1', T('users'), 'people')
    expect(tabs.list.map((t) => t.title)).toEqual(['people (filtered)', 'people'])
    const filtered = tabs.list[0]
    expect(filtered.kind === 'table' && filtered.filters).toEqual([F('id')])
    const preview = tabs.list[1]
    expect(preview.kind === 'table' && preview.preview).toBe(true)
  })

  it('opening the renamed table afterwards finds the retargeted tab', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users'))
    tabs.renameTable('s1', T('users'), 'people')
    tabs.openTable('s1', T('people'))
    expect(tabs.list).toHaveLength(1)
  })

  it('does not touch other tables, a table named like a prefix, or other sessions', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('users'))
    tabs.openTable('s1', T('users_archive'))
    tabs.openTable('s2', T('users'))
    tabs.renameTable('s1', T('users'), 'people')
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['people', 'users_archive'])
    expect(tabs.forSession('s2').map((t) => t.title)).toEqual(['users'])
  })
})

describe('tabs.reloadTable', () => {
  it('replaces the tab with an identical one under a new id, still selected', async () => {
    const { tabs } = await fresh()
    tabs.openTable('s1', T('a'))
    const b = tabs.openTable('s1', T('b'), [F('id')])
    expect(tabs.active('s1')?.id).toBe(b.id)
    tabs.reloadTable('s1', T('b'))
    const again = tabs.list[1]
    expect(again.id).not.toBe(b.id)
    expect(again).toMatchObject({ kind: 'table', title: 'b (filtered)', table: { name: 'b' }, filters: [F('id')] })
    expect(tabs.active('s1')?.id).toBe(again.id)
    expect(tabs.list).toHaveLength(2)
  })

  it('keeps an inactive tab inactive', async () => {
    const { tabs } = await fresh()
    const a = tabs.openTable('s1', T('a'))
    const b = tabs.openTable('s1', T('b'))
    tabs.reloadTable('s1', T('a'))
    expect(tabs.active('s1')?.id).toBe(b.id)
    expect(tabs.list[0].id).not.toBe(a.id)
  })
})
