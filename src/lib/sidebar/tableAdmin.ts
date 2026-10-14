// What the sidebar offers for dropping, truncating and renaming an object, and the wording of the confirmation dialog.
// Pure functions (no Svelte), so the rules are unit-tested.

import type { Engine, TableAction, TableKind, TableRef } from '../types'

export type AdminKind = TableAction['kind']

/** What the dialog reports back once the server ran the action. */
export interface AdminDone {
  action: AdminKind
  /** The new name, for a rename. */
  newName?: string
}

/**
 * The actions offered for an object, in menu order. Views can't be truncated; SQLite has no ALTER VIEW and
 * MongoDB can't rename a view; time-series collections can't be renamed in MongoDB; sequences exist as
 * objects of their own only on Postgres, MySQL and MariaDB; system objects are left alone.
 */
export function adminActions(kind: TableKind, engine: Engine): AdminKind[] {
  switch (kind) {
    case 'table':
    case 'virtual':
    case 'collection':
      return ['rename', 'truncate', 'drop']
    case 'timeSeries':
      return ['truncate', 'drop']
    case 'view':
      return engine === 'sqlite' || engine === 'mongodb' ? ['drop'] : ['rename', 'drop']
    case 'materializedView':
    case 'foreignTable':
      return ['rename', 'drop']
    case 'sequence':
      return engine === 'sqlite' || engine === 'mongodb' ? [] : ['rename', 'drop']
    default:
      return []
  }
}

/** "table", "view", "collection"… as used in sentences. */
export function noun(kind: TableKind, engine: Engine): string {
  if (engine === 'mongodb') return kind === 'view' ? 'view' : 'collection'
  switch (kind) {
    case 'view':
      return 'view'
    case 'sequence':
      return 'sequence'
    case 'materializedView':
      return 'materialized view'
    case 'foreignTable':
      return 'foreign table'
    default:
      return 'table'
  }
}

/** Tables and collections hold data of their own; views, sequences and the like do not. */
export function holdsRows(kind: TableKind): boolean {
  return kind === 'table' || kind === 'virtual' || kind === 'collection' || kind === 'timeSeries'
}

/** What a collection or table holds: "rows" / "documents". */
export function itemsNoun(engine: Engine): string {
  return engine === 'mongodb' ? 'documents' : 'rows'
}

/** Context-menu label. */
export function menuLabel(action: AdminKind, engine: Engine): string {
  if (action === 'rename') return 'Rename…'
  if (action === 'drop') return 'Drop…'
  return engine === 'mongodb' ? 'Empty collection…' : 'Truncate…'
}

/** Dialog title and accessible name: "Drop table", "Empty collection"… */
export function dialogTitle(action: AdminKind, kind: TableKind, engine: Engine): string {
  const what = noun(kind, engine)
  if (action === 'truncate') return engine === 'mongodb' ? 'Empty collection' : `Truncate ${what}`
  return `${action === 'drop' ? 'Drop' : 'Rename'} ${what}`
}

/** Label of the dialog's primary button. */
export function confirmLabel(action: AdminKind, engine: Engine): string {
  if (action === 'rename') return 'Rename'
  if (action === 'drop') return 'Drop'
  return engine === 'mongodb' ? 'Empty' : 'Truncate'
}

/** Past tense for the toast: "Dropped", "Truncated", "Emptied", "Renamed". */
export function doneVerb(action: AdminKind, engine: Engine): string {
  if (action === 'rename') return 'Renamed'
  if (action === 'drop') return 'Dropped'
  return engine === 'mongodb' ? 'Emptied' : 'Truncated'
}

/** Why a rename can't go ahead yet, or null. */
export function renameProblem(current: string, input: string): string | null {
  const next = input.trim()
  if (!next) return 'Enter a new name.'
  if (next === current) return 'Choose a different name.'
  return null
}

/** Dropping asks for the name to be typed (surrounding spaces from a paste are ignored). */
export function dropConfirmed(name: string, typed: string): boolean {
  return typed.trim() === name
}

/** The action to send for a dialog: null while a rename has no usable name. */
export function actionFor(action: AdminKind, current: string, input: string): TableAction | null {
  if (action !== 'rename') return { kind: action }
  return renameProblem(current, input) ? null : { kind: 'rename', newName: input.trim() }
}

/** The tree key (`schema.name` or `name`) of an object, as the sidebar and the schema snapshot use it. */
export function treeKey(schema: string | null | undefined, name: string): string {
  return schema ? `${schema}.${name}` : name
}

/**
 * The localStorage keys of what the table view remembers per table name: the pending-changes draft (under
 * the session id and the connection id) and the hidden columns. Must match `draftKeys` / `hiddenKey` in TableView.svelte.
 */
export function tableStateKeys(sessionId: string, configId: string, table: TableRef): string[] {
  const t = `${table.database}.${table.schema ?? ''}.${table.name}`
  return [`tablesail:draft:${sessionId}:${t}`, `tablesail:draft:${configId}:${t}`, `tablesail.hiddenColumns.${configId}.${t}`]
}

/**
 * Keeps that per-table state with the table: after a rename (`newName`) it moves to the new name, so the new
 * tab offers the pending changes again; after a drop (`newName` null) it is forgotten, so a table created later
 * under the same name doesn't inherit it.
 */
export function carryTableState(
  sessionId: string,
  configId: string,
  table: TableRef,
  newName: string | null,
  storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> = localStorage,
) {
  try {
    const from = tableStateKeys(sessionId, configId, table)
    const to = newName === null ? [] : tableStateKeys(sessionId, configId, { ...table, name: newName })
    from.forEach((key, i) => {
      const value = to[i] ? storage.getItem(key) : null
      if (value !== null) storage.setItem(to[i], value)
      storage.removeItem(key)
    })
  } catch {
    // storage blocked: the draft and the column choice are conveniences
  }
}
