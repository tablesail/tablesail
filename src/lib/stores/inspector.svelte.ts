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
  /** Which page of the right panel is showing. */
  tab = $state<'details' | 'assistant'>('details')
  current = $state<InspectedRow | null>(null)
  /** Last row each tab published, so only picking a different row reopens the panel. */
  private lastRow = new Map<string, number>()

  show(row: InspectedRow) {
    this.current = row
    if (this.lastRow.get(row.tabId) === row.rowIndex) return
    this.lastRow.set(row.tabId, row.rowIndex)
    // Like TablePlus: selecting a row brings up its details.
    if (!this.open) {
      this.tab = 'details'
      this.open = true
    }
  }

  clear(tabId: string) {
    this.lastRow.delete(tabId)
    if (this.current?.tabId === tabId) this.current = null
  }

  toggle() {
    this.open = !this.open
  }
}

export const inspector = new InspectorStore()
