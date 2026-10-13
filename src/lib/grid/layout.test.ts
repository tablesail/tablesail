import { describe, expect, it } from 'vitest'
import { HEADER_H, MAX_SCROLL_H, ROW_H, columnAt, prefixSums, scrollToVirtual, verticalMetrics, virtualToScroll } from './layout'

const CLIENT_H = 628 // 600px of body below the 28px header

describe('constants', () => {
  it('has the documented geometry', () => {
    expect(ROW_H).toBe(24)
    expect(HEADER_H).toBe(28)
    expect(MAX_SCROLL_H).toBe(8_000_000)
  })
})

describe('verticalMetrics', () => {
  it('an empty grid has nothing to scroll', () => {
    expect(verticalMetrics(0, CLIENT_H)).toEqual({ scaled: false, virtRange: 0, scrollRange: 0, spacerH: CLIENT_H })
  })

  it('fewer rows than fit on screen need no scrolling', () => {
    const m = verticalMetrics(10, CLIENT_H) // 240px < 600px body
    expect(m.virtRange).toBe(0)
    expect(m.scrollRange).toBe(0)
    expect(m.scaled).toBe(false)
  })

  it('exactly filling the body needs no scrolling', () => {
    expect(verticalMetrics(25, CLIENT_H).virtRange).toBe(0)
    expect(verticalMetrics(26, CLIENT_H).virtRange).toBe(ROW_H)
  })

  it('unscaled: the scroll range is the overflow, and the spacer adds the viewport', () => {
    const m = verticalMetrics(100, CLIENT_H)
    expect(m.scaled).toBe(false)
    expect(m.virtRange).toBe(100 * ROW_H - 600)
    expect(m.scrollRange).toBe(m.virtRange)
    expect(m.spacerH).toBe(m.scrollRange + CLIENT_H)
  })

  it('a viewport shorter than the header does not produce negative ranges', () => {
    const m = verticalMetrics(100, 10)
    expect(m.virtRange).toBe(100 * ROW_H)
    expect(m.virtRange).toBeGreaterThanOrEqual(0)
  })

  it('a zero-height viewport is safe', () => {
    expect(() => verticalMetrics(1000, 0)).not.toThrow()
    expect(verticalMetrics(0, 0).spacerH).toBe(0)
  })

  it('switches to scaled mode exactly when the content exceeds MAX_SCROLL_H', () => {
    const limit = Math.floor(MAX_SCROLL_H / ROW_H) // 333,333 rows = 7,999,992px
    expect(verticalMetrics(limit, CLIENT_H).scaled).toBe(false)
    expect(verticalMetrics(limit + 1, CLIENT_H).scaled).toBe(true) // 8,000,016px
  })

  it('scaled: scroll range is capped, virtual range is the real overflow', () => {
    const m = verticalMetrics(1_000_000, CLIENT_H)
    expect(m.scaled).toBe(true)
    expect(m.virtRange).toBe(1_000_000 * ROW_H - 600)
    expect(m.scrollRange).toBe(MAX_SCROLL_H)
    expect(m.spacerH).toBe(MAX_SCROLL_H + CLIENT_H)
  })

  it('the spacer never exceeds the browser element-height limits, however many rows', () => {
    for (const rows of [1e6, 1e7, 1e8, 1e9]) {
      expect(verticalMetrics(rows, 1200).spacerH).toBeLessThan(16_000_000)
    }
  })
})

describe('scrollToVirtual / virtualToScroll: unscaled', () => {
  const m = verticalMetrics(10_000, CLIENT_H)

  it('is the identity (floored)', () => {
    expect(scrollToVirtual(0, m)).toBe(0)
    expect(scrollToVirtual(1234, m)).toBe(1234)
    expect(scrollToVirtual(1234.9, m)).toBe(1234)
    expect(virtualToScroll(1234, m)).toBe(1234)
  })

  it('reaches both ends', () => {
    expect(scrollToVirtual(m.scrollRange, m)).toBe(m.virtRange)
    expect(virtualToScroll(m.virtRange, m)).toBe(m.scrollRange)
  })

  it('clamps out-of-range input', () => {
    expect(scrollToVirtual(-50, m)).toBe(0)
    expect(scrollToVirtual(m.scrollRange + 1000, m)).toBe(m.virtRange)
    expect(virtualToScroll(-50, m)).toBe(0)
    expect(virtualToScroll(m.virtRange + 1000, m)).toBe(m.scrollRange)
  })

  it('round-trips integers exactly', () => {
    for (const v of [0, 1, 24, 999, 100_000, m.virtRange - 1, m.virtRange]) {
      expect(scrollToVirtual(virtualToScroll(v, m), m)).toBe(v)
    }
  })
})

describe('scrollToVirtual / virtualToScroll: nothing to scroll', () => {
  it('returns 0 for any input when the content fits', () => {
    const m = verticalMetrics(3, CLIENT_H)
    for (const x of [-10, 0, 5, 1e9]) {
      expect(scrollToVirtual(x, m)).toBe(0)
      expect(virtualToScroll(x, m)).toBe(0)
    }
  })

  it('returns 0 for an empty grid', () => {
    const m = verticalMetrics(0, CLIENT_H)
    expect(scrollToVirtual(100, m)).toBe(0)
    expect(virtualToScroll(100, m)).toBe(0)
  })
})

describe('scrollToVirtual / virtualToScroll: scaled mode (1M+ rows)', () => {
  const rows = 1_000_000
  const m = verticalMetrics(rows, CLIENT_H)
  const ratio = m.virtRange / m.scrollRange // ~3 virtual px per scroll px

  it('is actually scaled', () => {
    expect(m.scaled).toBe(true)
    expect(ratio).toBeGreaterThan(2.9)
    expect(ratio).toBeLessThan(3.1)
  })

  it('top of the scroller is the first row', () => {
    expect(scrollToVirtual(0, m)).toBe(0)
    expect(virtualToScroll(0, m)).toBe(0)
  })

  it('bottom of the scroller is exactly the last page of rows', () => {
    expect(scrollToVirtual(m.scrollRange, m)).toBe(m.virtRange)
    expect(virtualToScroll(m.virtRange, m)).toBe(m.scrollRange)
    // the first visible row at the very bottom leaves exactly one viewport of rows
    const firstRow = Math.floor(scrollToVirtual(m.scrollRange, m) / ROW_H)
    expect(firstRow).toBe(Math.floor((rows * ROW_H - 600) / ROW_H))
    expect(rows - firstRow).toBeGreaterThanOrEqual(600 / ROW_H)
  })

  it('the middle of the scroller is the middle of the data', () => {
    const mid = scrollToVirtual(m.scrollRange / 2, m)
    expect(mid / m.virtRange).toBeCloseTo(0.5, 5)
    expect(Math.floor(mid / ROW_H)).toBeGreaterThan(rows / 2 - 30)
    expect(Math.floor(mid / ROW_H)).toBeLessThan(rows / 2 + 30)
  })

  it('clamps out-of-range input', () => {
    expect(scrollToVirtual(-1, m)).toBe(0)
    expect(scrollToVirtual(m.scrollRange * 2, m)).toBe(m.virtRange)
    expect(virtualToScroll(-1, m)).toBe(0)
    expect(virtualToScroll(m.virtRange * 2, m)).toBe(m.scrollRange)
  })

  it('scrollToVirtual is monotonic non-decreasing over a sweep', () => {
    let prev = -1
    for (let s = 0; s <= m.scrollRange; s += 9973) {
      const v = scrollToVirtual(s, m)
      expect(v).toBeGreaterThanOrEqual(prev)
      prev = v
    }
    expect(scrollToVirtual(m.scrollRange, m)).toBeGreaterThanOrEqual(prev)
  })

  it('each scroll pixel moves by about `ratio` virtual pixels (no big jumps)', () => {
    for (const s of [0, 1, 1000, 4_000_000, m.scrollRange - 1]) {
      const step = scrollToVirtual(s + 1, m) - scrollToVirtual(s, m)
      expect(step).toBeGreaterThanOrEqual(Math.floor(ratio) - 1)
      expect(step).toBeLessThanOrEqual(Math.ceil(ratio) + 1)
    }
  })

  it('virtualToScroll is monotonic non-decreasing over a sweep', () => {
    let prev = -1
    for (let v = 0; v <= m.virtRange; v += 99_991) {
      const s = virtualToScroll(v, m)
      expect(s).toBeGreaterThanOrEqual(prev)
      prev = s
    }
  })

  it('virtual -> scroll -> virtual lands within one scroll pixel of where it started', () => {
    const tolerance = Math.ceil(ratio / 2) + 1
    for (const v of [0, 1, 24, 12_345, 5_000_000, 12_000_000, m.virtRange - 24, m.virtRange]) {
      const back = scrollToVirtual(virtualToScroll(v, m), m)
      expect(Math.abs(back - v)).toBeLessThanOrEqual(tolerance)
    }
  })

  it('scroll -> virtual -> scroll is stable to within one pixel', () => {
    for (const s of [0, 1, 777, 3_999_999, 4_000_000, m.scrollRange - 1, m.scrollRange]) {
      expect(Math.abs(virtualToScroll(scrollToVirtual(s, m), m) - s)).toBeLessThanOrEqual(1)
    }
  })

  it('scrollbar position stays strictly inside the range at interior points', () => {
    const s = virtualToScroll(m.virtRange / 2, m)
    expect(s).toBeGreaterThan(0)
    expect(s).toBeLessThan(m.scrollRange)
  })

  it('handles 100M rows', () => {
    const big = verticalMetrics(100_000_000, CLIENT_H)
    expect(big.scaled).toBe(true)
    expect(scrollToVirtual(big.scrollRange, big)).toBe(big.virtRange)
    expect(Number.isSafeInteger(big.virtRange)).toBe(true)
    expect(scrollToVirtual(big.scrollRange / 2, big)).toBe(Math.floor(big.virtRange / 2))
  })

  it('just past the threshold the mapping is nearly 1:1 and still reaches both ends', () => {
    const edge = verticalMetrics(Math.floor(MAX_SCROLL_H / ROW_H) + 1, CLIENT_H)
    expect(edge.scaled).toBe(true)
    expect(scrollToVirtual(0, edge)).toBe(0)
    expect(scrollToVirtual(edge.scrollRange, edge)).toBe(edge.virtRange)
    expect(edge.virtRange / edge.scrollRange).toBeCloseTo(1, 3)
  })
})

describe('prefixSums', () => {
  it('computes left edges plus the total', () => {
    expect(prefixSums([100, 50, 25])).toEqual([0, 100, 150, 175])
  })
  it('empty widths give [0]', () => {
    expect(prefixSums([])).toEqual([0])
  })
  it('a single column', () => {
    expect(prefixSums([80])).toEqual([0, 80])
  })
  it('zero-width columns repeat the edge', () => {
    expect(prefixSums([10, 0, 10])).toEqual([0, 10, 10, 20])
  })
  it('does not modify its input', () => {
    const w = [1, 2, 3]
    prefixSums(w)
    expect(w).toEqual([1, 2, 3])
  })
})

describe('columnAt', () => {
  const edges = prefixSums([100, 50, 25]) // [0, 100, 150, 175]

  it('finds the column containing x', () => {
    expect(columnAt(edges, 0)).toBe(0)
    expect(columnAt(edges, 99)).toBe(0)
    expect(columnAt(edges, 100)).toBe(1)
    expect(columnAt(edges, 149)).toBe(1)
    expect(columnAt(edges, 150)).toBe(2)
    expect(columnAt(edges, 174.9)).toBe(2)
  })

  it('returns -1 outside every column', () => {
    expect(columnAt(edges, -0.1)).toBe(-1)
    expect(columnAt(edges, 175)).toBe(-1)
    expect(columnAt(edges, 9999)).toBe(-1)
  })

  it('returns -1 with no columns', () => {
    expect(columnAt([0], 0)).toBe(-1)
    expect(columnAt([], 0)).toBe(-1)
  })

  it('works for one column', () => {
    const one = prefixSums([40])
    expect(columnAt(one, 0)).toBe(0)
    expect(columnAt(one, 39)).toBe(0)
    expect(columnAt(one, 40)).toBe(-1)
  })

  it('skips zero-width columns', () => {
    const e = prefixSums([10, 0, 10]) // [0, 10, 10, 20]
    expect(columnAt(e, 9)).toBe(0)
    expect(columnAt(e, 10)).toBe(2)
  })

  it('agrees with a linear scan on many columns', () => {
    const widths = Array.from({ length: 500 }, (_, i) => 20 + (i % 7) * 13)
    const e = prefixSums(widths)
    const linear = (x: number) => {
      for (let i = 0; i < widths.length; i++) if (x >= e[i] && x < e[i + 1]) return i
      return -1
    }
    for (let x = -5; x < e[e.length - 1] + 5; x += 7) expect(columnAt(e, x)).toBe(linear(x))
  })
})
