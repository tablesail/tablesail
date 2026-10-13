// Pretty-printing and token colouring for JSON values in the Details panel.

import type { TypeFamily, Value } from '../types'

/** The parsed JSON behind a cell, or undefined when the cell is not JSON. */
export function jsonOf(v: Value | undefined, family: TypeFamily): unknown {
  if (v === null || v === undefined) return undefined
  if (typeof v === 'object' && '$json' in v) return v.$json
  // Some engines (MySQL JSON as text, SQLite) hand JSON over as a plain string.
  if (family === 'json' && typeof v === 'string') {
    try {
      return JSON.parse(v)
    } catch {
      return undefined
    }
  }
  return undefined
}

export interface JsonToken {
  text: string
  cls: '' | 'key' | 'str' | 'num' | 'lit' | 'punct'
}

const TOKEN = /("(?:\\.|[^"\\])*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b|([{}[\],])/g

/** Splits JSON text into coloured tokens (rendered as text, never as HTML). */
export function jsonTokens(text: string): JsonToken[] {
  const out: JsonToken[] = []
  let last = 0
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0
    if (at > last) out.push({ text: text.slice(last, at), cls: '' })
    if (m[1] !== undefined) {
      out.push({ text: m[1], cls: m[2] ? 'key' : 'str' })
      if (m[2]) out.push({ text: m[2], cls: 'punct' })
    } else if (m[3] !== undefined) out.push({ text: m[3], cls: 'num' })
    else if (m[4] !== undefined) out.push({ text: m[4], cls: 'lit' })
    else out.push({ text: m[0], cls: 'punct' })
    last = at + m[0].length
  }
  if (last < text.length) out.push({ text: text.slice(last), cls: '' })
  return out
}
