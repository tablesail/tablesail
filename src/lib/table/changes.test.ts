import { describe, expect, it } from 'vitest'
import { DEFAULT_MARKER_TYPE, bytesToBase64 } from '../format'
import type { CellInput, ColumnValue, TableRef, Value } from '../types'
import {
  ChangeStore,
  DEFAULT_MARKER,
  inputToValue,
  isChangeDraft,
  keyString,
  valueToInput,
  type ChangeDraft,
} from './changes.svelte'

const TABLE: TableRef = { database: 'db', schema: 'public', name: 'users' }
const key = (id: number | string): ColumnValue[] => [{ column: 'id', value: id }]
const ks = (id: number | string) => keyString(key(id))
const text = (value: string): CellInput => ({ kind: 'text', value })

describe('keyString', () => {
  it('is built from the key values only (column names do not matter)', () => {
    expect(keyString([{ column: 'id', value: 1 }])).toBe('[1]')
    expect(keyString([{ column: 'pk', value: 1 }])).toBe(keyString([{ column: 'id', value: 1 }]))
  })

  it('distinguishes composite keys by position and value', () => {
    const a = keyString([{ column: 'a', value: 1 }, { column: 'b', value: 2 }])
    const b = keyString([{ column: 'a', value: 2 }, { column: 'b', value: 1 }])
    expect(a).not.toBe(b)
  })

  it('distinguishes number 1 from string "1"', () => {
    expect(ks(1)).not.toBe(ks('1'))
  })

  it('handles tagged values', () => {
    expect(keyString([{ column: 'id', value: { $int: '9007199254740993' } }])).toBe('[{"$int":"9007199254740993"}]')
  })

  it('handles null keys', () => {
    expect(keyString([{ column: 'id', value: null }])).toBe('[null]')
  })
})

describe('inputToValue', () => {
  it('null input shows NULL', () => {
    expect(inputToValue({ kind: 'null' })).toBeNull()
  })

  it('default input shows the DEFAULT marker', () => {
    const v = inputToValue({ kind: 'default' })
    expect(v).toBe(DEFAULT_MARKER)
    expect(v).toEqual({ $other: 'DEFAULT', type: DEFAULT_MARKER_TYPE })
  })

  it('original input shows the original value', () => {
    expect(inputToValue({ kind: 'original', value: { $int: '5' } })).toEqual({ $int: '5' })
  })

  it('text input shows the text', () => {
    expect(inputToValue(text('hello'))).toBe('hello')
    expect(inputToValue(text(''))).toBe('')
  })

  it('text input for a boolean column is parsed', () => {
    for (const t of ['true', 'TRUE', 't', 'T', '1', 'yes', 'YES', 'on', ' true ']) {
      expect(inputToValue(text(t), 'boolean')).toBe(true)
    }
    for (const t of ['false', 'f', '0', 'no', 'off', '', 'maybe']) {
      expect(inputToValue(text(t), 'boolean')).toBe(false)
    }
  })

  it('boolean parsing only applies to boolean columns', () => {
    expect(inputToValue(text('true'), 'text')).toBe('true')
    expect(inputToValue(text('1'), 'integer')).toBe('1')
    expect(inputToValue(text('true'))).toBe('true')
  })

  it('json input is wrapped', () => {
    expect(inputToValue({ kind: 'json', value: { a: 1 } })).toEqual({ $json: { a: 1 } })
  })

  it('bytes input shows base64 with the decoded length', () => {
    const b64 = bytesToBase64(Uint8Array.from([1, 2, 3, 4, 5]))
    expect(inputToValue({ kind: 'bytes', value: b64 })).toEqual({ $bytes: b64, len: 5 })
  })

  it('invalid base64 falls back to an $other value rather than throwing', () => {
    const v = inputToValue({ kind: 'bytes', value: '***not base64***' })
    expect(v).toEqual({ $other: '***not base64***', type: 'bytes' })
  })
})

describe('valueToInput', () => {
  it('undefined (not loaded) has no input', () => {
    expect(valueToInput(undefined)).toBeUndefined()
  })

  it('NULL re-inserts as null', () => {
    expect(valueToInput(null)).toEqual({ kind: 'null' })
  })

  it('complete values re-insert as "original"', () => {
    for (const v of [true, 5, 'abc', { $int: '1' }, { $json: { a: 1 } }, { $uuid: 'x' }] as Value[]) {
      expect(valueToInput(v)).toEqual({ kind: 'original', value: v })
    }
  })

  it('truncated text and bytes cannot be re-inserted', () => {
    expect(valueToInput({ $text: 'prefix', len: 1000 })).toBeUndefined()
    expect(valueToInput({ $bytes: bytesToBase64(Uint8Array.from([1, 2])), len: 99 })).toBeUndefined()
  })

  it('a complete blob can be re-inserted', () => {
    const v: Value = { $bytes: bytesToBase64(Uint8Array.from([1, 2])), len: 2 }
    expect(valueToInput(v)).toEqual({ kind: 'original', value: v })
  })

  it('the DEFAULT marker has no input', () => {
    expect(valueToInput(DEFAULT_MARKER)).toBeUndefined()
  })

  it('an ordinary $other value is kept', () => {
    const v: Value = { $other: 'POINT(1 2)', type: 'geometry' }
    expect(valueToInput(v)).toEqual({ kind: 'original', value: v })
  })
})

describe('ChangeStore: empty state', () => {
  it('starts empty', () => {
    const s = new ChangeStore()
    expect(s.isEmpty).toBe(true)
    expect(s.hasRowChanges).toBe(false)
    expect(s.insertCount).toBe(0)
    expect(s.pending).toBe(0)
    expect(s.summary).toEqual({ inserts: 0, updates: 0, deletes: 0 })
    expect(s.version).toBe(0)
    expect(s.buildChangeSet(TABLE)).toEqual({ table: TABLE, changes: [] })
  })

  it('lookups on an empty store are undefined/false', () => {
    const s = new ChangeStore()
    expect(s.editFor(ks(1), 'name')).toBeUndefined()
    expect(s.rowEdit(ks(1))).toBeUndefined()
    expect(s.isDeleted(ks(1))).toBe(false)
    expect(s.insertedAt(0)).toBeUndefined()
  })
})

describe('ChangeStore: cell edits', () => {
  it('records an edit with the original value', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    expect(s.editFor(ks(1), 'name')).toEqual({ original: 'Ann', input: text('Anna') })
    expect(s.isEmpty).toBe(false)
    expect(s.hasRowChanges).toBe(true)
    expect(s.summary).toEqual({ inserts: 0, updates: 1, deletes: 0 })
    expect(s.pending).toBe(1)
  })

  it('edits to several columns of one row count as one update', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    s.setCell(key(1), 'age', 30, text('31'))
    expect(s.summary.updates).toBe(1)
    expect([...s.rowEdit(ks(1))!.keys()]).toEqual(['name', 'age'])
  })

  it('edits to several rows count separately', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'a', text('A'))
    s.setCell(key(2), 'name', 'b', text('B'))
    expect(s.summary.updates).toBe(2)
    expect(s.pending).toBe(2)
  })

  it('re-editing keeps the first original (what the database still holds)', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    s.setCell(key(1), 'name', 'Anna', text('Annabel')) // second edit sees the pending value as "original"
    expect(s.editFor(ks(1), 'name')).toEqual({ original: 'Ann', input: text('Annabel') })
  })

  it('reverting using the loaded original clears the edit', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    s.setCell(key(1), 'name', 'Ann', text('Ann'))
    expect(s.editFor(ks(1), 'name')).toBeUndefined()
    expect(s.rowEdit(ks(1))).toBeUndefined()
    expect(s.isEmpty).toBe(true)
    expect(s.summary.updates).toBe(0)
  })

  it('reverting one column leaves the others', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    s.setCell(key(1), 'age', 30, text('31'))
    s.setCell(key(1), 'name', 'Ann', text('Ann'))
    expect(s.editFor(ks(1), 'name')).toBeUndefined()
    expect(s.editFor(ks(1), 'age')).toBeDefined()
    expect(s.summary.updates).toBe(1)
  })

  it('"unchanged" edits to a never-edited cell do not bump the version', () => {
    const s = new ChangeStore()
    const v = s.version
    s.setCell(key(1), 'name', 'Ann', text('Ann'))
    expect(s.version).toBe(v)
    expect(s.isEmpty).toBe(true)
  })

  it('stores edits per key, so a different row with the same column is independent', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'a', text('x'))
    expect(s.editFor(ks(2), 'name')).toBeUndefined()
  })

  describe('"unchanged" detection', () => {
    const unchanged = (original: Value, input: CellInput) => {
      const s = new ChangeStore()
      s.setCell(key(1), 'c', original, input)
      return s.isEmpty
    }

    it('text vs text', () => {
      expect(unchanged('abc', text('abc'))).toBe(true)
      expect(unchanged('abc', text('abd'))).toBe(false)
      expect(unchanged('', text(''))).toBe(true)
    })

    it('numbers and tagged numbers compare by their edit text', () => {
      expect(unchanged(5, text('5'))).toBe(true)
      expect(unchanged(5, text('6'))).toBe(false)
      expect(unchanged({ $int: '42' }, text('42'))).toBe(true)
      expect(unchanged({ $int: '42' }, text('43'))).toBe(false)
      expect(unchanged({ $dec: '1.50' }, text('1.50'))).toBe(true)
      expect(unchanged({ $dec: '1.50' }, text('1.5'))).toBe(false)
    })

    it('booleans compare by parsed truthiness', () => {
      expect(unchanged(true, text('true'))).toBe(true)
      expect(unchanged(true, text('yes'))).toBe(true)
      expect(unchanged(true, text('false'))).toBe(false)
      expect(unchanged(false, text('0'))).toBe(true)
      expect(unchanged(false, text('1'))).toBe(false)
    })

    it('null vs text is a change, even for empty text', () => {
      expect(unchanged(null, text(''))).toBe(false)
      expect(unchanged(null, text('x'))).toBe(false)
    })

    it('undefined original vs text is a change', () => {
      expect(unchanged(undefined as unknown as Value, text('x'))).toBe(false)
    })

    it('null input on a null original is unchanged; on a value it is a change', () => {
      expect(unchanged(null, { kind: 'null' })).toBe(true)
      expect(unchanged('x', { kind: 'null' })).toBe(false)
    })

    it('"original" input is always unchanged (it means revert)', () => {
      expect(unchanged('x', { kind: 'original', value: 'x' })).toBe(true)
    })

    it('"default" input is always a change', () => {
      expect(unchanged(null, { kind: 'default' })).toBe(false)
      expect(unchanged('x', { kind: 'default' })).toBe(false)
    })

    it('bytes input is always a change', () => {
      expect(unchanged({ $bytes: 'AQI=', len: 2 }, { kind: 'bytes', value: 'AQI=' })).toBe(false)
    })

    it('JSON compares structurally by serialisation', () => {
      expect(unchanged({ $json: { a: 1 } }, { kind: 'json', value: { a: 1 } })).toBe(true)
      expect(unchanged({ $json: { a: 1 } }, { kind: 'json', value: { a: 2 } })).toBe(false)
      expect(unchanged({ $json: [1, 2] }, { kind: 'json', value: [1, 2] })).toBe(true)
      expect(unchanged({ $json: null }, { kind: 'json', value: null })).toBe(true)
    })

    it('JSON input against a non-JSON original is a change', () => {
      expect(unchanged('{"a":1}', { kind: 'json', value: { a: 1 } })).toBe(false)
      expect(unchanged(null, { kind: 'json', value: null })).toBe(false)
    })

    it('text input can never equal a truncated, binary or JSON original', () => {
      expect(unchanged({ $text: 'prefix', len: 100 }, text('prefix'))).toBe(false)
      expect(unchanged({ $bytes: 'AQI=', len: 2 }, text('0x0102'))).toBe(false)
      expect(unchanged({ $json: { a: 1 } }, text('{"a":1}'))).toBe(false)
    })

    it('dates and uuids compare by their string form', () => {
      expect(unchanged({ $date: '2024-01-02' }, text('2024-01-02'))).toBe(true)
      expect(unchanged({ $uuid: 'abc' }, text('abc'))).toBe(true)
      expect(unchanged({ $uuid: 'abc' }, text('abd'))).toBe(false)
    })
  })
})

describe('ChangeStore: deletes', () => {
  it('marks and unmarks a row', () => {
    const s = new ChangeStore()
    s.setDeleted(key(1), true)
    expect(s.isDeleted(ks(1))).toBe(true)
    expect(s.summary).toEqual({ inserts: 0, updates: 0, deletes: 1 })
    expect(s.hasRowChanges).toBe(true)
    s.setDeleted(key(1), false)
    expect(s.isDeleted(ks(1))).toBe(false)
    expect(s.isEmpty).toBe(true)
  })

  it('is idempotent and does not bump the version when nothing changes', () => {
    const s = new ChangeStore()
    s.setDeleted(key(1), true)
    const v = s.version
    s.setDeleted(key(1), true)
    s.setDeleted(key(2), false)
    expect(s.version).toBe(v)
    expect(s.summary.deletes).toBe(1)
  })

  it('a deleted row no longer counts as an update', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'a', text('b'))
    expect(s.summary.updates).toBe(1)
    s.setDeleted(key(1), true)
    expect(s.summary).toEqual({ inserts: 0, updates: 0, deletes: 1 })
    expect(s.pending).toBe(1)
  })

  it('un-deleting brings the pending edit back', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'name', 'a', text('b'))
    s.setDeleted(key(1), true)
    s.setDeleted(key(1), false)
    expect(s.summary).toEqual({ inserts: 0, updates: 1, deletes: 0 })
  })

  it('counts several deletes', () => {
    const s = new ChangeStore()
    for (const id of [1, 2, 3]) s.setDeleted(key(id), true)
    expect(s.summary.deletes).toBe(3)
  })
})

describe('ChangeStore: inserted rows', () => {
  it('adds rows with increasing ids', () => {
    const s = new ChangeStore()
    const a = s.addInsert()
    const b = s.addInsert()
    expect(a.id).toBe(1)
    expect(b.id).toBe(2)
    expect(s.insertCount).toBe(2)
    expect(s.summary.inserts).toBe(2)
    expect(s.isEmpty).toBe(false)
    expect(s.hasRowChanges).toBe(false) // inserts are not "row changes" of existing rows
  })

  it('copies the initial values', () => {
    const s = new ChangeStore()
    const init = new Map<string, CellInput>([['name', text('x')]])
    const row = s.addInsert(init)
    init.set('name', text('mutated'))
    expect(row.values.get('name')).toEqual(text('x'))
  })

  it('displays newest first', () => {
    const s = new ChangeStore()
    const a = s.addInsert()
    const b = s.addInsert()
    const c = s.addInsert()
    expect(s.insertedAt(0)).toBe(c)
    expect(s.insertedAt(1)).toBe(b)
    expect(s.insertedAt(2)).toBe(a)
    expect(s.insertedAt(3)).toBeUndefined()
    expect(s.insertedAt(-1)).toBeUndefined()
  })

  it('sets and overwrites a cell', () => {
    const s = new ChangeStore()
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', text('a'))
    s.setInsertCell(row.id, 'name', text('b'))
    expect(row.values.get('name')).toEqual(text('b'))
  })

  it('"default" removes the column so the database default applies', () => {
    const s = new ChangeStore()
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', text('a'))
    s.setInsertCell(row.id, 'name', { kind: 'default' })
    expect(row.values.has('name')).toBe(false)
  })

  it('"null" is stored as an explicit NULL', () => {
    const s = new ChangeStore()
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', { kind: 'null' })
    expect(row.values.get('name')).toEqual({ kind: 'null' })
  })

  it('ignores an unknown insert id', () => {
    const s = new ChangeStore()
    s.addInsert()
    const v = s.version
    s.setInsertCell(999, 'x', text('y'))
    expect(s.version).toBe(v)
  })

  it('removes an insert by id and keeps the others', () => {
    const s = new ChangeStore()
    const a = s.addInsert()
    const b = s.addInsert()
    s.removeInsert(a.id)
    expect(s.insertCount).toBe(1)
    expect(s.insertedAt(0)).toBe(b)
  })

  it('ids are never reused after a removal', () => {
    const s = new ChangeStore()
    const a = s.addInsert()
    s.removeInsert(a.id)
    expect(s.addInsert().id).toBe(a.id + 1)
  })

  it('removing an unknown id is a no-op', () => {
    const s = new ChangeStore()
    s.addInsert()
    const v = s.version
    s.removeInsert(42)
    expect(s.version).toBe(v)
    expect(s.insertCount).toBe(1)
  })
})

describe('ChangeStore: version bumps', () => {
  it('every mutation bumps the version', () => {
    const s = new ChangeStore()
    let v = s.version
    const bumped = () => {
      const changed = s.version > v
      v = s.version
      return changed
    }
    s.setCell(key(1), 'a', 'x', text('y'))
    expect(bumped()).toBe(true)
    s.setDeleted(key(2), true)
    expect(bumped()).toBe(true)
    const row = s.addInsert()
    expect(bumped()).toBe(true)
    s.setInsertCell(row.id, 'a', text('1'))
    expect(bumped()).toBe(true)
    s.removeInsert(row.id)
    expect(bumped()).toBe(true)
    s.discardRow(ks(1))
    expect(bumped()).toBe(true)
    s.clear()
    expect(bumped()).toBe(true)
    s.restore({ version: 1, inserts: [], edits: [], deletes: [] })
    expect(bumped()).toBe(true)
  })

  it('reads never bump the version', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    const v = s.version
    void s.isEmpty, s.hasRowChanges, s.insertCount, s.summary, s.pending
    s.editFor(ks(1), 'a')
    s.rowEdit(ks(1))
    s.isDeleted(ks(1))
    s.insertedAt(0)
    s.buildChangeSet(TABLE)
    s.toDraft()
    expect(s.version).toBe(v)
  })
})

describe('ChangeStore: discardRow / clear', () => {
  it('discardRow drops the edits of one row only', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setCell(key(2), 'a', 'x', text('y'))
    s.discardRow(ks(1))
    expect(s.rowEdit(ks(1))).toBeUndefined()
    expect(s.rowEdit(ks(2))).toBeDefined()
    expect(s.summary.updates).toBe(1)
  })

  it('discardRow un-deletes a deleted row', () => {
    const s = new ChangeStore()
    s.setDeleted(key(1), true)
    s.discardRow(ks(1))
    expect(s.isDeleted(ks(1))).toBe(false)
    expect(s.isEmpty).toBe(true)
  })

  it('discardRow removes both the edits and the delete of a row (regression: short-circuit)', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setDeleted(key(1), true)
    s.discardRow(ks(1))
    expect(s.rowEdit(ks(1))).toBeUndefined()
    expect(s.isDeleted(ks(1))).toBe(false)
    expect(s.isEmpty).toBe(true)
  })

  it('discardRow on an untouched row does nothing (no version bump)', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    const v = s.version
    s.discardRow(ks(99))
    expect(s.version).toBe(v)
  })

  it('discardRow leaves inserted rows alone', () => {
    const s = new ChangeStore()
    s.addInsert()
    s.discardRow(ks(1))
    expect(s.insertCount).toBe(1)
  })

  it('clear removes everything', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setDeleted(key(2), true)
    s.addInsert()
    s.clear()
    expect(s.isEmpty).toBe(true)
    expect(s.pending).toBe(0)
    expect(s.summary).toEqual({ inserts: 0, updates: 0, deletes: 0 })
    expect(s.buildChangeSet(TABLE).changes).toEqual([])
  })

  it('clear on an empty store is harmless', () => {
    const s = new ChangeStore()
    expect(() => s.clear()).not.toThrow()
    expect(s.isEmpty).toBe(true)
  })
})

describe('ChangeStore: buildChangeSet', () => {
  it('orders inserts, then updates, then deletes', () => {
    const s = new ChangeStore()
    s.setDeleted(key(3), true)
    s.setCell(key(2), 'name', 'old', text('new'))
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', text('fresh'))
    const { table, changes } = s.buildChangeSet(TABLE)
    expect(table).toBe(TABLE)
    expect(changes.map((c) => c.kind)).toEqual(['insert', 'update', 'delete'])
  })

  it('insert carries column/value pairs', () => {
    const s = new ChangeStore()
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', text('Zed'))
    s.setInsertCell(row.id, 'bio', { kind: 'null' })
    expect(s.buildChangeSet(TABLE).changes).toEqual([
      {
        kind: 'insert',
        values: [
          { column: 'name', value: text('Zed') },
          { column: 'bio', value: { kind: 'null' } },
        ],
      },
    ])
  })

  it('an insert with no values is an all-defaults insert', () => {
    const s = new ChangeStore()
    s.addInsert()
    expect(s.buildChangeSet(TABLE).changes).toEqual([{ kind: 'insert', values: [] }])
  })

  it('inserts are emitted oldest first (insertion order)', () => {
    const s = new ChangeStore()
    const a = s.addInsert()
    const b = s.addInsert()
    s.setInsertCell(a.id, 'n', text('first'))
    s.setInsertCell(b.id, 'n', text('second'))
    const values = s.buildChangeSet(TABLE).changes.map((c) => (c as { values: { value: CellInput }[] }).values[0].value)
    expect(values).toEqual([text('first'), text('second')])
  })

  it('update carries the key, original values (for the optimistic WHERE) and new inputs', () => {
    const s = new ChangeStore()
    s.setCell(key(7), 'name', 'Ann', text('Anna'))
    s.setCell(key(7), 'age', { $int: '30' }, text('31'))
    expect(s.buildChangeSet(TABLE).changes).toEqual([
      {
        kind: 'update',
        key: key(7),
        original: [
          { column: 'name', value: 'Ann' },
          { column: 'age', value: { $int: '30' } },
        ],
        set: [
          { column: 'name', value: text('Anna') },
          { column: 'age', value: text('31') },
        ],
      },
    ])
  })

  it('emits one update per edited row', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setCell(key(2), 'a', 'x', text('z'))
    expect(s.buildChangeSet(TABLE).changes.filter((c) => c.kind === 'update')).toHaveLength(2)
  })

  it('a deleted row yields only a delete, never an update', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setDeleted(key(1), true)
    expect(s.buildChangeSet(TABLE).changes).toEqual([{ kind: 'delete', key: key(1) }])
  })

  it('composite keys are passed through intact', () => {
    const s = new ChangeStore()
    const k: ColumnValue[] = [
      { column: 'a', value: 1 },
      { column: 'b', value: 'x' },
    ]
    s.setDeleted(k, true)
    expect(s.buildChangeSet(TABLE).changes).toEqual([{ kind: 'delete', key: k }])
  })

  it('the change count matches pending', () => {
    const s = new ChangeStore()
    s.addInsert()
    s.addInsert()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.setDeleted(key(2), true)
    s.setDeleted(key(3), true)
    expect(s.buildChangeSet(TABLE).changes).toHaveLength(s.pending)
    expect(s.pending).toBe(5)
  })

  it('building a change set does not consume the pending changes', () => {
    const s = new ChangeStore()
    s.setCell(key(1), 'a', 'x', text('y'))
    s.buildChangeSet(TABLE)
    expect(s.pending).toBe(1)
  })
})

describe('ChangeStore: drafts', () => {
  function populated() {
    const s = new ChangeStore()
    const row = s.addInsert()
    s.setInsertCell(row.id, 'name', text('New'))
    s.setCell(key(1), 'name', 'Ann', text('Anna'))
    s.setCell(key(1), 'meta', { $json: { a: 1 } }, { kind: 'json', value: { a: 2 } })
    s.setDeleted(key(5), true)
    return s
  }

  it('toDraft is JSON-serialisable and tagged version 1', () => {
    const draft = populated().toDraft()
    expect(draft.version).toBe(1)
    expect(JSON.parse(JSON.stringify(draft))).toEqual(draft)
  })

  it('round-trips through JSON into a fresh store', () => {
    const a = populated()
    const draft = JSON.parse(JSON.stringify(a.toDraft())) as ChangeDraft
    const b = new ChangeStore()
    b.restore(draft)
    expect(b.summary).toEqual(a.summary)
    expect(b.buildChangeSet(TABLE)).toEqual(a.buildChangeSet(TABLE))
    expect(b.isDeleted(ks(5))).toBe(true)
    expect(b.editFor(ks(1), 'name')).toEqual({ original: 'Ann', input: text('Anna') })
  })

  it('restore replaces any existing state', () => {
    const b = new ChangeStore()
    b.setCell(key(99), 'x', 'a', text('b'))
    b.setDeleted(key(98), true)
    b.addInsert()
    b.restore(populated().toDraft())
    expect(b.rowEdit(ks(99))).toBeUndefined()
    expect(b.isDeleted(ks(98))).toBe(false)
    expect(b.summary).toEqual({ inserts: 1, updates: 1, deletes: 1 })
  })

  it('restored inserts keep their ids and new ids continue after them', () => {
    const a = new ChangeStore()
    a.addInsert()
    a.addInsert()
    const b = new ChangeStore()
    b.restore(a.toDraft())
    expect(b.insertedAt(0)?.id).toBe(2)
    expect(b.insertedAt(1)?.id).toBe(1)
    expect(b.addInsert().id).toBe(3)
  })

  it('restore does not alias the draft maps', () => {
    const draft = populated().toDraft()
    const b = new ChangeStore()
    b.restore(draft)
    b.setInsertCell(b.insertedAt(0)!.id, 'name', text('changed'))
    expect(draft.inserts[0].values).toEqual([['name', text('New')]])
  })

  it('an empty store gives an empty draft', () => {
    expect(new ChangeStore().toDraft()).toEqual({ version: 1, inserts: [], edits: [], deletes: [] })
  })

  it('restoring an empty draft empties the store', () => {
    const b = populated()
    b.restore({ version: 1, inserts: [], edits: [], deletes: [] })
    expect(b.isEmpty).toBe(true)
  })
})

describe('isChangeDraft', () => {
  const valid = { version: 1, inserts: [], edits: [], deletes: [] }

  it('accepts a well-formed draft', () => {
    expect(isChangeDraft(valid)).toBe(true)
    expect(isChangeDraft(new ChangeStore().toDraft())).toBe(true)
  })

  it('rejects non-objects', () => {
    for (const x of [null, undefined, 0, 1, 'x', true, [], () => 1]) {
      expect(isChangeDraft(x)).toBe(false)
    }
  })

  it('rejects the wrong version', () => {
    expect(isChangeDraft({ ...valid, version: 2 })).toBe(false)
    expect(isChangeDraft({ ...valid, version: undefined })).toBe(false)
    expect(isChangeDraft({ ...valid, version: '1' })).toBe(false)
  })

  it('rejects missing or non-array sections', () => {
    expect(isChangeDraft({ version: 1, inserts: [], edits: [] })).toBe(false)
    expect(isChangeDraft({ ...valid, inserts: {} })).toBe(false)
    expect(isChangeDraft({ ...valid, edits: null })).toBe(false)
    expect(isChangeDraft({ ...valid, deletes: 'x' })).toBe(false)
  })
})
