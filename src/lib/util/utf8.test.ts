import { describe, expect, it } from 'vitest'
import { byteLength, byteToUtf16, bytesToUtf16Many, utf16ToByte } from './utf8'

// 1-, 2-, 3- and 4-byte UTF-8 characters (the last is a surrogate pair in UTF-16).
const MIXED = 'aé€😀z'

describe('byteLength', () => {
  it('matches TextEncoder for each width', () => {
    expect(byteLength('')).toBe(0)
    expect(byteLength('a')).toBe(1)
    expect(byteLength('é')).toBe(2)
    expect(byteLength('€')).toBe(3)
    expect(byteLength('😀')).toBe(4)
    expect(byteLength(MIXED)).toBe(1 + 2 + 3 + 4 + 1)
  })
})

describe('utf16ToByte', () => {
  it('is the identity for ASCII', () => {
    for (let i = 0; i <= 5; i++) expect(utf16ToByte('hello', i)).toBe(i)
  })

  it('counts multi-byte characters', () => {
    expect(utf16ToByte(MIXED, 0)).toBe(0)
    expect(utf16ToByte(MIXED, 1)).toBe(1) // after a
    expect(utf16ToByte(MIXED, 2)).toBe(3) // after é
    expect(utf16ToByte(MIXED, 3)).toBe(6) // after €
    expect(utf16ToByte(MIXED, 5)).toBe(10) // after the surrogate pair (2 UTF-16 units)
    expect(utf16ToByte(MIXED, 6)).toBe(11)
  })

  it('clamps an index past the end', () => {
    expect(utf16ToByte(MIXED, 999)).toBe(11)
    expect(utf16ToByte('', 5)).toBe(0)
  })

  it('treats a negative index as 0', () => {
    expect(utf16ToByte('abc', -3)).toBe(0)
  })

  it('full length equals byteLength for any string', () => {
    for (const s of ['', 'abc', MIXED, '日本語', '👨‍👩‍👧‍👦', 'a\u0000b', '\n\r\n']) {
      expect(utf16ToByte(s, s.length)).toBe(byteLength(s))
    }
  })

  it('a lone surrogate is counted like TextEncoder does (U+FFFD, 3 bytes)', () => {
    const lone = '\ud83d'
    expect(utf16ToByte(lone, 1)).toBe(byteLength(lone))
  })
})

describe('byteToUtf16', () => {
  it('is the identity for ASCII', () => {
    for (let i = 0; i <= 5; i++) expect(byteToUtf16('hello', i)).toBe(i)
  })

  it('maps byte boundaries back to UTF-16 indices', () => {
    expect(byteToUtf16(MIXED, 0)).toBe(0)
    expect(byteToUtf16(MIXED, 1)).toBe(1)
    expect(byteToUtf16(MIXED, 3)).toBe(2)
    expect(byteToUtf16(MIXED, 6)).toBe(3)
    expect(byteToUtf16(MIXED, 10)).toBe(5)
    expect(byteToUtf16(MIXED, 11)).toBe(6)
  })

  it('clamps to the text length', () => {
    expect(byteToUtf16(MIXED, 9999)).toBe(MIXED.length)
    expect(byteToUtf16('', 3)).toBe(0)
  })

  it('a byte offset inside a character rounds up to the end of that character', () => {
    expect(byteToUtf16('é', 1)).toBe(1)
    expect(byteToUtf16('€', 1)).toBe(1)
    expect(byteToUtf16('€', 2)).toBe(1)
    expect(byteToUtf16('😀', 2)).toBe(2) // never splits a surrogate pair
  })

  it('negative offset is 0', () => {
    expect(byteToUtf16('abc', -1)).toBe(0)
  })

  it('round-trips every character boundary', () => {
    const s = 'SELECT "名前", 😀 FROM t WHERE c = \'é€\';\n-- ✓'
    for (let i = 0; i <= s.length; i++) {
      // Indices between the two halves of a surrogate pair are not character boundaries.
      const prev = i > 0 ? s.charCodeAt(i - 1) : 0
      if (prev >= 0xd800 && prev <= 0xdbff) continue
      expect(byteToUtf16(s, utf16ToByte(s, i))).toBe(i)
    }
  })
})

describe('bytesToUtf16Many', () => {
  it('converts ascending offsets in one pass', () => {
    expect(bytesToUtf16Many(MIXED, [0, 1, 3, 6, 10, 11])).toEqual([0, 1, 2, 3, 5, 6])
  })

  it('agrees with byteToUtf16 for each offset', () => {
    const s = 'a€b😀c日本語\nxyz'
    const total = byteLength(s)
    const offsets = Array.from({ length: total + 1 }, (_, i) => i)
    expect(bytesToUtf16Many(s, offsets)).toEqual(offsets.map((b) => byteToUtf16(s, b)))
  })

  it('handles repeated and empty input', () => {
    expect(bytesToUtf16Many('abc', [])).toEqual([])
    expect(bytesToUtf16Many('abc', [1, 1, 1])).toEqual([1, 1, 1])
  })

  it('clamps offsets beyond the end', () => {
    expect(bytesToUtf16Many('é', [0, 2, 50])).toEqual([0, 1, 1])
  })
})
