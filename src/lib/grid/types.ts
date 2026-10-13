import type { CellInput, ColumnMeta, Value } from '../types'

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
