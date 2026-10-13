// Pending edits of one table: edited cells, inserted rows and deleted rows.
//
// The store is plain data behind a version counter, so a grid with thousands of rows
// pays nothing per row: components read `version` (directly or through any accessor
// below) to re-render when something changed.

import { DEFAULT_MARKER_TYPE, base64ToBytes, isTruncatedValue, valueToEditText } from '../format'
import type {
  CellInput,
  ChangeSet,
  ColumnValue,
  RowChange,
  TableRef,
  TypeFamily,
  Value,
} from '../types'

export interface CellEdit {
  /** Value as loaded (goes into the optimistic WHERE clause). */
  original: Value
  input: CellInput
}

interface RowEdit {
  key: ColumnValue[]
  cells: Map<string, CellEdit>
}

export interface InsertedRow {
  id: number
  values: Map<string, CellInput>
}

export interface ChangeSummary {
  inserts: number
  updates: number
  deletes: number
}

/** Serialisable form, used for the localStorage draft. */
export interface ChangeDraft {
  version: 1
  inserts: { id: number; values: [string, CellInput][] }[]
  edits: { key: ColumnValue[]; cells: { column: string; original: Value; input: CellInput }[] }[]
  deletes: ColumnValue[][]
}

/** Stable identity of a row: its primary key values as loaded. */
export function keyString(key: ColumnValue[]): string {
  return JSON.stringify(key.map((k) => k.value))
}

export const DEFAULT_MARKER: Value = { $other: 'DEFAULT', type: DEFAULT_MARKER_TYPE }

function isTrue(text: string): boolean {
  return /^(true|t|1|yes|on)$/i.test(text.trim())
}

/** What the grid shows for a pending input. */
export function inputToValue(input: CellInput, family?: TypeFamily): Value {
  switch (input.kind) {
    case 'null':
      return null
    case 'default':
      return DEFAULT_MARKER
    case 'original':
      return input.value
    case 'text':
      return family === 'boolean' ? isTrue(input.value) : input.value
    case 'json':
      return { $json: input.value }
    case 'bytes':
      try {
        return { $bytes: input.value, len: base64ToBytes(input.value).length }
      } catch {
        return { $other: input.value, type: 'bytes' }
      }
  }
}

/** An input that re-inserts a loaded value, or undefined when only a truncated prefix is known. */
export function valueToInput(value: Value | undefined): CellInput | undefined {
  if (value === undefined || isTruncatedValue(value)) return undefined
  if (value === null) return { kind: 'null' }
  if (typeof value === 'object' && '$other' in value && value.type === DEFAULT_MARKER_TYPE) return undefined
  return { kind: 'original', value }
}

function isUnchanged(original: Value, input: CellInput): boolean {
  switch (input.kind) {
    case 'original':
      return true
    case 'null':
      return original === null
    case 'default':
    case 'bytes':
      return false
    case 'json':
      return (
        typeof original === 'object' &&
        original !== null &&
        '$json' in original &&
        JSON.stringify(original.$json) === JSON.stringify(input.value)
      )
    case 'text': {
      if (original === null || original === undefined) return false
      if (typeof original === 'boolean') return isTrue(input.value) === original
      if (typeof original === 'object') {
        // Truncated text or bytes can't be compared; JSON has its own editor.
        if ('$text' in original || '$bytes' in original || '$json' in original) return false
      }
      return valueToEditText(original) === input.value
    }
  }
}

export class ChangeStore {
  /** Bumped on every mutation; read it to depend on the store. */
  version = $state(0)

  private edits = new Map<string, RowEdit>()
  private deleted = new Map<string, ColumnValue[]>()
  private inserts: InsertedRow[] = []
  private nextId = 1

  private touch() {
    this.version++
  }

  get isEmpty(): boolean {
    void this.version
    return this.edits.size === 0 && this.deleted.size === 0 && this.inserts.length === 0
  }

  /** True when any existing row has an edit or a delete (fast path for the grid). */
  get hasRowChanges(): boolean {
    void this.version
    return this.edits.size > 0 || this.deleted.size > 0
  }

  get insertCount(): number {
    void this.version
    return this.inserts.length
  }

  get summary(): ChangeSummary {
    void this.version
    let updates = 0
    for (const [ks, e] of this.edits) if (!this.deleted.has(ks) && e.cells.size > 0) updates++
    return { inserts: this.inserts.length, updates, deletes: this.deleted.size }
  }

  /** Number of pending operations (rows inserted + rows updated + rows deleted). */
  get pending(): number {
    const s = this.summary
    return s.inserts + s.updates + s.deletes
  }

  // -- inserted rows -------------------------------------------------------

  /** Display order is newest first. */
  insertedAt(displayIndex: number): InsertedRow | undefined {
    void this.version
    return this.inserts[this.inserts.length - 1 - displayIndex]
  }

  addInsert(values?: Map<string, CellInput>): InsertedRow {
    const row: InsertedRow = { id: this.nextId++, values: new Map(values ?? []) }
    this.inserts.push(row)
    this.touch()
    return row
  }

  setInsertCell(id: number, column: string, input: CellInput) {
    const row = this.inserts.find((r) => r.id === id)
    if (!row) return
    if (input.kind === 'default') row.values.delete(column)
    else row.values.set(column, input)
    this.touch()
  }

  removeInsert(id: number) {
    const i = this.inserts.findIndex((r) => r.id === id)
    if (i < 0) return
    this.inserts.splice(i, 1)
    this.touch()
  }

  // -- edits and deletes of existing rows ----------------------------------

  editFor(keyStr: string, column: string): CellEdit | undefined {
    void this.version
    return this.edits.get(keyStr)?.cells.get(column)
  }

  rowEdit(keyStr: string): ReadonlyMap<string, CellEdit> | undefined {
    void this.version
    return this.edits.get(keyStr)?.cells
  }

  isDeleted(keyStr: string): boolean {
    void this.version
    return this.deleted.has(keyStr)
  }

  /** Records an edit; setting a cell back to its loaded value removes the edit. */
  setCell(key: ColumnValue[], column: string, original: Value, input: CellInput) {
    const ks = keyString(key)
    let row = this.edits.get(ks)
    if (isUnchanged(original, input)) {
      if (!row?.cells.delete(column)) return
      if (row.cells.size === 0) this.edits.delete(ks)
    } else {
      if (!row) {
        row = { key, cells: new Map() }
        this.edits.set(ks, row)
      }
      // Keep the first original: it is what the database still holds.
      const prev = row.cells.get(column)
      row.cells.set(column, { original: prev ? prev.original : original, input })
    }
    this.touch()
  }

  setDeleted(key: ColumnValue[], deleted: boolean) {
    const ks = keyString(key)
    if (deleted === this.deleted.has(ks)) return
    if (deleted) this.deleted.set(ks, key)
    else this.deleted.delete(ks)
    this.touch()
  }

  // -- lifecycle -----------------------------------------------------------

  clear() {
    this.edits.clear()
    this.deleted.clear()
    this.inserts = []
    this.touch()
  }

  discardRow(keyStr: string) {
    // Both maps, always: `||` would skip the delete when the row also has edits.
    const hadEdits = this.edits.delete(keyStr)
    const hadDelete = this.deleted.delete(keyStr)
    if (hadEdits || hadDelete) this.touch()
  }

  /** Statements are built from this: inserts first, then updates, then deletes. */
  buildChangeSet(table: TableRef): ChangeSet {
    const changes: RowChange[] = []
    for (const row of this.inserts) {
      changes.push({
        kind: 'insert',
        values: [...row.values].map(([column, value]) => ({ column, value })),
      })
    }
    for (const [ks, row] of this.edits) {
      if (this.deleted.has(ks) || row.cells.size === 0) continue
      const cells = [...row.cells]
      changes.push({
        kind: 'update',
        key: row.key,
        original: cells.map(([column, c]) => ({ column, value: c.original })),
        set: cells.map(([column, c]) => ({ column, value: c.input })),
      })
    }
    for (const key of this.deleted.values()) changes.push({ kind: 'delete', key })
    return { table, changes }
  }

  toDraft(): ChangeDraft {
    return {
      version: 1,
      inserts: this.inserts.map((r) => ({ id: r.id, values: [...r.values] })),
      edits: [...this.edits.values()].map((e) => ({
        key: e.key,
        cells: [...e.cells].map(([column, c]) => ({ column, original: c.original, input: c.input })),
      })),
      deletes: [...this.deleted.values()],
    }
  }

  restore(draft: ChangeDraft) {
    this.edits.clear()
    this.deleted.clear()
    this.inserts = []
    for (const r of draft.inserts) {
      this.inserts.push({ id: r.id, values: new Map(r.values) })
      this.nextId = Math.max(this.nextId, r.id + 1)
    }
    for (const e of draft.edits) {
      this.edits.set(keyString(e.key), {
        key: e.key,
        cells: new Map(e.cells.map((c) => [c.column, { original: c.original, input: c.input }])),
      })
    }
    for (const key of draft.deletes) this.deleted.set(keyString(key), key)
    this.touch()
  }
}

export function isChangeDraft(x: unknown): x is ChangeDraft {
  if (typeof x !== 'object' || x === null) return false
  const d = x as Partial<ChangeDraft>
  return d.version === 1 && Array.isArray(d.inserts) && Array.isArray(d.edits) && Array.isArray(d.deletes)
}
