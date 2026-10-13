// Pure geometry for the virtualised grid (no DOM, no Svelte).

export const ROW_H = 36
export const HEADER_H = 38
/** Browsers cap element heights (~16M px WebKit, ~33M Chrome); beyond this the scroll range is scaled. */
export const MAX_SCROLL_H = 8_000_000

export interface VMetrics {
  /** rowCount * ROW_H exceeds MAX_SCROLL_H: scrollTop maps proportionally to the row position. */
  scaled: boolean
  /** Largest virtual offset (px) of the first visible row. */
  virtRange: number
  /** Largest scrollTop of the scroller. */
  scrollRange: number
  /** Height of the spacer inside the scroller. */
  spacerH: number
}

export function verticalMetrics(rowCount: number, clientH: number): VMetrics {
  const bodyH = Math.max(0, clientH - HEADER_H)
  const total = rowCount * ROW_H
  const virtRange = Math.max(0, total - bodyH)
  const scaled = total > MAX_SCROLL_H
  const scrollRange = scaled ? MAX_SCROLL_H : virtRange
  return { scaled, virtRange, scrollRange, spacerH: scrollRange + clientH }
}

/** Virtual pixel offset (of the top of the body) for a scroller position. */
export function scrollToVirtual(scrollTop: number, m: VMetrics): number {
  if (m.scrollRange <= 0 || m.virtRange <= 0) return 0
  const s = Math.min(Math.max(scrollTop, 0), m.scrollRange)
  return m.scaled ? Math.floor((s * m.virtRange) / m.scrollRange) : Math.floor(s)
}

export function virtualToScroll(vt: number, m: VMetrics): number {
  if (m.scrollRange <= 0 || m.virtRange <= 0) return 0
  const v = Math.min(Math.max(vt, 0), m.virtRange)
  return m.scaled ? Math.round((v * m.scrollRange) / m.virtRange) : v
}

/** Prefix sums: result[i] is the left edge of column i, result[n] the total width. */
export function prefixSums(widths: number[]): number[] {
  const out = new Array<number>(widths.length + 1)
  let x = 0
  for (let i = 0; i < widths.length; i++) {
    out[i] = x
    x += widths[i]
  }
  out[widths.length] = x
  return out
}

/** Index of the column containing x (grid coordinates), or -1 when x is outside every column. */
export function columnAt(colLeft: number[], x: number): number {
  const n = colLeft.length - 1
  if (n <= 0 || x < 0 || x >= colLeft[n]) return -1
  let lo = 0
  let hi = n - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (colLeft[mid] <= x) lo = mid
    else hi = mid - 1
  }
  return lo
}
