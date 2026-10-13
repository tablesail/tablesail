// What the right-hand Row inspector shows: the selected row of the active grid.

import type { CellInput, ColumnMeta, ForeignKeyInfo, Row, SessionInfo, TableRef } from '../types'

export interface InspectedRow {
  /** Which tab published this, so a hidden tab never overwrites the visible one. */
  tabId: string
  session: SessionInfo
  /** Source table (null for query results). */
  table: TableRef | null
  columns: ColumnMeta[]
  row: Row
  rowIndex: number
  primaryKey: string[]
  foreignKeys: ForeignKeyInfo[]
  editable: boolean
  /** Present when fields can be edited from the panel (stages a pending change, like a cell edit). */
  edit?: {
    canEdit: (column: number) => boolean
    commit: (column: number, input: CellInput) => void
  }
}

/** Longer than a double-click, so the second click still hits the cell it aimed at. */
export const OPEN_DELAY_MS = 350

class InspectorStore {
  open = $state(false)
  /** Which page of the right panel is showing. */
  tab = $state<'details' | 'assistant'>('details')
  current = $state<InspectedRow | null>(null)
  /** Last row each tab published, so only picking a different row reopens the panel. */
  private lastRow = new Map<string, number>()
  private openTimer: ReturnType<typeof setTimeout> | undefined
  /** A row click scheduled the panel to open (it counts as open for the toggle). */
  opening = $state(false)

  show(row: InspectedRow) {
    this.current = row
    if (this.lastRow.get(row.tabId) === row.rowIndex) return
    this.lastRow.set(row.tabId, row.rowIndex)
    // Like TablePlus: selecting a row brings up its details. Opening narrows the grid, so wait
    // until a double-click on the same cell has had time to land where the user aimed.
    if (!this.open) {
      clearTimeout(this.openTimer)
      this.opening = true
      this.openTimer = setTimeout(() => {
        this.opening = false
        if (this.open) return
        this.tab = 'details'
        this.open = true
      }, OPEN_DELAY_MS)
    }
  }

  /** The tab no longer has a selected row (e.g. after Discard): stop showing its old values. */
  unpublish(tabId: string) {
    if (this.current?.tabId === tabId) this.current = null
  }

  clear(tabId: string) {
    this.lastRow.delete(tabId)
    if (this.current?.tabId === tabId) this.current = null
  }

  /** Closes the panel, including one that is about to open after a row click. */
  close() {
    clearTimeout(this.openTimer)
    this.opening = false
    this.open = false
  }

  toggle() {
    if (this.open) this.close()
    else this.open = true
  }
}

export const inspector = new InspectorStore()
