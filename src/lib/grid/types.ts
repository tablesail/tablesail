import type { CellInput, ColumnMeta, Value } from '../types'
import type { RowFormat } from '../util/rowFormats'

/** Normalised rectangular selection; row/col bounds are inclusive. */
export interface GridSelection {
  row0: number
  row1: number
  col0: number
  col1: number
  /** The cell with the focus ring (where typing/Enter acts). */
  activeRow: number
  activeCol: number
}

export type EditMove = 'none' | 'down' | 'right' | 'left'

/** Handed to the `editor` snippet of DataGrid while a cell is being edited. */
export interface EditorContext {
  row: number
  col: number
  column: ColumnMeta
  value: Value | undefined
  /** Text typed to start the edit (replaces the content), if any. */
  initial: string | undefined
  /** Size of the editor box in px. */
  width: number
  height: number
  commit: (input: CellInput, move?: EditMove) => void
  cancel: () => void
}

/** What DataGrid.copyAs() put on the clipboard. */
export interface CopyReport {
  format: RowFormat
  /** Rows written to the clipboard. */
  rows: number
  /** Selected rows that were not loaded and so left out. */
  skipped: number
  /** Selected rows past the copy limit, left out. */
  capped: number
  /** Cells that only hold the beginning of a long text or binary value. */
  truncated: number
  /** False when the clipboard refused the text. */
  written: boolean
}
