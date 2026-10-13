// Cell formatting shared by the data grid, the table view and the SQL result grid.
//
// formatValue() is on the hot path of every grid repaint: it returns shared
// constants for the common cases and memoises tagged objects per Value instance.

import type { TypeFamily, Value } from './types'

export interface Formatted {
  text: string
  /** Space-separated CSS classes understood by DataGrid: num, null, missing, mono, bool (+ yes/no), trunc. */
  cls: string
}

/** Longest text put into a grid cell; a cell is never wider than ~400px anyway. */
const MAX_CELL_CHARS = 300
/** Bytes shown as hex in a cell. */
const BYTES_PREVIEW = 16

/** Marker for "use the column default" on inserted rows (see changes.svelte.ts). */
export const DEFAULT_MARKER_TYPE = '$default'

const NULL_CELL: Formatted = Object.freeze({ text: 'NULL', cls: 'null' })
const MISSING_CELL: Formatted = Object.freeze({ text: '', cls: 'missing' })
const TRUE_CELL: Formatted = Object.freeze({ text: 'true', cls: 'bool yes' })
const FALSE_CELL: Formatted = Object.freeze({ text: 'false', cls: 'bool no' })
const DEFAULT_CELL: Formatted = Object.freeze({ text: 'DEFAULT', cls: 'null' })

const objectCache = new WeakMap<object, Formatted>()

export function isNumericFamily(family?: TypeFamily): boolean {
  return family === 'integer' || family === 'float' || family === 'decimal'
}

function clip(text: string): string {
  return text.length > MAX_CELL_CHARS ? text.slice(0, MAX_CELL_CHARS) + '…' : text
}

export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

export function bytesToBase64(bytes: Uint8Array): string {
  let bin = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk))
  }
  return btoa(bin)
}

export function bytesToHex(bytes: Uint8Array): string {
  let out = ''
  for (let i = 0; i < bytes.length; i++) out += bytes[i].toString(16).padStart(2, '0')
  return out
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let u = 0
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024
    u++
  }
  const digits = (x: number) => (x >= 100 ? 0 : 1)
  // Rounding can land on 1024 of this unit (1 MB - 1 byte): show it as 1.0 of the next one.
  if (Number(v.toFixed(digits(v))) >= 1024 && u < units.length - 1) {
    v /= 1024
    u++
  }
  return `${v.toFixed(digits(v))} ${units[u]}`
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US')
}

/** True when the value holds only a prefix of the stored data (fetch the full cell to see all of it). */
export function isTruncatedValue(value: Value | undefined): boolean {
  if (value === null || value === undefined || typeof value !== 'object') return false
  if ('$text' in value) return true
  if ('$bytes' in value) {
    const b = value.$bytes
    const pad = b.endsWith('==') ? 2 : b.endsWith('=') ? 1 : 0
    return value.len > (b.length * 3) / 4 - pad
  }
  return false
}

function formatObject(value: Exclude<Value, null | boolean | number | string>): Formatted {
  if ('$int' in value) return { text: value.$int, cls: 'num' }
  if ('$dec' in value) return { text: clip(value.$dec), cls: 'num' }
  if ('$float' in value) return { text: value.$float, cls: 'num' }
  if ('$text' in value) return { text: value.$text.slice(0, MAX_CELL_CHARS) + '…', cls: 'trunc' }
  if ('$bytes' in value) {
    const head = base64ToBytes(value.$bytes.slice(0, 24)).subarray(0, BYTES_PREVIEW)
    const more = value.len > head.length
    return {
      text: `0x${bytesToHex(head)}${more ? '…' : ''} (${formatBytes(value.len)})`,
      cls: 'mono',
    }
  }
  if ('$json' in value) return { text: clip(JSON.stringify(value.$json) ?? 'null'), cls: '' }
  if ('$date' in value) return { text: value.$date, cls: '' }
  if ('$time' in value) return { text: value.$time, cls: '' }
  if ('$datetime' in value) return { text: value.$datetime, cls: '' }
  if ('$uuid' in value) return { text: value.$uuid, cls: 'mono' }
  if ('$oid' in value) return { text: value.$oid, cls: 'mono' }
  if (value.type === DEFAULT_MARKER_TYPE) return DEFAULT_CELL
  return { text: clip(value.$other), cls: '' }
}

export function formatValue(value: Value | undefined, family?: TypeFamily): Formatted {
  if (value === undefined) return MISSING_CELL
  if (value === null) return NULL_CELL
  switch (typeof value) {
    case 'boolean':
      return value ? TRUE_CELL : FALSE_CELL
    case 'number':
      return { text: String(value), cls: 'num' }
    case 'string':
      return {
        text: clip(value),
        cls: isNumericFamily(family) ? 'num' : family === 'uuid' ? 'mono' : '',
      }
  }
  const hit = objectCache.get(value)
  if (hit) return hit
  const out = formatObject(value)
  objectCache.set(value, out)
  return out
}

/** The text an inline editor starts with (and what copy puts on the clipboard). */
export function valueToEditText(value: Value | undefined): string {
  if (value === null || value === undefined) return ''
  switch (typeof value) {
    case 'boolean':
    case 'number':
      return String(value)
    case 'string':
      return value
  }
  if ('$int' in value) return value.$int
  if ('$dec' in value) return value.$dec
  if ('$float' in value) return value.$float
  if ('$text' in value) return value.$text
  if ('$bytes' in value) return '0x' + bytesToHex(base64ToBytes(value.$bytes))
  if ('$json' in value) return JSON.stringify(value.$json) ?? ''
  if ('$date' in value) return value.$date
  if ('$time' in value) return value.$time
  if ('$datetime' in value) return value.$datetime
  if ('$uuid' in value) return value.$uuid
  if ('$oid' in value) return value.$oid
  return value.type === DEFAULT_MARKER_TYPE ? '' : value.$other
}

/**
 * Colour family for a status-like enum value, so the grid can show it as a badge:
 * good (green), warn (amber), bad (red), info (blue) or neutral.
 */
export function statusTone(text: string): 'good' | 'warn' | 'bad' | 'info' | 'neutral' {
  const t = text.toLowerCase()
  if (/^(active|paid|ok|success|succeeded|done|complete|completed|approved|enabled|open|online|happy|yes|healthy|live|published|okay)$/.test(t)) return 'good'
  if (/^(pending|paused|waiting|draft|queued|processing|review|trial|warning)$/.test(t)) return 'warn'
  if (/^(failed|error|refunded|cancel(l)?ed|rejected|blocked|banned|deleted|disabled|closed|offline|sad|expired|overdue)$/.test(t)) return 'bad'
  if (/^(shipped|sent|new|scheduled|running|in[_ ]progress|delivered)$/.test(t)) return 'info'
  return 'neutral'
}
