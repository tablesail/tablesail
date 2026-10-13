// What the right-hand Row inspector shows: the selected row of the active grid.

import type { ColumnMeta, ForeignKeyInfo, Row, SessionInfo, TableRef } from '../types'

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
}

class InspectorStore {
  open = $state(false)
  current = $state<InspectedRow | null>(null)

  show(row: InspectedRow) {
    this.current = row
  }

  clear(tabId: string) {
    if (this.current?.tabId === tabId) this.current = null
  }

  toggle() {
    this.open = !this.open
  }
}

export const inspector = new InspectorStore()
