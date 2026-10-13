import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BrowseRequest, SessionInfo, TableRef } from '../types'

async function fresh() {
  vi.resetModules()
  return (await import('./ui.svelte')).ui
}

let ui: Awaited<ReturnType<typeof fresh>>
beforeEach(async () => {
  vi.useFakeTimers()
  ui = await fresh()
})
afterEach(() => {
  vi.useRealTimers()
})

describe('ui defaults', () => {
  it('starts with everything closed', () => {
    expect(ui.toasts).toEqual([])
    expect(ui.rail).toBe('browser')
    expect(ui.palette).toBe(false)
    expect(ui.console).toBe(false)
    expect(ui.sidebarHidden).toBe(false)
    expect(ui.settings).toBe(false)
    expect(ui.exportRequest).toBeNull()
    expect(ui.importRequest).toBeNull()
    expect(ui.shortcutsHelp).toBe(false)
    expect(ui.about).toBe(false)
  })
})

describe('ui.toast', () => {
  it('adds an info toast by default', () => {
    ui.toast('Saved')
    expect(ui.toasts).toEqual([{ id: 1, kind: 'info', text: 'Saved' }])
  })

  it('supports every kind', () => {
    ui.toast('a', 'info')
    ui.toast('b', 'error')
    ui.toast('c', 'success')
    expect(ui.toasts.map((t) => t.kind)).toEqual(['info', 'error', 'success'])
  })

  it('gives each toast a distinct, increasing id and keeps order', () => {
    ui.toast('one')
    ui.toast('two')
    ui.toast('three')
    expect(ui.toasts.map((t) => t.id)).toEqual([1, 2, 3])
    expect(ui.toasts.map((t) => t.text)).toEqual(['one', 'two', 'three'])
  })

  it('removes an info toast after 4 seconds', () => {
    ui.toast('bye')
    vi.advanceTimersByTime(3999)
    expect(ui.toasts).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(ui.toasts).toHaveLength(0)
  })

  it('removes a success toast after 4 seconds too', () => {
    ui.toast('ok', 'success')
    vi.advanceTimersByTime(4000)
    expect(ui.toasts).toHaveLength(0)
  })

  it('keeps an error toast twice as long', () => {
    ui.toast('bad', 'error')
    vi.advanceTimersByTime(7999)
    expect(ui.toasts).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(ui.toasts).toHaveLength(0)
  })

  it('honours a custom duration (doubled for errors)', () => {
    ui.toast('quick', 'info', 1000)
    ui.toast('quick error', 'error', 1000)
    vi.advanceTimersByTime(1000)
    expect(ui.toasts.map((t) => t.text)).toEqual(['quick error'])
    vi.advanceTimersByTime(1000)
    expect(ui.toasts).toEqual([])
  })

  it('each toast expires on its own schedule', () => {
    ui.toast('first')
    vi.advanceTimersByTime(2000)
    ui.toast('second')
    vi.advanceTimersByTime(2000) // first is 4s old
    expect(ui.toasts.map((t) => t.text)).toEqual(['second'])
    vi.advanceTimersByTime(2000)
    expect(ui.toasts).toEqual([])
  })

  it('dismiss removes only the given toast', () => {
    ui.toast('a')
    ui.toast('b')
    ui.toast('c')
    ui.dismiss(2)
    expect(ui.toasts.map((t) => t.text)).toEqual(['a', 'c'])
  })

  it('dismissing an unknown id changes nothing', () => {
    ui.toast('a')
    ui.dismiss(99)
    expect(ui.toasts).toHaveLength(1)
  })

  it('the timer of an already-dismissed toast is harmless', () => {
    ui.toast('a')
    ui.dismiss(1)
    ui.toast('b') // id 2
    vi.advanceTimersByTime(4000)
    expect(ui.toasts).toEqual([])
    expect(() => vi.runAllTimers()).not.toThrow()
  })

  it('does not reuse an id after a toast expired', () => {
    ui.toast('a')
    vi.advanceTimersByTime(4000)
    ui.toast('b')
    expect(ui.toasts[0].id).toBe(2)
  })

  it('replaces the array on change (so keyed lists re-render)', () => {
    ui.toast('a')
    const before = ui.toasts
    ui.toast('b')
    expect(ui.toasts).not.toBe(before)
  })
})

describe('ui export / import requests', () => {
  const session = { sessionId: 's1' } as unknown as SessionInfo
  const table: TableRef = { database: 'db', schema: 'public', name: 'users' }

  it('exportTable builds a table source named after the table', () => {
    const request: BrowseRequest = { table, filters: [], matchAll: true, sort: [], columns: [] }
    ui.exportTable(session, request)
    expect(ui.exportRequest).toMatchObject({ source: { kind: 'table', request }, name: 'users' })
    expect(ui.exportRequest?.session.sessionId).toBe('s1')
  })

  it('exportQuery builds a query source named "query"', () => {
    ui.exportQuery(session, 'db', 'SELECT 1')
    expect(ui.exportRequest).toMatchObject({ source: { kind: 'query', database: 'db', sql: 'SELECT 1' }, name: 'query' })
  })

  it('exportQuery accepts a null database', () => {
    ui.exportQuery(session, null, 'SELECT 1')
    expect(ui.exportRequest?.source).toEqual({ kind: 'query', database: null, sql: 'SELECT 1' })
  })

  it('a new export request replaces the previous one', () => {
    ui.exportQuery(session, null, 'A')
    ui.exportQuery(session, null, 'B')
    expect((ui.exportRequest?.source as { sql: string }).sql).toBe('B')
  })

  it('importInto records the target table', () => {
    ui.importInto(session, table)
    expect(ui.importRequest?.table).toEqual(table)
    expect(ui.importRequest?.session.sessionId).toBe('s1')
  })

  it('the requests can be cleared by the dialogs', () => {
    ui.importInto(session, table)
    ui.exportQuery(session, null, 'x')
    ui.importRequest = null
    ui.exportRequest = null
    expect(ui.importRequest).toBeNull()
    expect(ui.exportRequest).toBeNull()
  })
})
