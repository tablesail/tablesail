// Rows as text, both ways.
//
// Copy side: a selection of grid rows becomes INSERT statements, a JSON array of objects, a
// Markdown table or tab-separated text. Paste side: clipboard text is parsed into a grid of
// cells and each cell is turned into the CellInput that the change tracking understands.
//
// Everything here is pure (no DOM, no stores) so it can be tested per engine.

import {
  DEFAULT_MARKER_TYPE,
  base64ToBytes,
  bytesToBase64,
  bytesToHex,
  isNumericFamily,
  isTruncatedValue,
  valueToEditText,
} from '../format'
import type { CellInput, ColumnMeta, Engine, Row, TableRef, TypeFamily, Value } from '../types'
import { qualifiedName, quoteIdent } from './sql'

// ---------------------------------------------------------------------------
// Copy as...
// ---------------------------------------------------------------------------

export type RowFormat = 'insert' | 'json' | 'markdown' | 'tsv'

export const ROW_FORMAT_LABEL: Record<RowFormat, string> = {
  insert: 'INSERT statements',
  json: 'JSON',
  markdown: 'Markdown table',
  tsv: 'Tab-separated',
}

/** MongoDB has no SQL and no tables of columns: JSON (and plain text) only. */
export function availableFormats(engine: Engine): RowFormat[] {
  return engine === 'mongodb' ? ['json', 'tsv'] : ['insert', 'json', 'markdown', 'tsv']
}

/** Table name used in INSERT statements copied from a query result, which has no single source table. */
export const PLACEHOLDER_TABLE = 'table_name'

export interface FormatInput {
  engine: Engine
  /** The table the rows came from; null for query results (INSERT then uses PLACEHOLDER_TABLE). */
  table?: TableRef | null
  columns: ColumnMeta[]
  rows: Row[]
}

export function formatRows(format: RowFormat, input: FormatInput): string {
  switch (format) {
    case 'insert':
      return toInsertStatements(input.engine, input.table ?? null, input.columns, input.rows)
    case 'json':
      return toJson(input.engine, input.columns, input.rows)
    case 'markdown':
      return toMarkdown(input.columns, input.rows)
    case 'tsv':
      return toTsv(input.columns, input.rows, false)
  }
}

const plural = (n: number, one: string, many = one + 's') => `${n} ${n === 1 ? one : many}`

/** The toast after "Copy as": what was copied, and what had to be left out or cut short. */
export function copyMessage(
  format: RowFormat,
  report: { rows: number; skipped: number; truncated: number; capped?: number; written?: boolean },
): { text: string; kind: 'success' | 'error' } {
  if (report.rows === 0) return { text: 'Nothing to copy: the selected rows are not loaded.', kind: 'error' }
  if (report.written === false) return { text: 'Could not copy to the clipboard.', kind: 'error' }
  const parts = [`Copied ${plural(report.rows, 'row')} as ${ROW_FORMAT_LABEL[format]}.`]
  if (report.skipped > 0) parts.push(`${plural(report.skipped, 'row')} not loaded, left out.`)
  if (report.capped) parts.push(`${plural(report.capped, 'row')} over the copy limit, left out.`)
  if (report.truncated > 0) parts.push(`${plural(report.truncated, 'long value')} cut short.`)
  return { text: parts.join(' '), kind: 'success' }
}

/** Cells that only hold a prefix of the stored text or bytes (they are copied as loaded). */
export function countTruncated(rows: Row[]): number {
  let n = 0
  for (const row of rows) for (const v of row) if (isTruncatedValue(v)) n++
  return n
}

function isDefaultMarker(v: Value | undefined): boolean {
  return typeof v === 'object' && v !== null && '$other' in v && v.type === DEFAULT_MARKER_TYPE
}

// -- SQL literals ------------------------------------------------------------

const INT_RE = /^-?\d+$/
const NUMBER_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/

function isMysql(engine: Engine): boolean {
  return engine === 'mysql' || engine === 'mariadb'
}

/** A quoted string literal. MySQL and MariaDB also treat the backslash as an escape character. */
export function quoteString(engine: Engine, s: string): string {
  let out = s.replaceAll("'", "''")
  if (isMysql(engine)) out = out.replaceAll('\\', '\\\\').replaceAll('\0', '\\0').replaceAll('\x1a', '\\Z')
  return "'" + out + "'"
}

function nonFiniteLiteral(engine: Engine, kind: 'NaN' | 'Infinity' | '-Infinity'): string {
  switch (engine) {
    case 'postgres':
      return `'${kind}'`
    case 'sqlite':
      return kind === 'NaN' ? 'NULL' : kind === 'Infinity' ? '9e999' : '-9e999'
    default:
      return 'NULL'
  }
}

/** One value as a SQL literal: NULL, numbers and booleans bare, everything else quoted. */
export function sqlLiteral(engine: Engine, value: Value, family?: TypeFamily): string {
  if (value === null) return 'NULL'
  switch (typeof value) {
    case 'boolean':
      return engine === 'sqlite' ? (value ? '1' : '0') : value ? 'TRUE' : 'FALSE'
    case 'number':
      if (Number.isFinite(value)) return String(value)
      return nonFiniteLiteral(engine, Number.isNaN(value) ? 'NaN' : value > 0 ? 'Infinity' : '-Infinity')
    case 'string':
      // A numeric column that arrives as text (SQLite affinities) stays a number.
      return isNumericFamily(family) && NUMBER_RE.test(value.trim()) ? value.trim() : quoteString(engine, value)
  }
  if ('$int' in value) return INT_RE.test(value.$int) ? value.$int : quoteString(engine, value.$int)
  if ('$dec' in value) return NUMBER_RE.test(value.$dec) ? value.$dec : quoteString(engine, value.$dec)
  if ('$float' in value) return nonFiniteLiteral(engine, value.$float)
  if ('$text' in value) return quoteString(engine, value.$text)
  if ('$bytes' in value) {
    const hex = bytesToHex(base64ToBytes(value.$bytes))
    return engine === 'postgres' ? `'\\x${hex}'` : `X'${hex}'`
  }
  if ('$json' in value) return quoteString(engine, JSON.stringify(value.$json) ?? 'null')
  if ('$date' in value) return quoteString(engine, value.$date)
  if ('$time' in value) return quoteString(engine, value.$time)
  if ('$datetime' in value) return quoteString(engine, value.$datetime)
  if ('$uuid' in value) return quoteString(engine, value.$uuid)
  if ('$oid' in value) return quoteString(engine, value.$oid)
  return value.type === DEFAULT_MARKER_TYPE ? 'DEFAULT' : quoteString(engine, value.$other)
}

/**
 * One INSERT statement per row. Columns holding the "use the default" marker of a pending new row are
 * left out of that row's statement, so the default applies when the statements are run.
 */
export function toInsertStatements(engine: Engine, table: TableRef | null, columns: ColumnMeta[], rows: Row[]): string {
  const target = table ? qualifiedName(engine, table) : PLACEHOLDER_TABLE
  const names = columns.map((c) => quoteIdent(engine, c.name))
  const out: string[] = []
  for (const row of rows) {
    const cols: string[] = []
    const vals: string[] = []
    columns.forEach((c, i) => {
      const v = row[i]
      if (v === undefined || isDefaultMarker(v)) return
      cols.push(names[i])
      vals.push(sqlLiteral(engine, v, c.family))
    })
    if (cols.length === 0) {
      out.push(isMysql(engine) ? `INSERT INTO ${target} () VALUES ();` : `INSERT INTO ${target} DEFAULT VALUES;`)
    } else {
      out.push(`INSERT INTO ${target} (${cols.join(', ')}) VALUES (${vals.join(', ')});`)
    }
  }
  return out.join('\n')
}

// -- JSON --------------------------------------------------------------------

const JSON_NUMBER_RE = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$/

/** Pretty JSON for a nested document, indented to sit inside an object property. */
function nested(value: unknown, indent: string): string {
  return (JSON.stringify(value, null, 2) ?? 'null').replaceAll('\n', '\n' + indent)
}

/**
 * A value as JSON text with its real JSON type: numbers stay numbers (big integers and decimals keep every
 * digit), booleans and null stay themselves, documents are nested. undefined = leave the key out.
 * MongoDB uses Extended JSON for the types JSON lacks (ObjectId, dates, decimals, binary).
 */
function jsonText(engine: Engine, value: Value | undefined, indent: string): string | undefined {
  if (value === undefined || isDefaultMarker(value)) return undefined
  if (value === null) return 'null'
  switch (typeof value) {
    case 'boolean':
      return value ? 'true' : 'false'
    case 'number':
      return Number.isFinite(value) ? String(value) : 'null'
    case 'string':
      return JSON.stringify(value)
  }
  const mongo = engine === 'mongodb'
  if ('$int' in value) return INT_RE.test(value.$int) ? value.$int : JSON.stringify(value.$int)
  if ('$dec' in value) {
    if (mongo) return `{ "$numberDecimal": ${JSON.stringify(value.$dec)} }`
    return JSON_NUMBER_RE.test(value.$dec) ? value.$dec : JSON.stringify(value.$dec)
  }
  if ('$float' in value) return 'null' // NaN and the infinities have no JSON form
  if ('$text' in value) return JSON.stringify(value.$text)
  if ('$bytes' in value) {
    if (mongo) return `{ "$binary": { "base64": ${JSON.stringify(value.$bytes)}, "subType": "00" } }`
    return JSON.stringify('0x' + bytesToHex(base64ToBytes(value.$bytes)))
  }
  if ('$json' in value) return nested(value.$json, indent)
  if ('$date' in value) return JSON.stringify(value.$date)
  if ('$time' in value) return JSON.stringify(value.$time)
  if ('$datetime' in value) return mongo ? `{ "$date": ${JSON.stringify(value.$datetime)} }` : JSON.stringify(value.$datetime)
  if ('$uuid' in value) return JSON.stringify(value.$uuid)
  if ('$oid' in value) return mongo ? `{ "$oid": ${JSON.stringify(value.$oid)} }` : JSON.stringify(value.$oid)
  return JSON.stringify(value.$other)
}

/** Property names for the row objects; a repeated column name (SELECT a.id, b.id) gets _2, _3, ... */
function uniqueKeys(columns: ColumnMeta[]): string[] {
  const used = new Set<string>()
  return columns.map((c) => {
    let key = c.name
    for (let n = 2; used.has(key); n++) key = `${c.name}_${n}`
    used.add(key)
    return key
  })
}

/** A JSON array with one object per row. */
export function toJson(engine: Engine, columns: ColumnMeta[], rows: Row[]): string {
  if (rows.length === 0) return '[]'
  const keys = uniqueKeys(columns)
  const objects = rows.map((row) => {
    const props: string[] = []
    columns.forEach((_, i) => {
      const text = jsonText(engine, row[i], '    ')
      if (text !== undefined) props.push(`    ${JSON.stringify(keys[i])}: ${text}`)
    })
    return props.length === 0 ? '  {}' : `  {\n${props.join(',\n')}\n  }`
  })
  return `[\n${objects.join(',\n')}\n]`
}

// -- Markdown ----------------------------------------------------------------

function markdownCell(text: string): string {
  return text.replaceAll('|', '\\|').replace(/\r\n|\r|\n/g, '<br>')
}

/** A GitHub-style table. Pipes are escaped, line breaks become <br>, NULL shows as NULL. */
export function toMarkdown(columns: ColumnMeta[], rows: Row[]): string {
  const head = `| ${columns.map((c) => markdownCell(c.name)).join(' | ')} |`
  const rule = `| ${columns.map((c) => (isNumericFamily(c.family) ? '---:' : '---')).join(' | ')} |`
  const body = rows.map(
    (row) => `| ${columns.map((_, i) => (row[i] === null ? 'NULL' : markdownCell(valueToEditText(row[i])))).join(' | ')} |`,
  )
  return [head, rule, ...body].join('\n')
}

// -- Tab-separated -----------------------------------------------------------

/** A TSV field; quoted (with doubled quotes) when it holds a tab, a line break or a quote. */
export function tsvField(text: string): string {
  return /[\t\n\r"]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text
}

/**
 * A cell as copied: NULL is written as NULL, so pasting it back gives NULL again. A text value that is
 * literally "NULL" is quoted, which the paste side reads as text.
 */
export function tsvCell(value: Value | undefined): string {
  if (value === undefined) return ''
  if (value === null) return 'NULL'
  if (value === 'NULL') return '"NULL"'
  return tsvField(valueToEditText(value))
}

export function toTsv(columns: ColumnMeta[], rows: Row[], withHeader: boolean): string {
  const lines: string[] = []
  if (withHeader) lines.push(columns.map((c) => tsvField(c.name)).join('\t'))
  for (const row of rows) lines.push(columns.map((_, i) => tsvCell(row[i])).join('\t'))
  return lines.join('\n')
}

// ---------------------------------------------------------------------------
// Paste: clipboard text -> cells
// ---------------------------------------------------------------------------

/** One parsed clipboard field. `quoted` tells "NULL" (the null marker) from "\"NULL\"" (the text NULL). */
export interface ClipCell {
  text: string
  quoted: boolean
}

// The last text the app put on the clipboard itself. Such text is always tab-separated, so it is never
// second-guessed as CSV (a single column of values containing commas must not split into columns).
let ownClipboard: string | null = null

export function rememberCopied(text: string) {
  ownClipboard = text
}

export function wasCopiedHere(text: string): boolean {
  return ownClipboard !== null && ownClipboard === text
}

/** Splits delimiter-separated text into rows of fields. Quoted fields may hold the delimiter, quotes and line breaks. */
function parseDelimited(text: string, delim: string): ClipCell[][] {
  const n = text.length
  const rows: ClipCell[][] = []
  let row: ClipCell[] = []
  let i = 0

  const isBreak = (ch: string | undefined) => ch === '\n' || ch === '\r'

  for (;;) {
    let cell: ClipCell | null = null
    if (text[i] === '"') {
      let j = i + 1
      let buf = ''
      let closed = false
      while (j < n) {
        const ch = text[j]
        if (ch === '"') {
          if (text[j + 1] === '"') {
            buf += '"'
            j += 2
            continue
          }
          closed = true
          j++
          break
        }
        buf += ch
        j++
      }
      // Only a clean quoted field counts; a quote that never closes, or is followed by junk, is plain text.
      if (closed && (j >= n || text[j] === delim || isBreak(text[j]))) {
        cell = { text: buf, quoted: true }
        i = j
      }
    }
    if (!cell) {
      let j = i
      while (j < n && text[j] !== delim && !isBreak(text[j])) j++
      cell = { text: text.slice(i, j), quoted: false }
      i = j
    }
    row.push(cell)

    if (i >= n) {
      rows.push(row)
      break
    }
    if (text[i] === delim) {
      i++
      continue
    }
    // a line break ends the row (\r\n counts as one)
    i += text[i] === '\r' && text[i + 1] === '\n' ? 2 : 1
    rows.push(row)
    row = []
    if (i >= n) break // a trailing line break does not add an empty row
  }
  return rows
}

/** A number written with thousands separators ("1,234.50"): one value, not two CSV fields. */
const GROUPED_NUMBER_RE = /^[+-]?\d{1,3}(,\d{3})+(\.\d+)?$/

/** True when comma-separated parsing is plausible: every row has the same number (>= 2) of fields. */
function looksLikeCsv(rows: ClipCell[][]): boolean {
  const width = rows[0]?.length ?? 0
  if (width < 2 || !rows.every((r) => r.length === width)) return false
  if (rows.length === 1 && rows[0].every((c) => !c.quoted) && GROUPED_NUMBER_RE.test(rows[0].map((c) => c.text).join(','))) return false
  // A single line such as "Doe, John" is prose, not a CSV row: CSV does not pad its fields with spaces.
  if (rows.length === 1 && rows[0].some((c) => !c.quoted && c.text !== c.text.trim())) return false
  return true
}

export interface ParseOptions {
  /** Never fall back to CSV (the text is known to be tab-separated, for instance because the app copied it). */
  tsvOnly?: boolean
}

/**
 * Parses clipboard text as a grid. Tab-separated first (what spreadsheets and the grid's own copy produce);
 * without any tab, comma-separated when the text looks like CSV; otherwise one column.
 */
export function parseClipboard(text: string, opts: ParseOptions = {}): ClipCell[][] {
  const t = text.replace(/^\uFEFF/, '')
  if (t === '') return []
  if (t.includes('\t') || opts.tsvOnly || !t.includes(',')) return parseDelimited(t, '\t')
  const csv = parseDelimited(t, ',')
  return looksLikeCsv(csv) ? csv : parseDelimited(t, '\t')
}

/** parseClipboard() with plain strings, for callers that do not care about quoting. */
export function parseClipboardGrid(text: string, opts: ParseOptions = {}): string[][] {
  return parseClipboard(text, opts).map((row) => row.map((c) => c.text))
}

// -- one cell -> CellInput ---------------------------------------------------

export type SkipReason = 'generated' | 'notNull' | 'binary' | 'deleted' | 'notLoaded'

export const SKIP_LABEL: Record<SkipReason, string> = {
  generated: 'generated columns',
  notNull: 'NOT NULL',
  binary: 'binary columns',
  deleted: 'deleted rows',
  notLoaded: 'rows not loaded',
}

export type PasteOutcome = { input: CellInput } | { skip: SkipReason }

export interface PasteColumn {
  family: TypeFamily
  nullable: boolean
  generated?: boolean
}

/** Columns where an empty field is an empty string; for every other type it means "no value". */
const EMPTY_IS_TEXT = new Set<TypeFamily>(['text', 'other'])

function parseBool(text: string): boolean | null {
  const t = text.trim()
  if (/^(true|t|yes|y|on|1)$/i.test(t)) return true
  if (/^(false|f|no|n|off|0)$/i.test(t)) return false
  return null
}

function hexToBase64(text: string): string | null {
  const m = /^0x([0-9a-f]*)$/i.exec(text.trim())
  if (!m || m[1].length % 2 !== 0) return null
  const bytes = new Uint8Array(m[1].length / 2)
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(m[1].slice(i * 2, i * 2 + 2), 16)
  return bytesToBase64(bytes)
}

/**
 * Turns pasted text into the edit for one cell, following the rules of the inline cell editor:
 * NULL is NULL (a quoted "NULL" is text), an empty field is NULL for typed columns and '' for text,
 * booleans, JSON and bytes are converted, everything else goes through as text for the server to coerce.
 * `forInsert` is true for rows that do not exist yet, where "no value" can fall back to the column default.
 */
export function pasteInput(cell: ClipCell | string, col: PasteColumn, engine: Engine, forInsert = false): PasteOutcome {
  const { text, quoted } = typeof cell === 'string' ? { text: cell, quoted: false } : cell
  if (col.generated) return { skip: 'generated' }

  const noValue = (): PasteOutcome =>
    col.nullable ? { input: { kind: 'null' } } : forInsert ? { input: { kind: 'default' } } : { skip: 'notNull' }

  if (!quoted && text === 'NULL') return col.nullable ? { input: { kind: 'null' } } : { skip: 'notNull' }
  if (text === '') return EMPTY_IS_TEXT.has(col.family) ? { input: { kind: 'text', value: '' } } : noValue()

  switch (col.family) {
    case 'boolean': {
      const b = parseBool(text)
      if (b === null) return { input: { kind: 'text', value: text } } // not a boolean: the server reports it at commit
      if (engine === 'mongodb') return { input: { kind: 'json', value: b } }
      if (engine === 'postgres') return { input: { kind: 'text', value: b ? 'true' : 'false' } }
      return { input: { kind: 'text', value: b ? '1' : '0' } }
    }
    case 'json':
    case 'document':
      try {
        return { input: { kind: 'json', value: JSON.parse(text) as unknown } }
      } catch {
        return { input: { kind: 'text', value: text } }
      }
    case 'binary': {
      const b64 = hexToBase64(text)
      return b64 === null ? { skip: 'binary' } : { input: { kind: 'bytes', value: b64 } }
    }
    case 'integer':
    case 'float':
    case 'decimal':
    case 'date':
    case 'time':
    case 'dateTime':
    case 'uuid':
    case 'objectId':
      return { input: { kind: 'text', value: text.trim() } }
    default:
      return { input: { kind: 'text', value: text } }
  }
}

// -- the toast after a paste -------------------------------------------------

export interface PasteStats {
  pasted: number
  newRows: number
  skipped: Partial<Record<SkipReason, number>>
  /** Pasted columns that did not fit to the right of the target cell. */
  extraColumns: number
  /** Pasted rows left out because the paste was capped. */
  droppedRows?: number
}

export function pasteMessage(s: PasteStats): { text: string; kind: 'success' | 'error' } {
  const skippedTotal = Object.values(s.skipped).reduce((a, b) => a + (b ?? 0), 0)
  const parts: string[] = []
  parts.push(s.pasted === 0 ? 'Nothing was pasted.' : `Pasted ${plural(s.pasted, 'cell')}${s.newRows > 0 ? ` (${plural(s.newRows, 'new row')})` : ''}.`)
  if (skippedTotal > 0) {
    const why = (Object.keys(s.skipped) as SkipReason[])
      .filter((k) => (s.skipped[k] ?? 0) > 0)
      .map((k) => `${s.skipped[k]} ${SKIP_LABEL[k]}`)
      .join(', ')
    parts.push(`Skipped ${skippedTotal} (${why}).`)
  }
  if (s.extraColumns > 0) parts.push(`Ignored ${plural(s.extraColumns, 'extra column')} past the last column.`)
  if (s.droppedRows) parts.push(`Only the first rows were pasted, ${plural(s.droppedRows, 'row')} left out.`)
  return { text: parts.join(' '), kind: s.pasted > 0 ? 'success' : 'error' }
}
