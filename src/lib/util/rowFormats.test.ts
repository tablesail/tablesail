import { describe, expect, it } from 'vitest'
import { DEFAULT_MARKER_TYPE, bytesToBase64 } from '../format'
import type { ColumnMeta, Engine, Row, TableRef, TypeFamily, Value } from '../types'
import {
  PLACEHOLDER_TABLE,
  availableFormats,
  copyMessage,
  countTruncated,
  formatRows,
  parseClipboard,
  parseClipboardGrid,
  pasteInput,
  pasteMessage,
  quoteString,
  rememberCopied,
  sqlLiteral,
  toInsertStatements,
  toJson,
  toMarkdown,
  toTsv,
  tsvCell,
  tsvField,
  wasCopiedHere,
} from './rowFormats'

const SQL_ENGINES: Engine[] = ['postgres', 'mysql', 'mariadb', 'sqlite']
const col = (name: string, family: TypeFamily = 'text'): ColumnMeta => ({ name, typeName: family, family })
const table = (name: string, schema: string | null = null, database = 'app'): TableRef => ({ database, schema, name })
const bytes = (...b: number[]) => ({ $bytes: bytesToBase64(Uint8Array.from(b)), len: b.length }) as Value
const DEFAULT: Value = { $other: 'DEFAULT', type: DEFAULT_MARKER_TYPE }

// ---------------------------------------------------------------------------
// SQL literals
// ---------------------------------------------------------------------------

describe('quoteString', () => {
  it('doubles single quotes on every engine', () => {
    for (const e of SQL_ENGINES) expect(quoteString(e, "it's")).toBe("'it''s'")
  })

  it('leaves backslashes alone where they are ordinary characters', () => {
    expect(quoteString('postgres', 'C:\\temp\\n')).toBe("'C:\\temp\\n'")
    expect(quoteString('sqlite', 'a\\b')).toBe("'a\\b'")
  })

  it('doubles backslashes for MySQL and MariaDB, where they escape', () => {
    expect(quoteString('mysql', 'C:\\temp')).toBe("'C:\\\\temp'")
    expect(quoteString('mariadb', 'a\\')).toBe("'a\\\\'")
    // a backslash right before a quote must not swallow the doubled quote
    expect(quoteString('mysql', "\\'")).toBe("'\\\\'''")
  })

  it('writes NUL and Ctrl-Z as escapes for MySQL only', () => {
    expect(quoteString('mysql', 'a\0b\x1ac')).toBe("'a\\0b\\Zc'")
    expect(quoteString('postgres', 'a\x1ac')).toBe("'a\x1ac'")
  })

  it('keeps unicode, emoji and line breaks as they are', () => {
    for (const e of SQL_ENGINES) {
      expect(quoteString(e, 'héllo wörld 日本語 🚀')).toBe("'héllo wörld 日本語 🚀'")
      expect(quoteString(e, 'line1\nline2\ttab')).toBe("'line1\nline2\ttab'")
    }
  })

  it('quotes injection attempts so they stay one string', () => {
    expect(quoteString('postgres', "'; DROP TABLE users; --")).toBe("'''; DROP TABLE users; --'")
    expect(quoteString('mysql', "\\'; DROP TABLE users; --")).toBe("'\\\\''; DROP TABLE users; --'")
  })
})

describe('sqlLiteral', () => {
  it('writes NULL', () => {
    for (const e of SQL_ENGINES) expect(sqlLiteral(e, null)).toBe('NULL')
  })

  it('leaves numbers bare', () => {
    for (const e of SQL_ENGINES) {
      expect(sqlLiteral(e, 42)).toBe('42')
      expect(sqlLiteral(e, -1.5)).toBe('-1.5')
      expect(sqlLiteral(e, 0)).toBe('0')
      expect(sqlLiteral(e, { $int: '9223372036854775807' })).toBe('9223372036854775807')
      expect(sqlLiteral(e, { $dec: '12.50' })).toBe('12.50')
      expect(sqlLiteral(e, { $dec: '-0.001' })).toBe('-0.001')
    }
  })

  it('writes booleans as TRUE/FALSE, or 1/0 for SQLite', () => {
    for (const e of ['postgres', 'mysql', 'mariadb'] as Engine[]) {
      expect(sqlLiteral(e, true)).toBe('TRUE')
      expect(sqlLiteral(e, false)).toBe('FALSE')
    }
    expect(sqlLiteral('sqlite', true)).toBe('1')
    expect(sqlLiteral('sqlite', false)).toBe('0')
  })

  it('quotes a decimal that is not a number', () => {
    expect(sqlLiteral('postgres', { $dec: 'NaN' })).toBe("'NaN'")
  })

  it('handles NaN and the infinities per engine', () => {
    expect(sqlLiteral('postgres', { $float: 'NaN' })).toBe("'NaN'")
    expect(sqlLiteral('postgres', { $float: '-Infinity' })).toBe("'-Infinity'")
    expect(sqlLiteral('sqlite', { $float: 'Infinity' })).toBe('9e999')
    expect(sqlLiteral('sqlite', { $float: '-Infinity' })).toBe('-9e999')
    expect(sqlLiteral('sqlite', { $float: 'NaN' })).toBe('NULL')
    expect(sqlLiteral('mysql', { $float: 'Infinity' })).toBe('NULL')
    expect(sqlLiteral('mariadb', { $float: 'NaN' })).toBe('NULL')
  })

  it('quotes strings, even ones that look like numbers, unless the column is numeric', () => {
    expect(sqlLiteral('postgres', '007')).toBe("'007'")
    expect(sqlLiteral('postgres', '007', 'text')).toBe("'007'")
    expect(sqlLiteral('sqlite', '12.5', 'decimal')).toBe('12.5')
    expect(sqlLiteral('sqlite', 'abc', 'integer')).toBe("'abc'")
    expect(sqlLiteral('sqlite', '1; DROP TABLE t', 'integer')).toBe("'1; DROP TABLE t'")
  })

  it('writes bytes as a hex literal in the engine\'s syntax', () => {
    expect(sqlLiteral('postgres', bytes(0xde, 0xad, 0xbe, 0xef))).toBe("'\\xdeadbeef'")
    expect(sqlLiteral('mysql', bytes(0xde, 0xad, 0xbe, 0xef))).toBe("X'deadbeef'")
    expect(sqlLiteral('mariadb', bytes(1, 2))).toBe("X'0102'")
    expect(sqlLiteral('sqlite', bytes(0, 255))).toBe("X'00ff'")
    expect(sqlLiteral('postgres', bytes())).toBe("'\\x'")
    expect(sqlLiteral('sqlite', bytes())).toBe("X''")
  })

  it('writes JSON as a quoted string', () => {
    expect(sqlLiteral('postgres', { $json: { a: [1, "it's"] } })).toBe('\'{"a":[1,"it\'\'s"]}\'')
    expect(sqlLiteral('mysql', { $json: { path: 'C:\\x' } })).toBe('\'{"path":"C:\\\\\\\\x"}\'')
    expect(sqlLiteral('sqlite', { $json: null })).toBe("'null'")
  })

  it('quotes dates, times, uuids, object ids and unknown types', () => {
    expect(sqlLiteral('postgres', { $date: '2024-01-31' })).toBe("'2024-01-31'")
    expect(sqlLiteral('postgres', { $time: '10:11:12' })).toBe("'10:11:12'")
    expect(sqlLiteral('postgres', { $datetime: '2024-01-31 10:11:12+00' })).toBe("'2024-01-31 10:11:12+00'")
    expect(sqlLiteral('postgres', { $uuid: '123e4567-e89b-12d3-a456-426614174000' })).toBe("'123e4567-e89b-12d3-a456-426614174000'")
    expect(sqlLiteral('postgres', { $oid: '507f1f77bcf86cd799439011' })).toBe("'507f1f77bcf86cd799439011'")
    expect(sqlLiteral('postgres', { $other: '{1,2,3}', type: '_int4' })).toBe("'{1,2,3}'")
  })

  it('uses the loaded prefix of a truncated text', () => {
    expect(sqlLiteral('postgres', { $text: 'abc…', len: 5000 })).toBe("'abc…'")
  })

  it('writes the default marker as DEFAULT', () => {
    expect(sqlLiteral('postgres', DEFAULT)).toBe('DEFAULT')
  })
})

// ---------------------------------------------------------------------------
// INSERT statements
// ---------------------------------------------------------------------------

describe('toInsertStatements', () => {
  const cols = [col('id', 'integer'), col('name'), col('active', 'boolean')]
  const rows: Row[] = [
    [1, 'Alice', true],
    [2, null, false],
  ]

  it('writes one statement per row with a column list', () => {
    expect(toInsertStatements('postgres', table('users', 'public'), cols, rows)).toBe(
      [
        `INSERT INTO "public"."users" ("id", "name", "active") VALUES (1, 'Alice', TRUE);`,
        `INSERT INTO "public"."users" ("id", "name", "active") VALUES (2, NULL, FALSE);`,
      ].join('\n'),
    )
  })

  it('quotes identifiers the way each engine does', () => {
    const t = table('my table', 'sch', 'db')
    expect(toInsertStatements('postgres', t, [col('a"b')], [[1]])).toBe(`INSERT INTO "sch"."my table" ("a""b") VALUES (1);`)
    expect(toInsertStatements('mysql', t, [col('a`b')], [[1]])).toBe('INSERT INTO `db`.`my table` (`a``b`) VALUES (1);')
    expect(toInsertStatements('mariadb', t, [col('x')], [[1]])).toBe('INSERT INTO `db`.`my table` (`x`) VALUES (1);')
    expect(toInsertStatements('sqlite', t, [col('x y')], [[1]])).toBe('INSERT INTO "my table" ("x y") VALUES (1);')
  })

  it('uses 1/0 for booleans in SQLite and escapes backslashes for MySQL', () => {
    expect(toInsertStatements('sqlite', table('t'), [col('b', 'boolean')], [[true]])).toBe(`INSERT INTO "t" ("b") VALUES (1);`)
    expect(toInsertStatements('mysql', table('t'), [col('p')], [['C:\\x']])).toBe('INSERT INTO `app`.`t` (`p`) VALUES (\'C:\\\\x\');')
  })

  it('uses a placeholder table name for query results', () => {
    expect(toInsertStatements('postgres', null, cols, [rows[0]])).toBe(
      `INSERT INTO ${PLACEHOLDER_TABLE} ("id", "name", "active") VALUES (1, 'Alice', TRUE);`,
    )
    expect(toInsertStatements('mysql', null, [col('a')], [[1]])).toBe('INSERT INTO table_name (`a`) VALUES (1);')
  })

  it('escapes quotes and keeps unicode and line breaks', () => {
    expect(toInsertStatements('postgres', table('t', 'public'), [col('s')], [["O'Brien 🚀\nnext"]])).toBe(
      `INSERT INTO "public"."t" ("s") VALUES ('O''Brien 🚀\nnext');`,
    )
  })

  it('writes json, bytes, decimals and big integers sensibly', () => {
    const c = [col('j', 'json'), col('b', 'binary'), col('d', 'decimal'), col('n', 'integer')]
    const r: Row[] = [[{ $json: { k: 'v' } }, bytes(1, 2, 3), { $dec: '3.14' }, { $int: '12345678901234567890' }]]
    expect(toInsertStatements('postgres', table('t', 'public'), c, r)).toBe(
      `INSERT INTO "public"."t" ("j", "b", "d", "n") VALUES ('{"k":"v"}', '\\x010203', 3.14, 12345678901234567890);`,
    )
    expect(toInsertStatements('mysql', table('t'), c, r)).toBe(
      'INSERT INTO `app`.`t` (`j`, `b`, `d`, `n`) VALUES (\'{"k":"v"}\', X\'010203\', 3.14, 12345678901234567890);',
    )
  })

  it('leaves default-marker columns out of the statement', () => {
    expect(toInsertStatements('postgres', table('t', 'public'), cols, [[DEFAULT, 'x', DEFAULT]])).toBe(
      `INSERT INTO "public"."t" ("name") VALUES ('x');`,
    )
  })

  it('writes DEFAULT VALUES (or () VALUES ()) when every column is a default', () => {
    expect(toInsertStatements('postgres', table('t', 'public'), [col('a')], [[DEFAULT]])).toBe(`INSERT INTO "public"."t" DEFAULT VALUES;`)
    expect(toInsertStatements('sqlite', table('t'), [col('a')], [[DEFAULT]])).toBe(`INSERT INTO "t" DEFAULT VALUES;`)
    expect(toInsertStatements('mysql', table('t'), [col('a')], [[DEFAULT]])).toBe('INSERT INTO `app`.`t` () VALUES ();')
  })

  it('writes nothing for no rows', () => {
    expect(toInsertStatements('postgres', table('t'), cols, [])).toBe('')
  })
})

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

describe('toJson', () => {
  it('writes an array of objects with real JSON types', () => {
    const cols = [col('id', 'integer'), col('name'), col('ok', 'boolean'), col('note'), col('price', 'decimal')]
    const text = toJson('postgres', cols, [[1, 'Alice', true, null, { $dec: '12.50' }]])
    expect(JSON.parse(text)).toEqual([{ id: 1, name: 'Alice', ok: true, note: null, price: 12.5 }])
    expect(text).toContain('"price": 12.50') // the digits are kept as they were
    expect(text).toContain('"note": null')
    expect(text).toContain('"ok": true')
  })

  it('is pretty printed', () => {
    expect(toJson('sqlite', [col('a'), col('b')], [[1, 2], [3, 4]])).toBe(
      ['[', '  {', '    "a": 1,', '    "b": 2', '  },', '  {', '    "a": 3,', '    "b": 4', '  }', ']'].join('\n'),
    )
  })

  it('keeps numbers as numbers and does not lose big integers', () => {
    const text = toJson('postgres', [col('n', 'integer')], [[{ $int: '9223372036854775807' }]])
    expect(text).toContain('"n": 9223372036854775807')
    expect(text).not.toContain('"9223372036854775807"')
  })

  it('turns decimals that are not JSON numbers into strings', () => {
    const text = toJson('postgres', [col('d', 'decimal')], [[{ $dec: 'NaN' }], [{ $dec: '.5' }], [{ $dec: '007' }]])
    expect(JSON.parse(text)).toEqual([{ d: 'NaN' }, { d: '.5' }, { d: '007' }])
  })

  it('uses null for NaN and the infinities, which JSON cannot hold', () => {
    const text = toJson('postgres', [col('f', 'float')], [[{ $float: 'NaN' }], [{ $float: 'Infinity' }]])
    expect(JSON.parse(text)).toEqual([{ f: null }, { f: null }])
  })

  it('nests JSON documents instead of quoting them', () => {
    const text = toJson('postgres', [col('id', 'integer'), col('doc', 'json')], [[1, { $json: { tags: ['a', 'b'], n: { x: 1 } } }]])
    expect(JSON.parse(text)).toEqual([{ id: 1, doc: { tags: ['a', 'b'], n: { x: 1 } } }])
  })

  it('escapes quotes, control characters and unicode correctly', () => {
    const value = 'say "hi"\n\ttab \\ back 日本語 🚀'
    expect(JSON.parse(toJson('postgres', [col('s')], [[value]]))).toEqual([{ s: value }])
  })

  it('writes dates, uuids and bytes as strings', () => {
    const cols = [col('d', 'date'), col('t', 'dateTime'), col('u', 'uuid'), col('b', 'binary')]
    const rows: Row[] = [[{ $date: '2024-01-31' }, { $datetime: '2024-01-31 10:00:00+00' }, { $uuid: 'aaaa' }, bytes(0xca, 0xfe)]]
    expect(JSON.parse(toJson('mysql', cols, rows))).toEqual([{ d: '2024-01-31', t: '2024-01-31 10:00:00+00', u: 'aaaa', b: '0xcafe' }])
  })

  it('renames repeated column names so no value is lost', () => {
    const text = toJson('postgres', [col('id'), col('name'), col('id'), col('id')], [[1, 'a', 2, 3]])
    expect(JSON.parse(text)).toEqual([{ id: 1, name: 'a', id_2: 2, id_3: 3 }])
  })

  it('leaves out default-marker columns', () => {
    expect(JSON.parse(toJson('postgres', [col('a'), col('b')], [[1, DEFAULT]]))).toEqual([{ a: 1 }])
  })

  it('writes [] for no rows and {} for a row of only defaults', () => {
    expect(toJson('postgres', [col('a')], [])).toBe('[]')
    expect(JSON.parse(toJson('postgres', [col('a')], [[DEFAULT]]))).toEqual([{}])
  })

  it('uses Extended JSON for MongoDB types JSON lacks', () => {
    const cols = [col('_id', 'objectId'), col('at', 'dateTime'), col('amount', 'decimal'), col('doc', 'document'), col('n', 'integer')]
    const rows: Row[] = [
      [{ $oid: '507f1f77bcf86cd799439011' }, { $datetime: '2024-05-01T12:34:56.789Z' }, { $dec: '1.5' }, { $json: { a: 1 } }, 7],
    ]
    expect(JSON.parse(toJson('mongodb', cols, rows))).toEqual([
      {
        _id: { $oid: '507f1f77bcf86cd799439011' },
        at: { $date: '2024-05-01T12:34:56.789Z' },
        amount: { $numberDecimal: '1.5' },
        doc: { a: 1 },
        n: 7,
      },
    ])
  })
})

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

describe('toMarkdown', () => {
  it('writes a header, a rule and one line per row', () => {
    expect(toMarkdown([col('id', 'integer'), col('name')], [[1, 'Alice'], [2, 'Bob']])).toBe(
      ['| id | name |', '| ---: | --- |', '| 1 | Alice |', '| 2 | Bob |'].join('\n'),
    )
  })

  it('escapes pipes in values and in headers', () => {
    expect(toMarkdown([col('a|b')], [['x|y||z']])).toBe(['| a\\|b |', '| --- |', '| x\\|y\\|\\|z |'].join('\n'))
  })

  it('turns line breaks into <br>', () => {
    expect(toMarkdown([col('t')], [['one\ntwo\r\nthree\rfour']])).toBe(['| t |', '| --- |', '| one<br>two<br>three<br>four |'].join('\n'))
  })

  it('shows NULL as NULL and an empty string as empty', () => {
    expect(toMarkdown([col('a'), col('b')], [[null, '']])).toBe(['| a | b |', '| --- | --- |', '| NULL |  |'].join('\n'))
  })

  it('keeps unicode and formats typed values like the grid copy does', () => {
    const rows: Row[] = [['日本語 🚀', { $json: { a: 1 } }, bytes(1, 2), true, { $dec: '1.50' }]]
    const cols = [col('s'), col('j', 'json'), col('b', 'binary'), col('t', 'boolean'), col('d', 'decimal')]
    expect(toMarkdown(cols, rows).split('\n')[2]).toBe('| 日本語 🚀 | {"a":1} | 0x0102 | true | 1.50 |')
  })

  it('writes just the header and rule for no rows', () => {
    expect(toMarkdown([col('a')], [])).toBe(['| a |', '| --- |'].join('\n'))
  })
})

// ---------------------------------------------------------------------------
// TSV
// ---------------------------------------------------------------------------

describe('tab-separated copy', () => {
  it('quotes fields with tabs, line breaks or quotes', () => {
    expect(tsvField('plain')).toBe('plain')
    expect(tsvField('a\tb')).toBe('"a\tb"')
    expect(tsvField('a\nb')).toBe('"a\nb"')
    expect(tsvField('say "hi"')).toBe('"say ""hi"""')
  })

  it('writes NULL as NULL and the text NULL quoted', () => {
    expect(tsvCell(null)).toBe('NULL')
    expect(tsvCell('NULL')).toBe('"NULL"')
    expect(tsvCell('')).toBe('')
    expect(tsvCell(undefined)).toBe('')
    expect(tsvCell(true)).toBe('true')
    expect(tsvCell({ $dec: '1.50' })).toBe('1.50')
  })

  it('joins rows with newlines and optionally starts with the headers', () => {
    const cols = [col('id', 'integer'), col('name')]
    expect(toTsv(cols, [[1, 'a'], [2, null]], false)).toBe('1\ta\n2\tNULL')
    expect(toTsv(cols, [[1, 'a']], true)).toBe('id\tname\n1\ta')
  })
})

describe('formatRows / availableFormats / copyMessage', () => {
  it('dispatches by format', () => {
    const input = { engine: 'postgres' as Engine, table: table('t', 'public'), columns: [col('a')], rows: [[1]] as Row[] }
    expect(formatRows('insert', input)).toContain('INSERT INTO "public"."t"')
    expect(formatRows('json', input)).toBe('[\n  {\n    "a": 1\n  }\n]')
    expect(formatRows('markdown', input)).toBe('| a |\n| --- |\n| 1 |')
    expect(formatRows('tsv', input)).toBe('1')
  })

  it('offers JSON and plain text only for MongoDB', () => {
    expect(availableFormats('mongodb')).toEqual(['json', 'tsv'])
    for (const e of SQL_ENGINES) expect(availableFormats(e)).toEqual(['insert', 'json', 'markdown', 'tsv'])
  })

  it('counts truncated cells', () => {
    expect(countTruncated([[1, { $text: 'abc', len: 100 }, { $bytes: 'AAAA', len: 100 }, 'x']])).toBe(2)
    expect(countTruncated([[1, 'x']])).toBe(0)
  })

  it('describes what was copied', () => {
    expect(copyMessage('json', { rows: 1, skipped: 0, truncated: 0 })).toEqual({ text: 'Copied 1 row as JSON.', kind: 'success' })
    expect(copyMessage('insert', { rows: 3, skipped: 2, truncated: 1 }).text).toBe(
      'Copied 3 rows as INSERT statements. 2 rows not loaded, left out. 1 long value cut short.',
    )
    expect(copyMessage('markdown', { rows: 0, skipped: 4, truncated: 0 }).kind).toBe('error')
  })

  it('says when rows were left out because of the copy limit', () => {
    expect(copyMessage('json', { rows: 25000, skipped: 0, truncated: 0, capped: 75000 }).text).toBe(
      'Copied 25000 rows as JSON. 75000 rows over the copy limit, left out.',
    )
    expect(copyMessage('json', { rows: 5, skipped: 0, truncated: 0, capped: 1 }).text).toContain('1 row over the copy limit')
    expect(copyMessage('json', { rows: 5, skipped: 0, truncated: 0, capped: 0 }).text).not.toContain('limit')
  })

  it('reports an error instead of success when the clipboard refused the text', () => {
    expect(copyMessage('json', { rows: 2, skipped: 0, truncated: 0, capped: 0, written: false })).toEqual({
      text: 'Could not copy to the clipboard.',
      kind: 'error',
    })
    expect(copyMessage('json', { rows: 2, skipped: 0, truncated: 0, capped: 0, written: true }).kind).toBe('success')
  })
})

// ---------------------------------------------------------------------------
// Paste: parsing the clipboard
// ---------------------------------------------------------------------------

describe('parseClipboardGrid', () => {
  it('splits tab-separated rows', () => {
    expect(parseClipboardGrid('a\tb\tc\n1\t2\t3')).toEqual([['a', 'b', 'c'], ['1', '2', '3']])
  })

  it('accepts \\r\\n and \\r line breaks and ignores one trailing line break', () => {
    expect(parseClipboardGrid('a\tb\r\nc\td\r\n')).toEqual([['a', 'b'], ['c', 'd']])
    expect(parseClipboardGrid('a\rb\r')).toEqual([['a'], ['b']])
    expect(parseClipboardGrid('a\n')).toEqual([['a']])
  })

  it('keeps empty fields and blank lines', () => {
    expect(parseClipboardGrid('a\t\tc')).toEqual([['a', '', 'c']])
    expect(parseClipboardGrid('\tb')).toEqual([['', 'b']])
    expect(parseClipboardGrid('a\t')).toEqual([['a', '']])
    expect(parseClipboardGrid('a\n\nb')).toEqual([['a'], [''], ['b']])
  })

  it('returns nothing for empty text and one empty cell for a lone line break', () => {
    expect(parseClipboardGrid('')).toEqual([])
    expect(parseClipboardGrid('\n')).toEqual([['']])
  })

  it('reads one cell from a single value', () => {
    expect(parseClipboardGrid('hello world')).toEqual([['hello world']])
  })

  it('strips a byte order mark', () => {
    expect(parseClipboardGrid('\uFEFFa\tb')).toEqual([['a', 'b']])
  })

  it('handles quoted fields with tabs, line breaks and doubled quotes', () => {
    expect(parseClipboardGrid('"a\tb"\tc')).toEqual([['a\tb', 'c']])
    expect(parseClipboardGrid('"line1\nline2"\tx\n"y"\tz')).toEqual([['line1\nline2', 'x'], ['y', 'z']])
    expect(parseClipboardGrid('"say ""hi"""\tx')).toEqual([['say "hi"', 'x']])
    expect(parseClipboardGrid('"a\r\nb"')).toEqual([['a\r\nb']])
    expect(parseClipboardGrid('""\tx')).toEqual([['', 'x']])
  })

  it('treats a stray quote as text', () => {
    expect(parseClipboardGrid('5" pipe\tx')).toEqual([['5" pipe', 'x']])
    expect(parseClipboardGrid('"unterminated\tx')).toEqual([['"unterminated', 'x']])
    expect(parseClipboardGrid('"abc"def\tx')).toEqual([['"abc"def', 'x']])
  })

  it('keeps ragged rows ragged', () => {
    expect(parseClipboardGrid('a\tb\tc\nd')).toEqual([['a', 'b', 'c'], ['d']])
  })

  it('keeps unicode intact', () => {
    expect(parseClipboardGrid('日本語\t🚀\té')).toEqual([['日本語', '🚀', 'é']])
  })

  it('remembers which fields were quoted', () => {
    expect(parseClipboard('"NULL"\tNULL')).toEqual([
      [
        { text: 'NULL', quoted: true },
        { text: 'NULL', quoted: false },
      ],
    ])
  })

  it('round-trips the grid\'s own copy, whatever the values hold', () => {
    const cols = [col('a'), col('b'), col('c')]
    const rows: Row[] = [
      ['plain', 'has\ttab', 'has "quotes"'],
      ['multi\nline', '', null],
      ['NULL', 'comma, here', '5" pipe'],
    ]
    const back = parseClipboard(toTsv(cols, rows, false)).map((r) => r.map((c) => (c.text === 'NULL' && !c.quoted ? null : c.text)))
    expect(back).toEqual(rows)
  })
})

describe('CSV fallback', () => {
  it('parses comma-separated text when there are no tabs', () => {
    expect(parseClipboardGrid('id,name\n1,Alice\n2,Bob')).toEqual([['id', 'name'], ['1', 'Alice'], ['2', 'Bob']])
  })

  it('handles quoted CSV fields', () => {
    expect(parseClipboardGrid('1,"Doe, John","say ""hi"""\n2,"multi\nline",x')).toEqual([
      ['1', 'Doe, John', 'say "hi"'],
      ['2', 'multi\nline', 'x'],
    ])
  })

  it('splits a single CSV line', () => {
    expect(parseClipboardGrid('1,Alice,a@example.com')).toEqual([['1', 'Alice', 'a@example.com']])
  })

  it('does not split prose that has a comma', () => {
    expect(parseClipboardGrid('Hello, world')).toEqual([['Hello, world']])
    expect(parseClipboardGrid('Doe, John')).toEqual([['Doe, John']])
  })

  it('keeps a number with thousands separators in one cell', () => {
    expect(parseClipboardGrid('1,234')).toEqual([['1,234']])
    expect(parseClipboardGrid('-12,345.67')).toEqual([['-12,345.67']])
    expect(parseClipboardGrid('1,234,567')).toEqual([['1,234,567']])
    // anything that is not grouped like a number is still a CSV line
    expect(parseClipboardGrid('1,23')).toEqual([['1', '23']])
    expect(parseClipboardGrid('12,3456')).toEqual([['12', '3456']])
    expect(parseClipboardGrid('1,234\n5,678')).toEqual([['1', '234'], ['5', '678']])
  })

  it('does not treat uneven comma counts as CSV', () => {
    expect(parseClipboardGrid('first line, with a comma\nsecond line')).toEqual([['first line, with a comma'], ['second line']])
    expect(parseClipboardGrid('a,b\nc')).toEqual([['a,b'], ['c']])
  })

  it('prefers tabs over commas', () => {
    expect(parseClipboardGrid('a,b\tc,d')).toEqual([['a,b', 'c,d']])
  })

  it('never second-guesses text the app copied itself', () => {
    const copied = 'red,green\nblue,yellow'
    expect(parseClipboardGrid(copied)).toEqual([['red', 'green'], ['blue', 'yellow']])
    rememberCopied(copied)
    expect(wasCopiedHere(copied)).toBe(true)
    expect(wasCopiedHere('something else')).toBe(false)
    expect(parseClipboardGrid(copied, { tsvOnly: wasCopiedHere(copied) })).toEqual([['red,green'], ['blue,yellow']])
  })
})

// ---------------------------------------------------------------------------
// Paste: one cell -> CellInput
// ---------------------------------------------------------------------------

describe('pasteInput', () => {
  const column = (family: TypeFamily, nullable = true, generated = false) => ({ family, nullable, generated })
  const input = (text: string, family: TypeFamily, engine: Engine = 'postgres', nullable = true, forInsert = false) =>
    pasteInput(text, column(family, nullable), engine, forInsert)

  it('turns the text NULL into NULL', () => {
    expect(input('NULL', 'text')).toEqual({ input: { kind: 'null' } })
    expect(input('NULL', 'integer')).toEqual({ input: { kind: 'null' } })
  })

  it('reads a quoted "NULL" as text', () => {
    expect(pasteInput({ text: 'NULL', quoted: true }, column('text'), 'postgres')).toEqual({ input: { kind: 'text', value: 'NULL' } })
  })

  it('only knows NULL in capitals', () => {
    expect(input('null', 'text')).toEqual({ input: { kind: 'text', value: 'null' } })
    expect(input('Null', 'text')).toEqual({ input: { kind: 'text', value: 'Null' } })
  })

  it('skips NULL for a NOT NULL column', () => {
    expect(input('NULL', 'text', 'postgres', false)).toEqual({ skip: 'notNull' })
  })

  it('skips generated columns', () => {
    expect(pasteInput('x', column('text', true, true), 'postgres')).toEqual({ skip: 'generated' })
    expect(pasteInput('NULL', column('text', true, true), 'postgres')).toEqual({ skip: 'generated' })
  })

  it('pastes an empty field as an empty string into text columns', () => {
    expect(input('', 'text')).toEqual({ input: { kind: 'text', value: '' } })
    expect(input('', 'text', 'postgres', false)).toEqual({ input: { kind: 'text', value: '' } })
    expect(input('', 'other')).toEqual({ input: { kind: 'text', value: '' } })
  })

  it('pastes an empty field as NULL into typed columns', () => {
    for (const f of ['integer', 'float', 'decimal', 'date', 'time', 'dateTime', 'uuid', 'boolean', 'json', 'enum'] as TypeFamily[]) {
      expect(input('', f)).toEqual({ input: { kind: 'null' } })
    }
  })

  it('uses the default for an empty field in a NOT NULL typed column of a new row, and skips it on an existing one', () => {
    expect(input('', 'integer', 'postgres', false, true)).toEqual({ input: { kind: 'default' } })
    expect(input('', 'integer', 'postgres', false, false)).toEqual({ skip: 'notNull' })
  })

  it('passes numbers, dates and uuids through as text, trimmed', () => {
    expect(input(' 42 ', 'integer')).toEqual({ input: { kind: 'text', value: '42' } })
    expect(input('3.14', 'decimal')).toEqual({ input: { kind: 'text', value: '3.14' } })
    expect(input('2024-01-31 ', 'date')).toEqual({ input: { kind: 'text', value: '2024-01-31' } })
    expect(input('2024-01-31 10:00:00', 'dateTime')).toEqual({ input: { kind: 'text', value: '2024-01-31 10:00:00' } })
    expect(input('abc', 'integer')).toEqual({ input: { kind: 'text', value: 'abc' } }) // the server reports it at commit
  })

  it('keeps text exactly as pasted, spaces and unicode included', () => {
    expect(input('  padded 日本語 🚀  ', 'text')).toEqual({ input: { kind: 'text', value: '  padded 日本語 🚀  ' } })
    expect(input('line1\nline2', 'text')).toEqual({ input: { kind: 'text', value: 'line1\nline2' } })
  })

  it('converts booleans the way the cell editor does for each engine', () => {
    expect(input('true', 'boolean', 'postgres')).toEqual({ input: { kind: 'text', value: 'true' } })
    expect(input('NO', 'boolean', 'postgres')).toEqual({ input: { kind: 'text', value: 'false' } })
    expect(input('1', 'boolean', 'mysql')).toEqual({ input: { kind: 'text', value: '1' } })
    expect(input('f', 'boolean', 'mariadb')).toEqual({ input: { kind: 'text', value: '0' } })
    expect(input('yes', 'boolean', 'sqlite')).toEqual({ input: { kind: 'text', value: '1' } })
    expect(input('true', 'boolean', 'mongodb')).toEqual({ input: { kind: 'json', value: true } })
    expect(input('0', 'boolean', 'mongodb')).toEqual({ input: { kind: 'json', value: false } })
    expect(input('maybe', 'boolean')).toEqual({ input: { kind: 'text', value: 'maybe' } })
  })

  it('parses JSON for json and document columns', () => {
    expect(input('{"a":[1,2]}', 'json')).toEqual({ input: { kind: 'json', value: { a: [1, 2] } } })
    expect(input('[1, 2]', 'document', 'mongodb')).toEqual({ input: { kind: 'json', value: [1, 2] } })
    expect(input('42', 'json')).toEqual({ input: { kind: 'json', value: 42 } })
    expect(input('{broken', 'json')).toEqual({ input: { kind: 'text', value: '{broken' } })
  })

  it('reads 0x hex into bytes for binary columns and skips other text', () => {
    expect(input('0xdeadBEEF', 'binary')).toEqual({ input: { kind: 'bytes', value: bytesToBase64(Uint8Array.from([0xde, 0xad, 0xbe, 0xef])) } })
    expect(input('0x', 'binary')).toEqual({ input: { kind: 'bytes', value: '' } }) // an empty blob copies as 0x
    expect(input('hello', 'binary')).toEqual({ skip: 'binary' })
    expect(input('0xabc', 'binary')).toEqual({ skip: 'binary' }) // odd number of digits
  })

  it('round-trips a copied NULL and an empty string', () => {
    const copied = toTsv([col('a'), col('b')], [[null, '']], false)
    const [[a, b]] = parseClipboard(copied)
    expect(pasteInput(a, column('text'), 'postgres')).toEqual({ input: { kind: 'null' } })
    expect(pasteInput(b, column('text'), 'postgres')).toEqual({ input: { kind: 'text', value: '' } })
  })

  it('round-trips a copied text value of NULL', () => {
    const [[cell]] = parseClipboard(toTsv([col('a')], [['NULL']], false))
    expect(pasteInput(cell, column('text'), 'postgres')).toEqual({ input: { kind: 'text', value: 'NULL' } })
  })
})

describe('pasteMessage', () => {
  it('counts the pasted cells', () => {
    expect(pasteMessage({ pasted: 1, newRows: 0, skipped: {}, extraColumns: 0 })).toEqual({ text: 'Pasted 1 cell.', kind: 'success' })
    expect(pasteMessage({ pasted: 6, newRows: 0, skipped: {}, extraColumns: 0 }).text).toBe('Pasted 6 cells.')
  })

  it('mentions new rows, skipped cells and ignored columns', () => {
    expect(pasteMessage({ pasted: 5, newRows: 2, skipped: { notNull: 1, generated: 2 }, extraColumns: 1 }).text).toBe(
      'Pasted 5 cells (2 new rows). Skipped 3 (1 NOT NULL, 2 generated columns). Ignored 1 extra column past the last column.',
    )
    expect(pasteMessage({ pasted: 2, newRows: 1, skipped: {}, extraColumns: 3 }).text).toBe(
      'Pasted 2 cells (1 new row). Ignored 3 extra columns past the last column.',
    )
  })

  it('reports an error when nothing could be pasted', () => {
    expect(pasteMessage({ pasted: 0, newRows: 0, skipped: { notLoaded: 4 }, extraColumns: 0 })).toEqual({
      text: 'Nothing was pasted. Skipped 4 (4 rows not loaded).',
      kind: 'error',
    })
  })

  it('says when rows were left out because of the cap', () => {
    expect(pasteMessage({ pasted: 10, newRows: 0, skipped: {}, extraColumns: 0, droppedRows: 7 }).text).toContain('7 rows left out')
  })
})
