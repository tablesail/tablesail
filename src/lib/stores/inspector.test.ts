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
  inspector = await fresh()
})

describe('inspector: initial state', () => {
  it('is closed with nothing selected, on the details page', () => {
    expect(inspector.open).toBe(false)
    expect(inspector.current).toBeNull()
    expect(inspector.tab).toBe('details')
  })
})

describe('inspector.show', () => {
  it('publishes the row', () => {
    inspector.show(row('t1', 3))
    expect(inspector.current?.tabId).toBe('t1')
    expect(inspector.current?.rowIndex).toBe(3)
    expect(inspector.current?.row).toEqual([3])
  })

  it('opens the panel on the first row a tab publishes', () => {
    inspector.show(row('t1', 0))
    expect(inspector.open).toBe(true)
    expect(inspector.tab).toBe('details')
  })

  it('selecting a different row reopens a panel the user closed', () => {
    inspector.show(row('t1', 0))
    inspector.open = false
    inspector.show(row('t1', 1))
    expect(inspector.open).toBe(true)
  })

  it('re-publishing the same row does not reopen a closed panel', () => {
    inspector.show(row('t1', 4))
    inspector.open = false
    inspector.show(row('t1', 4))
    inspector.show(row('t1', 4))
    expect(inspector.open).toBe(false)
  })

  it('re-publishing the same row still refreshes the data shown', () => {
    inspector.show(row('t1', 4, { row: ['old'] }))
    inspector.open = false
    inspector.show(row('t1', 4, { row: ['new'] }))
    expect(inspector.current?.row).toEqual(['new'])
    expect(inspector.open).toBe(false)
  })

  it('the same row index in another tab counts as a new selection', () => {
    inspector.show(row('t1', 2))
    inspector.open = false
    inspector.show(row('t2', 2))
    expect(inspector.open).toBe(true)
  })

  it('remembers the last row of each tab independently', () => {
    inspector.show(row('t1', 1))
    inspector.show(row('t2', 5))
    inspector.open = false
    inspector.show(row('t1', 1)) // t1 is still on row 1
    expect(inspector.open).toBe(false)
    inspector.show(row('t2', 5)) // t2 is still on row 5
    expect(inspector.open).toBe(false)
    inspector.show(row('t1', 2)) // a real change
    expect(inspector.open).toBe(true)
  })

  it('going back to an earlier row counts as a different row', () => {
    inspector.show(row('t1', 1))
    inspector.show(row('t1', 2))
    inspector.open = false
    inspector.show(row('t1', 1))
    expect(inspector.open).toBe(true)
  })

  it('brings the details page forward when it opens from the assistant page', () => {
    inspector.tab = 'assistant'
    inspector.show(row('t1', 0))
    expect(inspector.open).toBe(true)
    expect(inspector.tab).toBe('details')
  })

  it('does not switch pages when the panel is already open', () => {
    inspector.open = true
    inspector.tab = 'assistant'
    inspector.show(row('t1', 0))
    inspector.show(row('t1', 1))
    expect(inspector.tab).toBe('assistant')
    expect(inspector.open).toBe(true)
  })

  it('keeps the row at index 0 distinct from "never seen" (0 is a real row)', () => {
    inspector.show(row('t1', 0))
    inspector.open = false
    inspector.show(row('t1', 0))
    expect(inspector.open).toBe(false)
  })

  it('carries the optional edit callbacks through', () => {
    const commits: [number, CellInput][] = []
    inspector.show(
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
    inspector.show(row('t1', 0))
    expect(inspector.current?.edit).toBeUndefined()
  })
})

describe('inspector.clear', () => {
  it('clears the current row when that tab published it', () => {
    inspector.show(row('t1', 0))
    inspector.clear('t1')
    expect(inspector.current).toBeNull()
  })

  it('leaves the current row alone when another tab published it', () => {
    inspector.show(row('t1', 0))
    inspector.show(row('t2', 3))
    inspector.clear('t1')
    expect(inspector.current?.tabId).toBe('t2')
  })

  it('forgets the tab\'s last row, so the same row reopens the panel afterwards', () => {
    inspector.show(row('t1', 7))
    inspector.open = false
    inspector.clear('t1')
    inspector.show(row('t1', 7))
    expect(inspector.open).toBe(true)
  })

  it('does not touch the open state', () => {
    inspector.show(row('t1', 0))
    inspector.clear('t1')
    expect(inspector.open).toBe(true)
  })

  it('clearing an unknown tab is harmless', () => {
    inspector.show(row('t1', 0))
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
