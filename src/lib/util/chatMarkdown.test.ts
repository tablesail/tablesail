import { describe, expect, it } from 'vitest'
import { parseInline, parseMarkdown, type Block, type Inline } from './chatMarkdown'

const t = (text: string): Inline => ({ kind: 'text', text })
const code = (text: string): Inline => ({ kind: 'code', text })
const bold = (text: string): Inline => ({ kind: 'bold', text })
const para = (...inlines: Inline[]): Block => ({ kind: 'paragraph', inlines })
const list = (ordered: boolean, ...items: Inline[][]): Block => ({ kind: 'list', ordered, items })
const fence = (lang: string, text: string): Block => ({ kind: 'code', lang, text })

describe('parseInline', () => {
  it('plain text is a single text run', () => {
    expect(parseInline('hello world')).toEqual([t('hello world')])
  })

  it('empty input has no runs', () => {
    expect(parseInline('')).toEqual([])
  })

  it('inline code', () => {
    expect(parseInline('use `SELECT 1` now')).toEqual([t('use '), code('SELECT 1'), t(' now')])
  })

  it('bold', () => {
    expect(parseInline('a **b** c')).toEqual([t('a '), bold('b'), t(' c')])
  })

  it('several in a row', () => {
    expect(parseInline('`a` **b** `c`')).toEqual([code('a'), t(' '), bold('b'), t(' '), code('c')])
  })

  it('code and bold at the very start and end', () => {
    expect(parseInline('`a`')).toEqual([code('a')])
    expect(parseInline('**a**')).toEqual([bold('a')])
    expect(parseInline('**a**`b`')).toEqual([bold('a'), code('b')])
  })

  it('bold markers inside inline code stay literal', () => {
    expect(parseInline('`**x**`')).toEqual([code('**x**')])
  })

  it('backticks inside bold stay literal (no nesting)', () => {
    expect(parseInline('**a `b` c**')).toEqual([bold('a `b` c')])
  })

  it('unterminated markers stay literal text', () => {
    expect(parseInline('a `b')).toEqual([t('a `b')])
    expect(parseInline('a **b')).toEqual([t('a **b')])
    expect(parseInline('**')).toEqual([t('**')])
    expect(parseInline('``')).toEqual([t('``')])
    expect(parseInline('****')).toEqual([t('****')])
  })

  it('inline code and bold do not span newlines', () => {
    expect(parseInline('`a\nb`')).toEqual([t('`a\nb`')])
    expect(parseInline('**a\nb**')).toEqual([t('**a\nb**')])
  })

  it('text between matches is never re-read as code or bold (regression)', () => {
    // Starting and ending with a marker is not enough on its own:
    expect(parseInline('`x\ny`')).toEqual([t('`x\ny`')])
    expect(parseInline('**x\ny**')).toEqual([t('**x\ny**')])
    expect(parseInline('`a` and **x\ny**')).toEqual([code('a'), t(' and **x\ny**')])
  })

  it('bold that contains a lone asterisk (e.g. **COUNT(*)**) is recognised', () => {
    expect(parseInline('use **COUNT(*)** here')).toEqual([
      { kind: 'text', text: 'use ' },
      { kind: 'bold', text: 'COUNT(*)' },
      { kind: 'text', text: ' here' },
    ])
  })

  it('a single asterisk is not bold', () => {
    expect(parseInline('2 * 3 * 4')).toEqual([t('2 * 3 * 4')])
  })

  it('HTML stays plain text', () => {
    expect(parseInline('<b>x</b> <script>alert(1)</script>')).toEqual([t('<b>x</b> <script>alert(1)</script>')])
    expect(parseInline('`<img src=x onerror=alert(1)>`')).toEqual([code('<img src=x onerror=alert(1)>')])
    expect(parseInline('**<i>x</i>**')).toEqual([bold('<i>x</i>')])
  })

  it('reassembling the runs never loses characters of plain text', () => {
    const src = 'mixed `code` and **bold** and plain'
    const joined = parseInline(src)
      .map((i) => (i.kind === 'code' ? `\`${i.text}\`` : i.kind === 'bold' ? `**${i.text}**` : i.text))
      .join('')
    expect(joined).toBe(src)
  })
})

describe('parseMarkdown: paragraphs', () => {
  it('empty and whitespace-only input has no blocks', () => {
    expect(parseMarkdown('')).toEqual([])
    expect(parseMarkdown('  \n\n \t\n')).toEqual([])
  })

  it('a single paragraph', () => {
    expect(parseMarkdown('Hello there.')).toEqual([para(t('Hello there.'))])
  })

  it('blank lines separate paragraphs', () => {
    expect(parseMarkdown('one\n\ntwo\n\n\n\nthree')).toEqual([para(t('one')), para(t('two')), para(t('three'))])
  })

  it('consecutive lines join into one paragraph, keeping the newline', () => {
    expect(parseMarkdown('line one\nline two')).toEqual([para(t('line one\nline two'))])
  })

  it('normalises CRLF and lone CR line endings', () => {
    expect(parseMarkdown('a\r\n\r\nb')).toEqual([para(t('a')), para(t('b'))])
    expect(parseMarkdown('a\r\rb')).toEqual([para(t('a')), para(t('b'))])
    expect(parseMarkdown('a\r\nb')).toEqual([para(t('a\nb'))])
  })

  it('inline formatting applies inside paragraphs', () => {
    expect(parseMarkdown('Run `VACUUM` **now**')).toEqual([para(t('Run '), code('VACUUM'), t(' '), bold('now'))])
  })

  it('HTML-looking text is left as text', () => {
    const html = '<div onclick="x()">hi</div>\n<script>alert(1)</script>'
    expect(parseMarkdown(html)).toEqual([para(t(html))])
  })

  it('an HTML comment or tag line never becomes a block of its own', () => {
    expect(parseMarkdown('<!-- c -->')).toEqual([para(t('<!-- c -->'))])
    expect(parseMarkdown('<pre>\ncode\n</pre>')).toEqual([para(t('<pre>\ncode\n</pre>'))])
  })
})

describe('parseMarkdown: headings', () => {
  it('headings become bold paragraphs', () => {
    expect(parseMarkdown('# Title')).toEqual([para(bold('Title'))])
    expect(parseMarkdown('### Sub heading')).toEqual([para(bold('Sub heading'))])
    expect(parseMarkdown('###### six')).toEqual([para(bold('six'))])
  })

  it('strips ** from within a heading and trims', () => {
    expect(parseMarkdown('## **Bold** heading  ')).toEqual([para(bold('Bold heading'))])
  })

  it('a heading ends the paragraph before it and starts a new one after', () => {
    expect(parseMarkdown('intro\n# H\nbody')).toEqual([para(t('intro')), para(bold('H')), para(t('body'))])
  })

  it('# without a space is not a heading', () => {
    expect(parseMarkdown('#hashtag')).toEqual([para(t('#hashtag'))])
  })

  it('seven hashes is not a heading', () => {
    expect(parseMarkdown('####### x')).toEqual([para(t('####### x'))])
  })

  it('four spaces of indent is not a heading', () => {
    expect(parseMarkdown('    # x')).toEqual([para(t('    # x'))])
  })
})

describe('parseMarkdown: lists', () => {
  it('unordered list with -, * and •', () => {
    expect(parseMarkdown('- a\n- b\n- c')).toEqual([list(false, [t('a')], [t('b')], [t('c')])])
    expect(parseMarkdown('* a\n* b')).toEqual([list(false, [t('a')], [t('b')])])
    expect(parseMarkdown('• a\n• b')).toEqual([list(false, [t('a')], [t('b')])])
  })

  it('ordered list with 1. and 1)', () => {
    expect(parseMarkdown('1. one\n2. two\n3. three')).toEqual([list(true, [t('one')], [t('two')], [t('three')])])
    expect(parseMarkdown('1) one\n2) two')).toEqual([list(true, [t('one')], [t('two')])])
  })

  it('multi-digit numbers', () => {
    expect(parseMarkdown('10. ten\n11. eleven')).toEqual([list(true, [t('ten')], [t('eleven')])])
  })

  it('inline formatting in list items', () => {
    expect(parseMarkdown('- use `x`\n- **b**')).toEqual([list(false, [t('use '), code('x')], [bold('b')])])
  })

  it('switching list type starts a new list', () => {
    expect(parseMarkdown('- a\n1. b')).toEqual([list(false, [t('a')]), list(true, [t('b')])])
    expect(parseMarkdown('1. a\n- b')).toEqual([list(true, [t('a')]), list(false, [t('b')])])
  })

  it('a paragraph before a list is flushed first', () => {
    expect(parseMarkdown('Steps:\n- a\n- b')).toEqual([para(t('Steps:')), list(false, [t('a')], [t('b')])])
  })

  it('a non-indented line after a list starts a paragraph', () => {
    expect(parseMarkdown('- a\n- b\nafter')).toEqual([list(false, [t('a')], [t('b')]), para(t('after'))])
  })

  it('an indented line continues the previous item', () => {
    expect(parseMarkdown('- first\n  continues here\n- second')).toEqual([
      list(false, [t('first continues here')], [t('second')]),
    ])
  })

  it('a blank line ends the list', () => {
    expect(parseMarkdown('- a\n\n- b')).toEqual([list(false, [t('a')]), list(false, [t('b')])])
  })

  it('"**bold** at line start" is not a list item', () => {
    expect(parseMarkdown('**bold** text')).toEqual([para(bold('bold'), t(' text'))])
  })

  it('a hyphen without a following space is not a list item', () => {
    expect(parseMarkdown('-not a list')).toEqual([para(t('-not a list'))])
    expect(parseMarkdown('---')).toEqual([para(t('---'))])
  })

  it('"2024. text" is read as an ordered item (documented limitation of the small parser)', () => {
    expect(parseMarkdown('2024. was a year')).toEqual([list(true, [t('was a year')])])
  })

  it('HTML inside list items stays text', () => {
    expect(parseMarkdown('- <b>x</b>')).toEqual([list(false, [t('<b>x</b>')])])
  })

  it.todo('loose lists (items separated by blank lines) should render as one list with continuous numbering')
})

describe('parseMarkdown: fenced code blocks', () => {
  it('fence with a language', () => {
    expect(parseMarkdown('```sql\nSELECT 1;\n```')).toEqual([fence('sql', 'SELECT 1;')])
  })

  it('fence without a language', () => {
    expect(parseMarkdown('```\nplain\n```')).toEqual([fence('', 'plain')])
  })

  it('lower-cases the language tag', () => {
    expect(parseMarkdown('```SQL\nx\n```')).toEqual([fence('sql', 'x')])
  })

  it('accepts symbols in language names', () => {
    expect(parseMarkdown('```c++\nx\n```')[0]).toMatchObject({ lang: 'c++' })
    expect(parseMarkdown('```c#\nx\n```')[0]).toMatchObject({ lang: 'c#' })
    expect(parseMarkdown('```objective-c\nx\n```')[0]).toMatchObject({ lang: 'objective-c' })
  })

  it('allows a space between the fence and the language', () => {
    expect(parseMarkdown('``` sql\nx\n```')).toEqual([fence('sql', 'x')])
  })

  it('ignores trailing info after the language', () => {
    expect(parseMarkdown('```sql title="q"\nx\n```')[0]).toMatchObject({ kind: 'code', lang: 'sql' })
  })

  it('preserves multi-line content, indentation and blank lines verbatim', () => {
    const src = '```sql\nSELECT a,\n       b\n\nFROM t;\n```'
    expect(parseMarkdown(src)).toEqual([fence('sql', 'SELECT a,\n       b\n\nFROM t;')])
  })

  it('does not interpret markdown inside a fence', () => {
    expect(parseMarkdown('```\n# not a heading\n- not a list\n**not bold**\n```')).toEqual([
      fence('', '# not a heading\n- not a list\n**not bold**'),
    ])
  })

  it('keeps HTML inside a fence as text', () => {
    expect(parseMarkdown('```html\n<script>alert(1)</script>\n```')).toEqual([fence('html', '<script>alert(1)</script>')])
  })

  it('an empty fence gives empty text', () => {
    expect(parseMarkdown('```\n```')).toEqual([fence('', '')])
  })

  it('an unterminated fence runs to the end of the input', () => {
    expect(parseMarkdown('```sql\nSELECT 1;\nSELECT 2;')).toEqual([fence('sql', 'SELECT 1;\nSELECT 2;')])
  })

  it('an unterminated fence with nothing after it is empty', () => {
    expect(parseMarkdown('```sql')).toEqual([fence('sql', '')])
  })

  it('text before and after a fence stays separate', () => {
    expect(parseMarkdown('Before:\n```sql\nSELECT 1;\n```\nAfter.')).toEqual([
      para(t('Before:')),
      fence('sql', 'SELECT 1;'),
      para(t('After.')),
    ])
  })

  it('a fence interrupts a list', () => {
    expect(parseMarkdown('- a\n```\nx\n```\n- b')).toEqual([list(false, [t('a')]), fence('', 'x'), list(false, [t('b')])])
  })

  it('several fences in one reply', () => {
    const out = parseMarkdown('```sql\na\n```\n\ntext\n\n```js\nb\n```')
    expect(out).toEqual([fence('sql', 'a'), para(t('text')), fence('js', 'b')])
  })

  it('indented fences are recognised', () => {
    expect(parseMarkdown('  ```sql\n  x\n  ```')).toEqual([fence('sql', '  x')])
  })

  it('a line with inline triple backticks is not a fence', () => {
    for (const src of ['use ```x``` here', '```x```']) {
      const blocks = parseMarkdown(src)
      expect(blocks).toHaveLength(1)
      expect(blocks[0].kind).toBe('paragraph')
    }
  })

  it('CRLF input inside fences is normalised', () => {
    expect(parseMarkdown('```sql\r\nSELECT 1;\r\n```')).toEqual([fence('sql', 'SELECT 1;')])
  })
})

describe('parseMarkdown: realistic replies', () => {
  it('an assistant answer with prose, a list and SQL', () => {
    const reply = [
      'Here is a query:',
      '',
      '```sql',
      'SELECT status, count(*) FROM orders GROUP BY status;',
      '```',
      '',
      'Notes:',
      '1. `status` is grouped',
      '2. **row count** counts rows',
    ].join('\n')
    expect(parseMarkdown(reply)).toEqual([
      para(t('Here is a query:')),
      fence('sql', 'SELECT status, count(*) FROM orders GROUP BY status;'),
      para(t('Notes:')),
      list(true, [code('status'), t(' is grouped')], [bold('row count'), t(' counts rows')]),
    ])
  })

  it('never throws on adversarial input', () => {
    const nasty = ['`'.repeat(1000), '*'.repeat(1000), '- '.repeat(500), '```\n'.repeat(101), '\n'.repeat(1000), '\u0000\ud800', '> quote\n| a | b |\n|---|---|']
    for (const src of nasty) expect(() => parseMarkdown(src)).not.toThrow()
  })

  it('handles a large input quickly and without recursion limits', () => {
    const src = Array.from({ length: 5000 }, (_, i) => `- item ${i} with \`code\` and **bold**`).join('\n')
    const out = parseMarkdown(src)
    expect(out).toHaveLength(1)
    expect(out[0]).toMatchObject({ kind: 'list', items: expect.any(Array) })
    expect((out[0] as Extract<Block, { kind: 'list' }>).items).toHaveLength(5000)
  })
})
