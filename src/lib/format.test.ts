import { describe, expect, it } from 'vitest'
import {
  DEFAULT_MARKER_TYPE,
  base64ToBytes,
  bytesToBase64,
  bytesToHex,
  formatBytes,
  formatCount,
  formatValue,
  isNumericFamily,
  isTruncatedValue,
  statusTone,
  valueToEditText,
} from './format'
import type { Value } from './types'

const b64 = (bytes: number[]) => bytesToBase64(Uint8Array.from(bytes))

describe('formatValue: scalars', () => {
  it('renders undefined as an empty "missing" cell', () => {
    expect(formatValue(undefined)).toEqual({ text: '', cls: 'missing' })
  })

  it('renders null as NULL', () => {
    expect(formatValue(null)).toEqual({ text: 'NULL', cls: 'null' })
  })

  it('distinguishes undefined from null even with a numeric family', () => {
    expect(formatValue(undefined, 'integer').cls).toBe('missing')
    expect(formatValue(null, 'integer').cls).toBe('null')
  })

  it('renders booleans with yes/no classes', () => {
    expect(formatValue(true)).toEqual({ text: 'true', cls: 'bool yes' })
    expect(formatValue(false)).toEqual({ text: 'false', cls: 'bool no' })
  })

  it('renders JS numbers right-aligned', () => {
    expect(formatValue(42)).toEqual({ text: '42', cls: 'num' })
    expect(formatValue(-0.5)).toEqual({ text: '-0.5', cls: 'num' })
    expect(formatValue(0)).toEqual({ text: '0', cls: 'num' })
  })

  it('renders plain strings according to the column family', () => {
    expect(formatValue('hello')).toEqual({ text: 'hello', cls: '' })
    expect(formatValue('12', 'integer').cls).toBe('num')
    expect(formatValue('1.5', 'float').cls).toBe('num')
    expect(formatValue('1.5', 'decimal').cls).toBe('num')
    expect(formatValue('abc', 'uuid').cls).toBe('mono')
    expect(formatValue('abc', 'text').cls).toBe('')
    expect(formatValue('', 'text')).toEqual({ text: '', cls: '' })
  })

  it('returns shared constants for the common cases', () => {
    expect(formatValue(null)).toBe(formatValue(null))
    expect(formatValue(true)).toBe(formatValue(true))
    expect(formatValue(undefined)).toBe(formatValue(undefined))
  })

  it('shared constants are frozen', () => {
    expect(Object.isFrozen(formatValue(null))).toBe(true)
    expect(Object.isFrozen(formatValue(undefined))).toBe(true)
    expect(Object.isFrozen(formatValue(true))).toBe(true)
    expect(Object.isFrozen(formatValue(false))).toBe(true)
  })
})

describe('formatValue: long text clipping', () => {
  it('leaves text up to 300 chars alone', () => {
    const s = 'a'.repeat(300)
    expect(formatValue(s).text).toBe(s)
  })

  it('clips longer strings to 300 chars plus an ellipsis', () => {
    const out = formatValue('a'.repeat(301)).text
    expect(out).toBe('a'.repeat(300) + '…')
    expect(out.length).toBe(301)
  })

  it('clips a huge string', () => {
    expect(formatValue('x'.repeat(100_000)).text.length).toBe(301)
  })

  it('clips $dec and $other the same way', () => {
    const long = '9'.repeat(400)
    expect(formatValue({ $dec: long }).text).toBe('9'.repeat(300) + '…')
    expect(formatValue({ $other: long, type: 'geometry' }).text).toBe('9'.repeat(300) + '…')
  })

  it('clips serialised JSON', () => {
    const out = formatValue({ $json: { v: 'z'.repeat(500) } }).text
    expect(out.length).toBe(301)
    expect(out.endsWith('…')).toBe(true)
    expect(out.startsWith('{"v":"zzz')).toBe(true)
  })
})

describe('formatValue: tagged encodings', () => {
  it('$int is a num with the exact digits (beyond 2^53)', () => {
    expect(formatValue({ $int: '9223372036854775807' })).toEqual({ text: '9223372036854775807', cls: 'num' })
  })

  it('$dec keeps its scale', () => {
    expect(formatValue({ $dec: '12.3400' })).toEqual({ text: '12.3400', cls: 'num' })
  })

  it('$float special values render as num', () => {
    expect(formatValue({ $float: 'NaN' })).toEqual({ text: 'NaN', cls: 'num' })
    expect(formatValue({ $float: 'Infinity' })).toEqual({ text: 'Infinity', cls: 'num' })
    expect(formatValue({ $float: '-Infinity' })).toEqual({ text: '-Infinity', cls: 'num' })
  })

  it('$text (a truncated prefix) shows the prefix, an ellipsis and the trunc class', () => {
    expect(formatValue({ $text: 'abcdef', len: 100000 })).toEqual({ text: 'abcdef…', cls: 'trunc' })
  })

  it('$text longer than the cell limit is cut at 300 chars', () => {
    const out = formatValue({ $text: 'q'.repeat(1000), len: 5000 })
    expect(out.text).toBe('q'.repeat(300) + '…')
    expect(out.cls).toBe('trunc')
  })

  it('$json renders compact JSON', () => {
    expect(formatValue({ $json: { a: [1, 2], b: null } })).toEqual({ text: '{"a":[1,2],"b":null}', cls: '' })
    expect(formatValue({ $json: 'str' }).text).toBe('"str"')
    expect(formatValue({ $json: 5 }).text).toBe('5')
    expect(formatValue({ $json: null }).text).toBe('null')
    expect(formatValue({ $json: [] }).text).toBe('[]')
  })

  it('$json holding undefined falls back to "null" rather than throwing', () => {
    expect(formatValue({ $json: undefined }).text).toBe('null')
  })

  it('$date, $time and $datetime pass through', () => {
    expect(formatValue({ $date: '2024-02-29' })).toEqual({ text: '2024-02-29', cls: '' })
    expect(formatValue({ $time: '13:45:10.5' })).toEqual({ text: '13:45:10.5', cls: '' })
    expect(formatValue({ $datetime: '2024-02-29T13:45:10Z' })).toEqual({ text: '2024-02-29T13:45:10Z', cls: '' })
  })

  it('$uuid and $oid are monospaced', () => {
    expect(formatValue({ $uuid: '123e4567-e89b-12d3-a456-426614174000' })).toEqual({
      text: '123e4567-e89b-12d3-a456-426614174000',
      cls: 'mono',
    })
    expect(formatValue({ $oid: '507f1f77bcf86cd799439011' })).toEqual({ text: '507f1f77bcf86cd799439011', cls: 'mono' })
  })

  it('$other passes through its text', () => {
    expect(formatValue({ $other: 'POINT(1 2)', type: 'geometry' })).toEqual({ text: 'POINT(1 2)', cls: '' })
  })

  it('the $default marker renders as DEFAULT, styled like NULL', () => {
    expect(formatValue({ $other: 'DEFAULT', type: DEFAULT_MARKER_TYPE })).toEqual({ text: 'DEFAULT', cls: 'null' })
  })

  it('memoises the result per object instance', () => {
    const v: Value = { $int: '7' }
    expect(formatValue(v)).toBe(formatValue(v))
    expect(formatValue({ $int: '7' })).not.toBe(formatValue(v))
    expect(formatValue({ $int: '7' })).toEqual(formatValue(v))
  })
})

describe('formatValue: $bytes', () => {
  it('shows hex and the byte size for a short blob', () => {
    const v: Value = { $bytes: b64([0xde, 0xad, 0xbe, 0xef]), len: 4 }
    expect(formatValue(v)).toEqual({ text: '0xdeadbeef (4 B)', cls: 'mono' })
  })

  it('previews at most 16 bytes and marks the rest with an ellipsis', () => {
    const bytes = Array.from({ length: 32 }, (_, i) => i)
    const out = formatValue({ $bytes: b64(bytes), len: 32 })
    expect(out.text).toBe('0x' + '000102030405060708090a0b0c0d0e0f' + '… (32 B)')
    expect(out.cls).toBe('mono')
  })

  it('exactly 16 bytes has no ellipsis', () => {
    const bytes = Array.from({ length: 16 }, (_, i) => i)
    expect(formatValue({ $bytes: b64(bytes), len: 16 }).text).toBe('0x000102030405060708090a0b0c0d0e0f (16 B)')
  })

  it('reports the real length when the payload is only a prefix', () => {
    const out = formatValue({ $bytes: b64([1, 2, 3, 4]), len: 5 * 1024 * 1024 })
    expect(out.text).toBe('0x01020304… (5.0 MB)')
  })

  it('empty blob', () => {
    expect(formatValue({ $bytes: '', len: 0 }).text).toBe('0x (0 B)')
  })

  it('a 1-byte blob (padded base64)', () => {
    expect(formatValue({ $bytes: b64([255]), len: 1 }).text).toBe('0xff (1 B)')
  })
})

describe('base64 / hex helpers', () => {
  it('round-trips arbitrary bytes', () => {
    const bytes = Uint8Array.from({ length: 256 }, (_, i) => i)
    expect(Array.from(base64ToBytes(bytesToBase64(bytes)))).toEqual(Array.from(bytes))
  })

  it('round-trips a buffer larger than the internal chunk size', () => {
    const bytes = Uint8Array.from({ length: 0x8000 * 2 + 17 }, (_, i) => (i * 7) & 0xff)
    expect(Array.from(base64ToBytes(bytesToBase64(bytes)))).toEqual(Array.from(bytes))
  })

  it('encodes known vectors', () => {
    expect(bytesToBase64(new TextEncoder().encode('Man'))).toBe('TWFu')
    expect(bytesToBase64(new TextEncoder().encode('Ma'))).toBe('TWE=')
    expect(bytesToBase64(new TextEncoder().encode('M'))).toBe('TQ==')
    expect(bytesToBase64(new Uint8Array())).toBe('')
  })

  it('bytesToHex zero-pads', () => {
    expect(bytesToHex(Uint8Array.from([0, 1, 15, 16, 255]))).toBe('00010f10ff')
    expect(bytesToHex(new Uint8Array())).toBe('')
  })

  it('base64ToBytes rejects invalid input', () => {
    expect(() => base64ToBytes('***')).toThrow()
  })
})

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [1, '1 B'],
    [1023, '1023 B'],
    [1024, '1.0 KB'],
    [1536, '1.5 KB'],
    [102_400, '100 KB'],
    [1024 * 1024, '1.0 MB'],
    [5 * 1024 * 1024, '5.0 MB'],
    [1024 ** 3, '1.0 GB'],
    [1024 ** 4, '1.0 TB'],
    [1024 ** 5, '1024 TB'],
  ])('%d -> %s', (n, expected) => {
    expect(formatBytes(n)).toBe(expected)
  })

  it('rolls over to the next unit instead of printing 1024 of the previous one', () => {
    expect(formatBytes(1024 * 1024 - 1)).toBe('1.0 MB')
    expect(formatBytes(1024 ** 3 - 1)).toBe('1.0 GB')
  })
})

describe('formatCount', () => {
  it('groups thousands with commas', () => {
    expect(formatCount(0)).toBe('0')
    expect(formatCount(999)).toBe('999')
    expect(formatCount(1000)).toBe('1,000')
    expect(formatCount(1234567)).toBe('1,234,567')
    expect(formatCount(-1234)).toBe('-1,234')
  })
})

describe('isNumericFamily', () => {
  it('is true only for integer, float and decimal', () => {
    expect(isNumericFamily('integer')).toBe(true)
    expect(isNumericFamily('float')).toBe(true)
    expect(isNumericFamily('decimal')).toBe(true)
    expect(isNumericFamily('text')).toBe(false)
    expect(isNumericFamily('boolean')).toBe(false)
    expect(isNumericFamily(undefined)).toBe(false)
  })
})

describe('isTruncatedValue', () => {
  it('is false for scalars, null and undefined', () => {
    for (const v of [null, undefined, true, 1, 'text'] as (Value | undefined)[]) expect(isTruncatedValue(v)).toBe(false)
  })

  it('$text is always a truncated prefix', () => {
    expect(isTruncatedValue({ $text: 'abc', len: 3 })).toBe(true)
  })

  it('$bytes is truncated only when it holds fewer bytes than len', () => {
    expect(isTruncatedValue({ $bytes: b64([1, 2, 3, 4]), len: 4 })).toBe(false)
    expect(isTruncatedValue({ $bytes: b64([1, 2, 3, 4]), len: 5 })).toBe(true)
    // padded base64: 'TQ==' is one byte, 'TWE=' is two
    expect(isTruncatedValue({ $bytes: 'TQ==', len: 1 })).toBe(false)
    expect(isTruncatedValue({ $bytes: 'TQ==', len: 2 })).toBe(true)
    expect(isTruncatedValue({ $bytes: 'TWE=', len: 2 })).toBe(false)
    expect(isTruncatedValue({ $bytes: 'TWE=', len: 3 })).toBe(true)
    expect(isTruncatedValue({ $bytes: '', len: 0 })).toBe(false)
  })

  it('other tagged values are complete', () => {
    expect(isTruncatedValue({ $int: '1' })).toBe(false)
    expect(isTruncatedValue({ $json: {} })).toBe(false)
    expect(isTruncatedValue({ $uuid: 'x' })).toBe(false)
  })
})

describe('valueToEditText', () => {
  it('null and undefined become empty text', () => {
    expect(valueToEditText(null)).toBe('')
    expect(valueToEditText(undefined)).toBe('')
  })

  it('scalars', () => {
    expect(valueToEditText(true)).toBe('true')
    expect(valueToEditText(false)).toBe('false')
    expect(valueToEditText(12.5)).toBe('12.5')
    expect(valueToEditText('abc')).toBe('abc')
    expect(valueToEditText('')).toBe('')
  })

  it('tagged values', () => {
    expect(valueToEditText({ $int: '123' })).toBe('123')
    expect(valueToEditText({ $dec: '1.50' })).toBe('1.50')
    expect(valueToEditText({ $float: 'NaN' })).toBe('NaN')
    expect(valueToEditText({ $text: 'prefix', len: 99 })).toBe('prefix')
    expect(valueToEditText({ $bytes: b64([0xca, 0xfe]), len: 2 })).toBe('0xcafe')
    expect(valueToEditText({ $json: { a: 1 } })).toBe('{"a":1}')
    expect(valueToEditText({ $json: undefined })).toBe('')
    expect(valueToEditText({ $date: '2024-01-02' })).toBe('2024-01-02')
    expect(valueToEditText({ $time: '10:00:00' })).toBe('10:00:00')
    expect(valueToEditText({ $datetime: '2024-01-02T10:00:00' })).toBe('2024-01-02T10:00:00')
    expect(valueToEditText({ $uuid: 'u' })).toBe('u')
    expect(valueToEditText({ $oid: 'o' })).toBe('o')
    expect(valueToEditText({ $other: 'POINT(1 2)', type: 'geometry' })).toBe('POINT(1 2)')
  })

  it('the DEFAULT marker edits as empty text', () => {
    expect(valueToEditText({ $other: 'DEFAULT', type: DEFAULT_MARKER_TYPE })).toBe('')
  })

  it('full-length bytes are not clipped (unlike the cell preview)', () => {
    const bytes = Array.from({ length: 40 }, (_, i) => i)
    expect(valueToEditText({ $bytes: b64(bytes), len: 40 })).toBe('0x' + bytesToHex(Uint8Array.from(bytes)))
  })
})

describe('statusTone', () => {
  it.each([
    'active', 'paid', 'ok', 'success', 'succeeded', 'done', 'complete', 'completed', 'approved',
    'enabled', 'open', 'online', 'happy', 'yes', 'healthy', 'live', 'published',
  ])('%s is good', (s) => expect(statusTone(s)).toBe('good'))

  it.each(['pending', 'paused', 'waiting', 'draft', 'queued', 'processing', 'review', 'trial', 'warning'])(
    '%s is warn',
    (s) => expect(statusTone(s)).toBe('warn'),
  )

  it.each([
    'failed', 'error', 'refunded', 'canceled', 'cancelled', 'rejected', 'blocked', 'banned',
    'deleted', 'disabled', 'closed', 'offline', 'sad', 'expired', 'overdue',
  ])('%s is bad', (s) => expect(statusTone(s)).toBe('bad'))

  it.each(['shipped', 'sent', 'new', 'scheduled', 'running', 'in_progress', 'in progress', 'delivered'])(
    '%s is info',
    (s) => expect(statusTone(s)).toBe('info'),
  )

  it('is case-insensitive', () => {
    expect(statusTone('ACTIVE')).toBe('good')
    expect(statusTone('Pending')).toBe('warn')
    expect(statusTone('FAILED')).toBe('bad')
    expect(statusTone('Shipped')).toBe('info')
  })

  it('is neutral for anything else, including substrings and padded values', () => {
    expect(statusTone('')).toBe('neutral')
    expect(statusTone('banana')).toBe('neutral')
    expect(statusTone('inactive')).toBe('neutral')
    expect(statusTone('not paid')).toBe('neutral')
    expect(statusTone(' active')).toBe('neutral')
    expect(statusTone('active ')).toBe('neutral')
    expect(statusTone('in-progress')).toBe('neutral')
  })

  it('"okay" is as good as "ok" (regression: a stray `ok?ay` pattern used to make it warn)', () => {
    expect(statusTone('okay')).toBe('good')
    expect(statusTone('ok')).toBe('good')
    expect(statusTone('oay')).toBe('neutral')
  })
})
