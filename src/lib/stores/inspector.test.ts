import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { CellInput, SessionInfo } from '../types'
import type { InspectedRow } from './inspector.svelte'

async function fresh() {
  vi.resetModules()
  return (await import('./inspector.svelte')).inspector
}

const session = { sessionId: 's1' } as unknown as SessionInfo

function row(tabId: string, rowIndex: number, over: Partial<InspectedRow> = {}): InspectedRow {
  return {
    tabId,
    session,
    table: { database: 'db', name: 't' },
    columns: [{ name: 'id', typeName: 'int', family: 'integer' }],
    row: [rowIndex],
    rowIndex,
    primaryKey: ['id'],
    foreignKeys: [],
    editable: true,
    ...over,
  }
}

let inspector: Awaited<ReturnType<typeof fresh>>
beforeEach(async () => {
  vi.useFakeTimers()
  inspector = await fresh()
})

/** show() opens the panel after a short delay (so double-clicks land); run it to completion. */
function show(r: InspectedRow) {
  inspector.show(r)
  vi.runAllTimers()
}

describe('inspector: initial state', () => {
  it('is closed with nothing selected, on the details page', () => {
    expect(inspector.open).toBe(false)
    expect(inspector.current).toBeNull()
    expect(inspector.tab).toBe('details')
  })
})

describe('inspector.show', () => {
  it('publishes the row', () => {
    show(row('t1', 3))
    expect(inspector.current?.tabId).toBe('t1')
    expect(inspector.current?.rowIndex).toBe(3)
    expect(inspector.current?.row).toEqual([3])
  })

  it('opens the panel on the first row a tab publishes', () => {
    show(row('t1', 0))
    expect(inspector.open).toBe(true)
    expect(inspector.tab).toBe('details')
  })

  it('selecting a different row reopens a panel the user closed', () => {
    show(row('t1', 0))
    inspector.open = false
    show(row('t1', 1))
    expect(inspector.open).toBe(true)
  })

  it('re-publishing the same row does not reopen a closed panel', () => {
    show(row('t1', 4))
    inspector.open = false
    show(row('t1', 4))
    show(row('t1', 4))
    expect(inspector.open).toBe(false)
  })

  it('re-publishing the same row still refreshes the data shown', () => {
    show(row('t1', 4, { row: ['old'] }))
    inspector.open = false
    show(row('t1', 4, { row: ['new'] }))
    expect(inspector.current?.row).toEqual(['new'])
    expect(inspector.open).toBe(false)
  })

  it('the same row index in another tab counts as a new selection', () => {
    show(row('t1', 2))
    inspector.open = false
    show(row('t2', 2))
    expect(inspector.open).toBe(true)
  })

  it('remembers the last row of each tab independently', () => {
    show(row('t1', 1))
    show(row('t2', 5))
    inspector.open = false
    show(row('t1', 1)) // t1 is still on row 1
    expect(inspector.open).toBe(false)
    show(row('t2', 5)) // t2 is still on row 5
    expect(inspector.open).toBe(false)
    show(row('t1', 2)) // a real change
    expect(inspector.open).toBe(true)
  })

  it('going back to an earlier row counts as a different row', () => {
    show(row('t1', 1))
    show(row('t1', 2))
    inspector.open = false
    show(row('t1', 1))
    expect(inspector.open).toBe(true)
  })

  it('brings the details page forward when it opens from the assistant page', () => {
    inspector.tab = 'assistant'
    show(row('t1', 0))
    expect(inspector.open).toBe(true)
    expect(inspector.tab).toBe('details')
  })

  it('does not switch pages when the panel is already open', () => {
    inspector.open = true
    inspector.tab = 'assistant'
    show(row('t1', 0))
    show(row('t1', 1))
    expect(inspector.tab).toBe('assistant')
    expect(inspector.open).toBe(true)
  })

  it('keeps the row at index 0 distinct from "never seen" (0 is a real row)', () => {
    show(row('t1', 0))
    inspector.open = false
    show(row('t1', 0))
    expect(inspector.open).toBe(false)
  })

  it('carries the optional edit callbacks through', () => {
    const commits: [number, CellInput][] = []
    show(
      row('t1', 0, {
        edit: {
          canEdit: (c) => c === 0,
          commit: (c, input) => commits.push([c, input]),
        },
      }),
    )
    expect(inspector.current?.edit?.canEdit(0)).toBe(true)
    expect(inspector.current?.edit?.canEdit(1)).toBe(false)
    inspector.current?.edit?.commit(0, { kind: 'text', value: 'x' })
    expect(commits).toEqual([[0, { kind: 'text', value: 'x' }]])
  })

  it('a row without edit callbacks has none', () => {
    show(row('t1', 0))
    expect(inspector.current?.edit).toBeUndefined()
  })
})

describe('inspector.clear', () => {
  it('clears the current row when that tab published it', () => {
    show(row('t1', 0))
    inspector.clear('t1')
    expect(inspector.current).toBeNull()
  })

  it('leaves the current row alone when another tab published it', () => {
    show(row('t1', 0))
    show(row('t2', 3))
    inspector.clear('t1')
    expect(inspector.current?.tabId).toBe('t2')
  })

  it('forgets the tab\'s last row, so the same row reopens the panel afterwards', () => {
    show(row('t1', 7))
    inspector.open = false
    inspector.clear('t1')
    show(row('t1', 7))
    expect(inspector.open).toBe(true)
  })

  it('does not touch the open state', () => {
    show(row('t1', 0))
    inspector.clear('t1')
    expect(inspector.open).toBe(true)
  })

  it('clearing an unknown tab is harmless', () => {
    show(row('t1', 0))
    expect(() => inspector.clear('ghost')).not.toThrow()
    expect(inspector.current?.tabId).toBe('t1')
  })

  it('clearing when nothing is shown is harmless', () => {
    expect(() => inspector.clear('t1')).not.toThrow()
    expect(inspector.current).toBeNull()
  })
})

describe('inspector.toggle', () => {
  it('flips the open flag', () => {
    inspector.toggle()
    expect(inspector.open).toBe(true)
    inspector.toggle()
    expect(inspector.open).toBe(false)
  })

  it('does not change the page', () => {
    inspector.tab = 'assistant'
    inspector.toggle()
    expect(inspector.tab).toBe('assistant')
  })
})

describe('inspector: open delay', () => {
  it('waits a double-click interval before opening, so the layout does not shift mid double-click', async () => {
    const { OPEN_DELAY_MS } = await import('./inspector.svelte')
    inspector.show(row('t1', 0))
    expect(inspector.open).toBe(false)
    vi.advanceTimersByTime(OPEN_DELAY_MS)
    expect(inspector.open).toBe(true)
  })
})
