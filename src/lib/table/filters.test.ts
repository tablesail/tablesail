import { describe, expect, it } from 'vitest'
import type { FilterOp } from '../types'
import { FILTER_OPS, opTakesValue, toFilters, type FilterRow } from './filters'

const row = (over: Partial<FilterRow>): FilterRow => ({ id: 1, column: 'name', op: 'eq', value: 'x', ...over })

describe('FILTER_OPS', () => {
  it('covers every FilterOp exactly once', () => {
    const all: FilterOp[] = [
      'eq', 'neq', 'lt', 'lte', 'gt', 'gte', 'contains', 'notContains', 'startsWith', 'endsWith',
      'like', 'in', 'notIn', 'isNull', 'isNotNull',
    ]
    expect(FILTER_OPS.map((o) => o.op).sort()).toEqual([...all].sort())
    expect(new Set(FILTER_OPS.map((o) => o.op)).size).toBe(FILTER_OPS.length)
  })

  it('every op has a non-empty label', () => {
    for (const o of FILTER_OPS) expect(o.label.length).toBeGreaterThan(0)
  })

  it('only the NULL checks take no value', () => {
    expect(FILTER_OPS.filter((o) => o.noValue).map((o) => o.op)).toEqual(['isNull', 'isNotNull'])
  })
})

describe('opTakesValue', () => {
  it('is false for isNull / isNotNull', () => {
    expect(opTakesValue('isNull')).toBe(false)
    expect(opTakesValue('isNotNull')).toBe(false)
  })

  it('is true for every other operator', () => {
    for (const o of FILTER_OPS.filter((x) => !x.noValue)) expect(opTakesValue(o.op)).toBe(true)
  })

  it('treats an unknown operator as taking a value', () => {
    expect(opTakesValue('bogus' as FilterOp)).toBe(true)
  })
})

describe('toFilters', () => {
  it('converts complete rows, dropping the UI-only id', () => {
    expect(toFilters([row({ id: 7, column: 'age', op: 'gte', value: '18' })])).toEqual([{ column: 'age', op: 'gte', value: '18' }])
  })

  it('preserves order', () => {
    const out = toFilters([row({ column: 'a' }), row({ column: 'b' }), row({ column: 'c' })])
    expect(out.map((f) => f.column)).toEqual(['a', 'b', 'c'])
  })

  it('ignores rows without a column', () => {
    expect(toFilters([row({ column: '' })])).toEqual([])
  })

  it('ignores rows with an empty value when the operator needs one', () => {
    expect(toFilters([row({ value: '' })])).toEqual([])
    expect(toFilters([row({ op: 'in', value: '' })])).toEqual([])
  })

  it('keeps a whitespace-only value (it is a real value)', () => {
    expect(toFilters([row({ value: ' ' })])).toEqual([{ column: 'name', op: 'eq', value: ' ' }])
  })

  it('keeps the value "0"', () => {
    expect(toFilters([row({ value: '0' })])).toEqual([{ column: 'name', op: 'eq', value: '0' }])
  })

  it('value-less operators produce a filter without a value, ignoring any leftover text', () => {
    const [f] = toFilters([row({ op: 'isNull', value: 'stale text' })])
    expect(f).toEqual({ column: 'name', op: 'isNull' })
    expect('value' in f).toBe(false)
    expect(toFilters([row({ op: 'isNotNull', value: '' })])).toEqual([{ column: 'name', op: 'isNotNull' }])
  })

  it('a value-less operator without a column is still ignored', () => {
    expect(toFilters([row({ column: '', op: 'isNull' })])).toEqual([])
  })

  it('mixes complete and incomplete rows', () => {
    const out = toFilters([
      row({ id: 1, column: 'a', value: '1' }),
      row({ id: 2, column: 'b', value: '' }),
      row({ id: 3, column: '', value: 'x' }),
      row({ id: 4, column: 'c', op: 'isNull', value: '' }),
    ])
    expect(out).toEqual([
      { column: 'a', op: 'eq', value: '1' },
      { column: 'c', op: 'isNull' },
    ])
  })

  it('returns an empty array for no rows', () => {
    expect(toFilters([])).toEqual([])
  })

  it('passes list operators through unparsed', () => {
    expect(toFilters([row({ op: 'in', value: 'a, b, c' })])).toEqual([{ column: 'name', op: 'in', value: 'a, b, c' }])
  })
})
