// A deliberately small markdown reader for assistant replies: paragraphs, lists,
// `inline code`, **bold** and fenced code blocks. It produces plain data that the
// panel renders with ordinary text bindings, so model output is never injected as HTML.

export interface Inline {
  kind: 'text' | 'code' | 'bold'
  text: string
}

export type Block =
  | { kind: 'paragraph'; inlines: Inline[] }
  | { kind: 'list'; ordered: boolean; items: Inline[][] }
  | { kind: 'code'; lang: string; text: string }

const FENCE_OPEN = /^\s*```\s*([\w+#.-]*)[^`]*$/
const FENCE_CLOSE = /^\s*```\s*$/
const LIST_ITEM = /^\s*([-*•]|\d+[.)])\s+(.*)$/
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/

export function parseInline(text: string): Inline[] {
  const out: Inline[] = []
  // split() with a capture group puts every match at an odd index; the even parts are the
  // text between matches and must stay text even if they happen to start and end with a marker.
  text.split(/(`[^`\n]+`|\*\*[^*\n]+\*\*)/).forEach((part, i) => {
    if (!part) return
    if (i % 2 === 0) out.push({ kind: 'text', text: part })
    else if (part.startsWith('`')) out.push({ kind: 'code', text: part.slice(1, -1) })
    else out.push({ kind: 'bold', text: part.slice(2, -2) })
  })
  return out
}

export function parseMarkdown(source: string): Block[] {
  const blocks: Block[] = []
  let paragraph: string[] = []
  let list: { ordered: boolean; items: string[] } | null = null

  const flushParagraph = () => {
    if (paragraph.length) blocks.push({ kind: 'paragraph', inlines: parseInline(paragraph.join('\n')) })
    paragraph = []
  }
  const flushList = () => {
    if (list) blocks.push({ kind: 'list', ordered: list.ordered, items: list.items.map(parseInline) })
    list = null
  }
  const flush = () => {
    flushParagraph()
    flushList()
  }

  const lines = source.replace(/\r\n?/g, '\n').split('\n')
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const open = FENCE_OPEN.exec(line)
    if (open) {
      flush()
      const code: string[] = []
      for (i++; i < lines.length && !FENCE_CLOSE.test(lines[i]); i++) code.push(lines[i])
      blocks.push({ kind: 'code', lang: open[1].toLowerCase(), text: code.join('\n') })
      continue
    }
    if (!line.trim()) {
      flush()
      continue
    }
    const heading = HEADING.exec(line)
    if (heading) {
      flush()
      blocks.push({ kind: 'paragraph', inlines: [{ kind: 'bold', text: heading[1].replace(/\*\*/g, '').trim() }] })
      continue
    }
    const item = LIST_ITEM.exec(line)
    if (item) {
      flushParagraph()
      const ordered = /\d/.test(item[1])
      if (list && list.ordered !== ordered) flushList()
      list ??= { ordered, items: [] }
      list.items.push(item[2])
      continue
    }
    if (list && /^\s+/.test(line)) {
      // Continuation of the previous list item.
      list.items[list.items.length - 1] += ' ' + line.trim()
      continue
    }
    flushList()
    paragraph.push(line)
  }
  flush()
  return blocks
}
