import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { routeInvoke, settle, type InvokeHandlers } from '../../test/helpers'
import type { ColumnMeta, CoreError, QueryStatus, ResultSetStatus, Row, RowsPage, StatementSpan, StatementStatus } from '../types'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

import { PAGE, QueryRunner } from './runner.svelte'

const coreError = (kind: CoreError['kind'], message: string): CoreError => ({ kind, message, code: null, statement: null })
const COLS: ColumnMeta[] = [
  { name: 'id', typeName: 'int4', family: 'integer' },
  { name: 'name', typeName: 'text', family: 'text' },
]

const span = (i: number, sql = `SELECT ${i}`): StatementSpan => ({ start: 0, end: sql.length, sql, kind: 'read', destructive: false })

function set(over: Partial<ResultSetStatus> = {}): ResultSetStatus {
  return { setIndex: 0, columns: COLS, loaded: 0, complete: false, truncated: false, ...over }
}

function stmt(index: number, sets: ResultSetStatus[], over: Partial<StatementStatus> = {}): StatementStatus {
  return { index, sql: `SELECT ${index}`, state: 'running', error: null, affected: null, sets, elapsedMs: 0, ...over }
}

const status = (finished: boolean, statements: StatementStatus[], queryId = 'q1'): QueryStatus => ({ queryId, finished, statements })

/** `n` rows starting at `offset`, in the shape the grid reads: [id, name]. */
const rows = (offset: number, n: number): Row[] => Array.from({ length: n }, (_, i) => [offset + i, `row ${offset + i}`])

interface Backend {
  calls: { command: string; args: Record<string, unknown> }[]
  /** Statuses returned by successive query_status calls; the last one repeats. */
  statuses: QueryStatus[]
  totalRows: number
  fetches: [number, number][]
}

function install(over: Partial<{ handlers: InvokeHandlers; statuses: QueryStatus[]; totalRows: number; statements: StatementSpan[]; queryId: string }> = {}) {
  const backend: Backend = { calls: [], statuses: over.statuses ?? [status(true, [stmt(0, [set({ loaded: 3, complete: true })], { state: 'done' })])], totalRows: over.totalRows ?? 3, fetches: [] }
  let statusCalls = 0
  const routed = routeInvoke({
    query_run: () => ({ queryId: over.queryId ?? 'q1', statements: over.statements ?? [span(0)] }),
    query_status: () => backend.statuses[Math.min(statusCalls++, backend.statuses.length - 1)],
    query_fetch: ({ offset, limit }): RowsPage => {
      const off = Number(offset)
      const lim = Number(limit)
      backend.fetches.push([off, lim])
      const n = Math.max(0, Math.min(lim, backend.totalRows - off))
      return { rows: rows(off, n), offset: off, loaded: backend.totalRows, complete: off + n >= backend.totalRows }
    },
    query_cancel: () => undefined,
    query_close: () => undefined,
    ...over.handlers,
  })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    backend.calls.push({ command, args })
    return routed(command, args)
  })
  return backend
}

const commands = (b: Backend) => b.calls.map((c) => c.command)

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
})

describe('QueryRunner: initial state', () => {
  it('is idle', () => {
    const r = new QueryRunner()
    expect(r.running).toBe(false)
    expect(r.queryId).toBeNull()
    expect(r.status).toBeNull()
    expect(r.statements).toEqual([])
    expect(r.runError).toBeNull()
    expect(r.elapsedMs).toBe(0)
    expect(r.revision).toBe(0)
    expect(r.runId).toBe(0)
  })

  it('page size is 500', () => {
    expect(PAGE).toBe(500)
  })

  it('info() of an unknown result set is empty', () => {
    const r = new QueryRunner()
    expect(r.info(0, 0)).toEqual({ columns: [], loaded: 0, complete: false, truncated: false, failed: null })
  })

  it('rowCount of an unknown set is one speculative page', () => {
    expect(new QueryRunner().rowCount(0, 0)).toBe(PAGE)
  })

  it('getRow of an unknown set is undefined', () => {
    expect(new QueryRunner().getRow(0, 0, 0)).toBeUndefined()
  })
})

describe('QueryRunner.run: a finished query', () => {
  it('starts the query and reports its statements', async () => {
    const b = install({ statements: [span(0, 'SELECT 1'), span(1, 'SELECT 2')] })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1; SELECT 2')
    expect(b.calls.find((c) => c.command === 'query_run')?.args).toEqual({ sessionId: 's1', database: 'app', sql: 'SELECT 1; SELECT 2', txId: null })
    expect(r.queryId).toBe('q1')
    expect(r.statements.map((s) => s.sql)).toEqual(['SELECT 1', 'SELECT 2'])
  })

  it('runs inside a manual transaction when given its id', async () => {
    const b = install()
    await new QueryRunner().run('s1', 'app', 'UPDATE t SET a = 1', 'tx7')
    expect(b.calls.find((c) => c.command === 'query_run')?.args).toEqual({ sessionId: 's1', database: 'app', sql: 'UPDATE t SET a = 1', txId: 'tx7' })
  })

  it('passes a null database through', async () => {
    const b = install()
    await new QueryRunner().run('s1', null, 'SELECT 1')
    expect(b.calls.find((c) => c.command === 'query_run')?.args.database).toBeNull()
  })

  it('polls until finished, then stops running', async () => {
    install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.running).toBe(false)
    expect(r.status?.finished).toBe(true)
    expect(r.runError).toBeNull()
  })

  it('increments runId for every run', async () => {
    install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.runId).toBe(1)
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.runId).toBe(2)
  })

  it('exposes columns, counts and completion per result set', async () => {
    install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.info(0, 0)).toEqual({ columns: COLS, loaded: 3, complete: true, truncated: false, failed: null })
  })

  it('fetches the first page of every result set automatically and serves its rows', async () => {
    const b = install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(b.fetches).toEqual([[0, PAGE]])
    expect(r.getRow(0, 0, 0)).toEqual([0, 'row 0'])
    expect(r.getRow(0, 0, 2)).toEqual([2, 'row 2'])
    expect(r.getRow(0, 0, 3)).toBeUndefined()
  })

  it('a complete result is sized to exactly its rows', async () => {
    install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.rowCount(0, 0)).toBe(3)
  })

  it('bumps the revision as data arrives', async () => {
    install()
    const r = new QueryRunner()
    const before = r.revision
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.revision).toBeGreaterThan(before)
  })

  it('measures elapsed time', async () => {
    install({
      statuses: [status(false, [stmt(0, [set()])]), status(true, [stmt(0, [set({ loaded: 3, complete: true })], { state: 'done' })])],
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await vi.advanceTimersByTimeAsync(500)
    await p
    expect(r.elapsedMs).toBeGreaterThanOrEqual(0)
    expect(Number.isFinite(r.elapsedMs)).toBe(true)
  })
})

describe('QueryRunner.run: polling', () => {
  it('keeps running while the backend says unfinished, polling every 60 ms', async () => {
    const b = install({
      statuses: [
        status(false, [stmt(0, [set({ loaded: 0 })])]),
        status(false, [stmt(0, [set({ loaded: 10 })])]),
        status(true, [stmt(0, [set({ loaded: 10, complete: true })], { state: 'done' })]),
      ],
      totalRows: 10,
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await settle(10)
    expect(r.running).toBe(true)
    expect(commands(b).filter((c) => c === 'query_status')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(59)
    expect(commands(b).filter((c) => c === 'query_status')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(commands(b).filter((c) => c === 'query_status')).toHaveLength(2)
    expect(r.running).toBe(true)
    await vi.advanceTimersByTimeAsync(60)
    await p
    expect(commands(b).filter((c) => c === 'query_status')).toHaveLength(3)
    expect(r.running).toBe(false)
  })

  it('shows progress while running: speculative row count until complete', async () => {
    install({
      statuses: [status(false, [stmt(0, [set({ loaded: 120 })])]), status(true, [stmt(0, [set({ loaded: 120, complete: true })], { state: 'done' })])],
      totalRows: 120,
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await settle(10)
    expect(r.info(0, 0).loaded).toBe(120)
    expect(r.info(0, 0).complete).toBe(false)
    expect(r.rowCount(0, 0)).toBe(120 + PAGE)
    await vi.advanceTimersByTimeAsync(100)
    await p
    expect(r.rowCount(0, 0)).toBe(120)
  })

  it('the loaded count never goes backwards and completion is sticky', async () => {
    install({
      statuses: [
        status(false, [stmt(0, [set({ loaded: 50, complete: true })])]),
        status(true, [stmt(0, [set({ loaded: 10, complete: false })], { state: 'done' })]),
      ],
      totalRows: 50,
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await vi.advanceTimersByTimeAsync(200)
    await p
    expect(r.info(0, 0).loaded).toBe(50)
    expect(r.info(0, 0).complete).toBe(true)
  })

  it('follows the truncated flag of the latest status', async () => {
    install({
      statuses: [status(true, [stmt(0, [set({ loaded: 10_000, complete: true, truncated: true })], { state: 'done' })])],
      totalRows: 10_000,
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.info(0, 0).truncated).toBe(true)
  })

  it('handles scripts: several statements, several result sets', async () => {
    const b = install({
      statuses: [
        status(true, [
          stmt(0, [set({ setIndex: 0, loaded: 2, complete: true })], { state: 'done' }),
          stmt(1, [set({ setIndex: 0, loaded: 1, complete: true }), set({ setIndex: 1, loaded: 4, complete: true })], { state: 'done' }),
        ]),
      ],
      totalRows: 2,
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'a; b')
    await settle()
    expect(b.calls.filter((c) => c.command === 'query_fetch').map((c) => [c.args.statementIndex, c.args.setIndex])).toEqual([
      [0, 0],
      [1, 0],
      [1, 1],
    ])
    expect(r.info(1, 1).loaded).toBe(4)
    expect(r.getRow(1, 1, 0)).toEqual([0, 'row 0'])
  })

  it('statements without result sets (writes) are fine', async () => {
    const b = install({ statuses: [status(true, [stmt(0, [], { state: 'done', affected: 5 })])] })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'UPDATE t SET a = 1')
    expect(r.status?.statements[0].affected).toBe(5)
    expect(commands(b)).not.toContain('query_fetch')
  })
})

describe('QueryRunner.run: errors', () => {
  it('a failure to start sets runError and leaves the runner idle', async () => {
    install({
      handlers: {
        query_run: () => {
          throw coreError('database', 'syntax error at or near "SELEC"')
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELEC 1')
    expect(r.runError).toBe('syntax error at or near "SELEC"')
    expect(r.running).toBe(false)
    expect(r.queryId).toBeNull()
    expect(r.statements).toEqual([])
  })

  it('a later run clears the previous error', async () => {
    install({
      handlers: {
        query_run: () => {
          throw new Error('first fails')
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'x')
    expect(r.runError).toBe('first fails')
    install()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.runError).toBeNull()
  })

  it('a failure while polling sets runError and stops', async () => {
    install({
      handlers: {
        query_status: () => {
          throw coreError('connection', 'lost connection')
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    expect(r.runError).toBe('lost connection')
    expect(r.running).toBe(false)
    expect(r.queryId).toBe('q1') // still open so the user can retry or it gets closed with the tab
  })

  it('statement-level errors come through the status, not runError', async () => {
    const err = coreError('database', 'division by zero')
    install({ statuses: [status(true, [stmt(0, [], { state: 'error', error: err })])] })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1/0')
    expect(r.runError).toBeNull()
    expect(r.status?.statements[0].error).toEqual(err)
    expect(r.running).toBe(false)
  })

  it('a failed page fetch is recorded on the result set and not retried automatically', async () => {
    const b = install({
      handlers: {
        query_fetch: () => {
          throw coreError('database', 'cursor closed')
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.info(0, 0).failed).toBe('cursor closed')
    const before = b.calls.filter((c) => c.command === 'query_fetch').length
    r.needRows(0, 0, 0, 10)
    await settle()
    expect(b.calls.filter((c) => c.command === 'query_fetch').length).toBe(before)
  })

  it('a later successful fetch of another page clears the failure', async () => {
    let fail = true
    install({
      statuses: [status(true, [stmt(0, [set({ loaded: 1200, complete: true })], { state: 'done' })])],
      totalRows: 1200,
      handlers: {
        query_fetch: ({ offset, limit }) => {
          if (fail) {
            fail = false
            throw new Error('transient')
          }
          const off = Number(offset)
          const n = Math.min(Number(limit), 1200 - off)
          return { rows: rows(off, n), offset: off, loaded: 1200, complete: off + n >= 1200 }
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.info(0, 0).failed).toBe('transient')
    r.needRows(0, 0, 500, 1000)
    await settle()
    expect(r.info(0, 0).failed).toBeNull()
    expect(r.getRow(0, 0, 500)).toEqual([500, 'row 500'])
  })
})

describe('QueryRunner paging: needRows', () => {
  async function bigResult() {
    const b = install({
      statuses: [status(true, [stmt(0, [set({ loaded: 1300, complete: true })], { state: 'done' })])],
      totalRows: 1300,
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    b.fetches.length = 0
    return { r, b }
  }

  it('loads the pages covering the requested rows', async () => {
    const { r, b } = await bigResult()
    r.needRows(0, 0, 600, 1100)
    await settle()
    expect(b.fetches).toEqual([[500, PAGE], [1000, PAGE]])
    expect(r.getRow(0, 0, 500)).toEqual([500, 'row 500'])
    expect(r.getRow(0, 0, 1099)).toEqual([1099, 'row 1099'])
  })

  it('does not refetch pages already loaded', async () => {
    const { r, b } = await bigResult()
    r.needRows(0, 0, 0, 500)
    r.needRows(0, 0, 10, 20)
    await settle()
    expect(b.fetches).toEqual([])
  })

  it('does not fetch a page twice while it is in flight', async () => {
    const { r, b } = await bigResult()
    r.needRows(0, 0, 500, 600)
    r.needRows(0, 0, 500, 600)
    r.needRows(0, 0, 550, 560)
    await settle()
    expect(b.fetches).toEqual([[500, PAGE]])
  })

  it('the end of the range is exclusive', async () => {
    const { r, b } = await bigResult()
    r.needRows(0, 0, 0, 1000) // rows 0..999 -> pages 0,1; page 2 not needed
    await settle()
    expect(b.fetches).toEqual([[500, PAGE]])
  })

  it('an empty or inverted range still loads the page of its start', async () => {
    const { r, b } = await bigResult()
    r.needRows(0, 0, 500, 500)
    await settle()
    expect(b.fetches).toEqual([[500, PAGE]])
  })

  it('the last, short page is read correctly', async () => {
    const { r } = await bigResult()
    r.needRows(0, 0, 1000, 1300)
    await settle()
    expect(r.getRow(0, 0, 1299)).toEqual([1299, 'row 1299'])
    expect(r.getRow(0, 0, 1300)).toBeUndefined()
  })

  it('a short page of an incomplete result is fetched again later (more rows may have arrived)', async () => {
    const b = install({
      statuses: [status(false, [stmt(0, [set({ loaded: 3 })])]), status(false, [stmt(0, [set({ loaded: 3 })])])],
      totalRows: 3,
      handlers: {
        query_fetch: ({ offset }) => ({ rows: rows(Number(offset), 3), offset: Number(offset), loaded: 3, complete: false }),
      },
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await settle(30)
    const first = b.calls.filter((c) => c.command === 'query_fetch').length
    expect(first).toBe(1)
    r.needRows(0, 0, 0, 10)
    await settle(30)
    expect(b.calls.filter((c) => c.command === 'query_fetch').length).toBe(first + 1)
    await r.close()
    await vi.advanceTimersByTimeAsync(200)
    await p
  })

  it('does nothing without a running/open query', async () => {
    const b = install()
    const r = new QueryRunner()
    r.needRows(0, 0, 0, 10)
    await settle()
    expect(commands(b)).not.toContain('query_fetch')
  })

  it('updates the loaded count from fetched rows', async () => {
    install({
      statuses: [status(true, [stmt(0, [set({ loaded: 0, complete: false })], { state: 'done' })])],
      handlers: {
        query_fetch: ({ offset }) => ({ rows: rows(Number(offset), 7), offset: Number(offset), loaded: 7, complete: true }),
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.info(0, 0).loaded).toBe(7)
    expect(r.info(0, 0).complete).toBe(true)
    expect(r.rowCount(0, 0)).toBe(7)
  })
})

describe('QueryRunner.cancel', () => {
  it('asks the backend to cancel a running query', async () => {
    const b = install({ statuses: [status(false, [stmt(0, [set()])])] })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT pg_sleep(60)')
    await settle(10)
    expect(r.running).toBe(true)
    await r.cancel()
    expect(b.calls.find((c) => c.command === 'query_cancel')?.args).toEqual({ queryId: 'q1' })
    await r.close()
    await vi.advanceTimersByTimeAsync(200)
    await p
  })

  it('the run ends once the backend reports the cancellation', async () => {
    const b = install({
      statuses: [
        status(false, [stmt(0, [set()])]),
        status(true, [stmt(0, [], { state: 'cancelled', error: coreError('cancelled', 'cancelled') })]),
      ],
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT pg_sleep(60)')
    await settle(10)
    await r.cancel()
    await vi.advanceTimersByTimeAsync(100)
    await p
    expect(r.running).toBe(false)
    expect(r.status?.statements[0].state).toBe('cancelled')
    expect(commands(b)).toContain('query_cancel')
  })

  it('does nothing when nothing is running', async () => {
    const b = install()
    const r = new QueryRunner()
    await r.cancel()
    expect(commands(b)).not.toContain('query_cancel')
    await r.run('s1', 'app', 'SELECT 1')
    await r.cancel() // finished
    expect(commands(b)).not.toContain('query_cancel')
  })

  it('records a failed cancel in runError', async () => {
    install({
      statuses: [status(false, [stmt(0, [set()])])],
      handlers: {
        query_cancel: () => {
          throw coreError('other', 'cannot cancel')
        },
      },
    })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await settle(10)
    await r.cancel()
    expect(r.runError).toBe('cannot cancel')
    await r.close()
    await vi.advanceTimersByTimeAsync(200)
    await p
  })
})

describe('QueryRunner.close', () => {
  it('releases the backend query and clears everything', async () => {
    const b = install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    await r.close()
    expect(b.calls.find((c) => c.command === 'query_close')?.args).toEqual({ queryId: 'q1' })
    expect(r.queryId).toBeNull()
    expect(r.status).toBeNull()
    expect(r.statements).toEqual([])
    expect(r.running).toBe(false)
    expect(r.getRow(0, 0, 0)).toBeUndefined()
    expect(r.info(0, 0).columns).toEqual([])
  })

  it('bumps the revision so the grid redraws', async () => {
    install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    const before = r.revision
    await r.close()
    expect(r.revision).toBeGreaterThan(before)
  })

  it('is safe to call repeatedly and when idle', async () => {
    const b = install()
    const r = new QueryRunner()
    await r.close()
    await r.run('s1', 'app', 'SELECT 1')
    await r.close()
    await r.close()
    expect(b.calls.filter((c) => c.command === 'query_close')).toHaveLength(1)
  })

  it('swallows a backend failure to close', async () => {
    install({
      handlers: {
        query_close: () => {
          throw new Error('already gone')
        },
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await expect(r.close()).resolves.toBeUndefined()
    expect(r.queryId).toBeNull()
  })

  it('stops an in-flight polling loop', async () => {
    const b = install({ statuses: [status(false, [stmt(0, [set()])])] })
    const r = new QueryRunner()
    const p = r.run('s1', 'app', 'SELECT 1')
    await settle(10)
    await r.close()
    const polls = commands(b).filter((c) => c === 'query_status').length
    await vi.advanceTimersByTimeAsync(1000)
    await p
    expect(commands(b).filter((c) => c === 'query_status').length).toBe(polls)
    expect(r.running).toBe(false)
  })

  it('a page that arrives after close is discarded', async () => {
    let release: (() => void) | undefined
    install({
      statuses: [status(true, [stmt(0, [set({ loaded: 3, complete: true })], { state: 'done' })])],
      handlers: {
        query_fetch: () => new Promise((res) => (release = () => res({ rows: rows(0, 3), offset: 0, loaded: 3, complete: true }))),
      },
    })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    await r.close()
    const revision = r.revision
    release?.()
    await settle()
    expect(r.getRow(0, 0, 0)).toBeUndefined()
    expect(r.revision).toBe(revision)
  })
})

describe('QueryRunner: re-running', () => {
  it('closes the previous query before starting the next', async () => {
    const b = install()
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    b.calls.length = 0
    await r.run('s1', 'app', 'SELECT 2')
    const order = commands(b)
    expect(order.indexOf('query_close')).toBeGreaterThanOrEqual(0)
    expect(order.indexOf('query_close')).toBeLessThan(order.indexOf('query_run'))
  })

  it('starts from a clean slate (old rows and errors are gone)', async () => {
    install({ statuses: [status(true, [stmt(0, [set({ loaded: 3, complete: true })], { state: 'done' })])] })
    const r = new QueryRunner()
    await r.run('s1', 'app', 'SELECT 1')
    await settle()
    expect(r.getRow(0, 0, 0)).toBeDefined()
    install({
      handlers: {
        query_run: () => {
          throw new Error('nope')
        },
      },
    })
    await r.run('s1', 'app', 'SELECT 2')
    expect(r.getRow(0, 0, 0)).toBeUndefined()
    expect(r.runError).toBe('nope')
  })

  it('a superseded run does not overwrite the newer one, and its query is closed', async () => {
    const queryRunResolvers: ((v: unknown) => void)[] = []
    let n = 0
    const b = install({
      handlers: {
        query_run: () => {
          const id = `q${++n}`
          return new Promise((res) => queryRunResolvers.push(() => res({ queryId: id, statements: [span(n)] })))
        },
        query_status: ({ queryId }) => status(true, [stmt(0, [set({ loaded: 1, complete: true })], { state: 'done' })], String(queryId)),
        query_fetch: ({ offset }) => ({ rows: rows(Number(offset), 1), offset: Number(offset), loaded: 1, complete: true }),
      },
    })
    const r = new QueryRunner()
    const first = r.run('s1', 'app', 'slow')
    await settle()
    const second = r.run('s1', 'app', 'fast')
    await settle()
    expect(queryRunResolvers).toHaveLength(2)
    queryRunResolvers[1](undefined) // the newer run answers first
    await second
    expect(r.queryId).toBe('q2')
    queryRunResolvers[0](undefined) // the older answer arrives late
    await first
    await settle()
    expect(r.queryId).toBe('q2')
    expect(b.calls.some((c) => c.command === 'query_close' && c.args.queryId === 'q1')).toBe(true)
    expect(r.running).toBe(false)
  })

  it('a superseded run\'s start failure does not set runError', async () => {
    const rejecters: ((e: unknown) => void)[] = []
    let n = 0
    install({
      handlers: {
        query_run: () => {
          n++
          if (n === 1) return new Promise((_res, rej) => rejecters.push(rej))
          return { queryId: 'q2', statements: [span(0)] }
        },
        query_status: () => status(true, [stmt(0, [set({ loaded: 1, complete: true })], { state: 'done' })], 'q2'),
      },
    })
    const r = new QueryRunner()
    const first = r.run('s1', 'app', 'one')
    await settle()
    await r.run('s1', 'app', 'two')
    rejecters[0](new Error('stale failure'))
    await first
    expect(r.runError).toBeNull()
  })
})
