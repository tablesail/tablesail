// The backend reports statement offsets as UTF-8 byte offsets; CodeMirror positions are
// UTF-16 code unit indices. These helpers convert between the two for a given text.

const encoder = new TextEncoder()

function utf8Len(codePoint: number): number {
  if (codePoint < 0x80) return 1
  if (codePoint < 0x800) return 2
  if (codePoint < 0x10000) return 3
  return 4
}

/** UTF-8 byte offset of the UTF-16 index `index` in `text`. */
export function utf16ToByte(text: string, index: number): number {
  let bytes = 0
  const end = Math.min(index, text.length)
  for (let i = 0; i < end; ) {
    const cp = text.codePointAt(i)!
    bytes += utf8Len(cp)
    i += cp > 0xffff ? 2 : 1
  }
  return bytes
}

/** UTF-16 index of the UTF-8 byte offset `byte` in `text` (clamped to the text length). */
export function byteToUtf16(text: string, byte: number): number {
  let bytes = 0
  let i = 0
  while (i < text.length && bytes < byte) {
    const cp = text.codePointAt(i)!
    bytes += utf8Len(cp)
    i += cp > 0xffff ? 2 : 1
  }
  return i
}

export function byteLength(text: string): number {
  return encoder.encode(text).length
}

/** Converts many byte offsets in one pass; `bytes` must be ascending. */
export function bytesToUtf16Many(text: string, bytes: number[]): number[] {
  const out: number[] = []
  let b = 0
  let i = 0
  for (const target of bytes) {
    while (i < text.length && b < target) {
      const cp = text.codePointAt(i)!
      b += utf8Len(cp)
      i += cp > 0xffff ? 2 : 1
    }
    out.push(i)
  }
  return out
}
