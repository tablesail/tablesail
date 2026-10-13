import { describe, expect, it } from 'vitest'
import type { TypeFamily, Value } from '../types'
import { jsonOf, jsonTokens } from './jsonView'

describe('jsonOf', () => {
  it('returns the payload of a $json value for any family', () => {
    expect(jsonOf({ $json: { a: 1 } }, 'json')).toEqual({ a: 1 })
    expect(jsonOf({ $json: [1, 2] }, 'document')).toEqual([1, 2])
    expect(jsonOf({ $json: 'str' }, 'text')).toBe('str')
    expect(jsonOf({ $json: 0 }, 'json')).toBe(0)
    expect(jsonOf({ $json: false }, 'json')).toBe(false)
  })

  it('returns a JSON null payload as null (distinct from "not JSON")', () => {
    expect(jsonOf({ $json: null }, 'json')).toBeNull()
  })

  it('returns undefined for SQL NULL and missing cells', () => {
    expect(jsonOf(null, 'json')).toBeUndefined()
    expect(jsonOf(undefined, 'json')).toBeUndefined()
  })

  it('parses plain strings in json columns', () => {
    expect(jsonOf('{"a":[1,2,{"b":null}]}', 'json')).toEqual({ a: [1, 2, { b: null }] })
    expect(jsonOf('42', 'json')).toBe(42)
    expect(jsonOf('"x"', 'json')).toBe('x')
    expect(jsonOf('null', 'json')).toBeNull()
  })

  it('returns undefined for invalid JSON text in json columns', () => {
    expect(jsonOf('{not json', 'json')).toBeUndefined()
    expect(jsonOf('', 'json')).toBeUndefined()
    expect(jsonOf("{'a':1}", 'json')).toBeUndefined()
  })

  it('does not parse strings in non-json columns', () => {
    expect(jsonOf('{"a":1}', 'text')).toBeUndefined()
    expect(jsonOf('[1]', 'other')).toBeUndefined()
  })

  it('returns undefined for other value kinds', () => {
    const others: Value[] = [1, true, { $int: '1' }, { $text: '{"a":1}', len: 99 }, { $uuid: 'x' }, { $other: '{}', type: 'x' }]
    for (const v of others) expect(jsonOf(v, 'json')).toBeUndefined()
  })

  it('every family is accepted', () => {
    const fams: TypeFamily[] = ['json', 'document', 'text', 'other']
    for (const f of fams) expect(() => jsonOf({ $json: 1 }, f)).not.toThrow()
  })
})

const join = (text: string) => jsonTokens(text).map((t) => t.text).join('')
const classes = (text: string) => jsonTokens(text).filter((t) => t.cls).map((t) => `${t.cls}:${t.text}`)

describe('jsonTokens', () => {
  it('classifies each token kind', () => {
    expect(classes('{"a": 1, "b": [true, false, null], "c": "x"}')).toEqual([
      'punct:{',
      'key:"a"',
      'punct::',
      'num:1',
      'punct:,',
      'key:"b"',
      'punct::',
      'punct:[',
      'lit:true',
      'punct:,',
      'lit:false',
      'punct:,',
      'lit:null',
      'punct:]',
      'punct:,',
      'key:"c"',
      'punct::',
      'str:"x"',
      'punct:}',
    ])
  })

  it('numbers: negative, decimal and exponent forms', () => {
    expect(classes('[-1, 2.5, 3e10, 4E-2, -0.5e+3, 0]')).toEqual([
      'punct:[',
      'num:-1',
      'punct:,',
      'num:2.5',
      'punct:,',
      'num:3e10',
      'punct:,',
      'num:4E-2',
      'punct:,',
      'num:-0.5e+3',
      'punct:,',
      'num:0',
      'punct:]',
    ])
  })

  it('concatenates back to the input exactly (pretty-printed)', () => {
    const text = JSON.stringify({ a: [1, 2.5, { b: 'x', c: null }], d: true, e: 'str', f: {} }, null, 2)
    expect(join(text)).toBe(text)
  })

  it('concatenates back to the input exactly (compact)', () => {
    const text = JSON.stringify({ a: [1, 2, 3], s: 'hello world', n: -4.5e-7, t: true })
    expect(join(text)).toBe(text)
  })

  it('round-trips empty and whitespace-only input', () => {
    expect(jsonTokens('')).toEqual([])
    expect(jsonTokens('   \n\t')).toEqual([{ text: '   \n\t', cls: '' }])
  })

  it('round-trips text that is not JSON at all', () => {
    for (const text of ['hello world', '<script>alert(1)</script>', "it's 5 o'clock", '@@@ ###', '{{{', '"unterminated']) {
      expect(join(text)).toBe(text)
    }
  })

  it('keeps numbers inside strings as part of the string', () => {
    const toks = jsonTokens('{"id": "order-123", "n": 7}')
    expect(toks.filter((t) => t.cls === 'num').map((t) => t.text)).toEqual(['7'])
    expect(toks.find((t) => t.text === '"order-123"')?.cls).toBe('str')
  })

  it('keeps colons, brackets and commas inside strings as part of the string', () => {
    const text = '{"k": "a:b,[c]{d}"}'
    const toks = jsonTokens(text)
    expect(toks.find((t) => t.text === '"a:b,[c]{d}"')?.cls).toBe('str')
    expect(toks.filter((t) => t.cls === 'punct').map((t) => t.text)).toEqual(['{', ':', '}'])
    expect(join(text)).toBe(text)
  })

  it('keeps literals inside strings as part of the string', () => {
    const toks = jsonTokens('["true", "null", true]')
    expect(toks.filter((t) => t.cls === 'lit').map((t) => t.text)).toEqual(['true'])
    expect(toks.filter((t) => t.cls === 'str').map((t) => t.text)).toEqual(['"true"', '"null"'])
  })

  it('handles escaped quotes inside strings', () => {
    const text = '{"q": "say \\"hi\\": 1, 2"}'
    const toks = jsonTokens(text)
    expect(toks.find((t) => t.cls === 'str')?.text).toBe('"say \\"hi\\": 1, 2"')
    expect(toks.filter((t) => t.cls === 'num')).toEqual([])
    expect(join(text)).toBe(text)
  })

  it('handles a string ending in an escaped backslash', () => {
    const text = '{"p": "C:\\\\dir\\\\", "n": 1}'
    const toks = jsonTokens(text)
    expect(toks.find((t) => t.cls === 'str')?.text).toBe('"C:\\\\dir\\\\"')
    expect(toks.filter((t) => t.cls === 'num').map((t) => t.text)).toEqual(['1'])
    expect(join(text)).toBe(text)
  })

  it('handles unicode escapes and real unicode', () => {
    const text = '{"é": "\\u00e9 😀"}'
    expect(join(text)).toBe(text)
    expect(jsonTokens(text).find((t) => t.cls === 'key')?.text).toBe('"é"')
  })

  it('a string followed by a colon is a key; the colon (with its whitespace) is punctuation', () => {
    const toks = jsonTokens('{"a"  : "b"}')
    expect(toks.map((t) => `${t.cls}|${t.text}`)).toEqual(['punct|{', 'key|"a"', 'punct|  :', '|' + ' ', 'str|"b"', 'punct|}'])
  })

  it('a string value that merely contains a colon is not a key', () => {
    const toks = jsonTokens('["a:b"]')
    expect(toks.find((t) => t.text === '"a:b"')?.cls).toBe('str')
  })

  it('does not treat literal-looking words inside identifiers as literals', () => {
    expect(jsonTokens('nullable truest').filter((t) => t.cls === 'lit')).toEqual([])
  })

  it('works on deeply nested documents', () => {
    const text = JSON.stringify(JSON.parse('{"a":'.repeat(50) + '1' + '}'.repeat(50)), null, 1)
    expect(join(text)).toBe(text)
  })

  it('works on a big array (perf smoke)', () => {
    const text = JSON.stringify(Array.from({ length: 20_000 }, (_, i) => ({ i, s: `row ${i}`, ok: i % 2 === 0 })))
    expect(join(text)).toBe(text)
  })

  it('never emits an empty token', () => {
    for (const text of ['{"a":1}', '[]', '', ' ', '"x"', '1 2 3']) {
      for (const t of jsonTokens(text)) expect(t.text.length).toBeGreaterThan(0)
    }
  })
})
