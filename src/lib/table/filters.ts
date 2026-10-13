import type { Filter, FilterOp } from '../types'

export interface FilterRow {
  id: number
  column: string
  op: FilterOp
  value: string
}

export const FILTER_OPS: { op: FilterOp; label: string; noValue?: boolean }[] = [
  { op: 'eq', label: '=' },
  { op: 'neq', label: '≠' },
  { op: 'lt', label: '<' },
  { op: 'lte', label: '≤' },
  { op: 'gt', label: '>' },
  { op: 'gte', label: '≥' },
  { op: 'contains', label: 'contains' },
  { op: 'notContains', label: 'does not contain' },
  { op: 'startsWith', label: 'starts with' },
  { op: 'endsWith', label: 'ends with' },
  { op: 'like', label: 'LIKE' },
  { op: 'in', label: 'in (a, b, …)' },
  { op: 'notIn', label: 'not in (a, b, …)' },
  { op: 'isNull', label: 'is NULL', noValue: true },
  { op: 'isNotNull', label: 'is not NULL', noValue: true },
]

export function opTakesValue(op: FilterOp): boolean {
  return FILTER_OPS.find((o) => o.op === op)?.noValue !== true
}

/** Rows that are complete become filters; a value-less row is ignored until it is filled in. */
export function toFilters(rows: FilterRow[]): Filter[] {
  const out: Filter[] = []
  for (const r of rows) {
    if (!r.column) continue
    if (opTakesValue(r.op)) {
      if (r.value === '') continue
      out.push({ column: r.column, op: r.op, value: r.value })
    } else {
      out.push({ column: r.column, op: r.op })
    }
  }
  return out
}
