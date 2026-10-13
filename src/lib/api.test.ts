import { beforeEach, describe, expect, it, vi } from 'vitest'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

import { api, errorMessage, isCoreError } from './api'
import type {
  AssistantRequest,
  BrowseRequest,
  ChangeSet,
  ConnectionConfig,
  CoreError,
  CsvImport,
  ExportFormat,
  ExportSource,
  TableRef,
} from './types'

beforeEach(() => {
  core.invoke.mockReset()
  core.invoke.mockResolvedValue(undefined)
})

const coreError = (over: Partial<CoreError> = {}): CoreError => ({
  kind: 'database',
  message: 'relation "x" does not exist',
  code: '42P01',
  statement: 'SELECT * FROM x',
  ...over,
})

describe('isCoreError', () => {
  it('accepts objects with kind and message', () => {
    expect(isCoreError(coreError())).toBe(true)
    expect(isCoreError({ kind: 'io', message: 'disk full' })).toBe(true)
  })

  it('rejects objects missing either field', () => {
    expect(isCoreError({ kind: 'io' })).toBe(false)
    expect(isCoreError({ message: 'x' })).toBe(false)
    expect(isCoreError({})).toBe(false)
  })

  it('rejects primitives, null and undefined', () => {
    for (const x of [null, undefined, 'boom', 42, true, Symbol('s')]) expect(isCoreError(x)).toBe(false)
  })

  it('a plain Error is not a CoreError', () => {
    expect(isCoreError(new Error('x'))).toBe(false)
  })

  it('does not look at the field types (shape check only)', () => {
    expect(isCoreError({ kind: 1, message: 2 })).toBe(true)
  })
})

describe('errorMessage', () => {
  it('uses the message of a CoreError', () => {
    expect(errorMessage(coreError({ message: 'permission denied' }))).toBe('permission denied')
  })

  it('uses the message of an Error', () => {
    expect(errorMessage(new Error('plain'))).toBe('plain')
    expect(errorMessage(new TypeError('typed'))).toBe('typed')
  })

  it('stringifies anything else', () => {
    expect(errorMessage('just a string')).toBe('just a string')
    expect(errorMessage(404)).toBe('404')
    expect(errorMessage(null)).toBe('null')
    expect(errorMessage(undefined)).toBe('undefined')
    expect(errorMessage(true)).toBe('true')
  })

  it('prefers CoreError handling over Error handling for error-like objects', () => {
    const e = Object.assign(new Error('from Error'), { kind: 'other', message: 'from Error' })
    expect(errorMessage(e)).toBe('from Error')
  })

  it('an object without a message falls back to "[object Object]"', () => {
    expect(errorMessage({ foo: 1 })).toBe('[object Object]')
  })

  it('handles every error kind the backend can send', () => {
    const kinds = ['database', 'connection', 'ssh', 'cancelled', 'unsupported', 'invalidInput', 'notFound', 'io', 'other'] as const
    for (const kind of kinds) expect(errorMessage(coreError({ kind, message: `m-${kind}` }))).toBe(`m-${kind}`)
  })
})

describe('api wrappers: command names and argument keys', () => {
  const table: TableRef = { database: 'db', schema: 'public', name: 'users' }
  const request: BrowseRequest = { table, filters: [], matchAll: true, sort: [], columns: [] }
  const config = { id: 'c1', name: 'n' } as ConnectionConfig
  const changes: ChangeSet = { table, changes: [] }
  const source: ExportSource = { kind: 'table', request }
  const format: ExportFormat = { kind: 'jsonLines' }
  const csvImport: CsvImport = { path: '/x.csv', table, delimiter: ',', hasHeader: true, mapping: ['a', null] }
  const assistantReq: AssistantRequest = {
    engine: 'postgres',
    serverVersion: '16',
    database: 'db',
    schemaContext: '',
    messages: [{ role: 'user', content: 'hi' }],
  }

  const cases: [string, () => Promise<unknown>, string, Record<string, unknown> | undefined][] = [
    ['appInfo', () => api.appInfo(), 'app_info', undefined],
    ['connectionsList', () => api.connectionsList(), 'connections_list', undefined],
    ['connectionSave', () => api.connectionSave(config), 'connection_save', { config, secrets: null }],
    ['connectionSave+secrets', () => api.connectionSave(config, { dbPassword: 'p' }), 'connection_save', { config, secrets: { dbPassword: 'p' } }],
    ['connectionDelete', () => api.connectionDelete('c1'), 'connection_delete', { id: 'c1' }],
    ['connectionTest', () => api.connectionTest(config), 'connection_test', { config, secrets: null }],
    ['connectionOpen', () => api.connectionOpen('c1'), 'connection_open', { id: 'c1', secrets: null }],
    ['connectionClose', () => api.connectionClose('s1'), 'connection_close', { sessionId: 's1' }],
    ['schemaDatabases', () => api.schemaDatabases('s1'), 'schema_databases', { sessionId: 's1' }],
    ['schemaSchemas', () => api.schemaSchemas('s1', 'db'), 'schema_schemas', { sessionId: 's1', database: 'db' }],
    ['schemaTables', () => api.schemaTables('s1', 'db', null), 'schema_tables', { sessionId: 's1', database: 'db', schema: null }],
    ['schemaSnapshot', () => api.schemaSnapshot('s1', 'db', true), 'schema_snapshot', { sessionId: 's1', database: 'db', refresh: true }],
    ['tableStructure', () => api.tableStructure('s1', table), 'table_structure', { sessionId: 's1', table }],
    ['browsePage', () => api.browsePage('s1', request, 100, 50), 'browse_page', { sessionId: 's1', request, offset: 100, limit: 50 }],
    ['browseCount', () => api.browseCount('s1', request), 'browse_count', { sessionId: 's1', request }],
    ['cellFetch', () => api.cellFetch('s1', table, [{ column: 'id', value: 1 }], 'bio'), 'cell_fetch', { sessionId: 's1', table, key: [{ column: 'id', value: 1 }], column: 'bio' }],
    ['sqlSplit', () => api.sqlSplit('SELECT 1', 'postgres'), 'sql_split', { sql: 'SELECT 1', engine: 'postgres' }],
    ['queryRun', () => api.queryRun('s1', null, 'SELECT 1'), 'query_run', { sessionId: 's1', database: null, sql: 'SELECT 1' }],
    ['queryStatus', () => api.queryStatus('q1'), 'query_status', { queryId: 'q1' }],
    ['queryFetch', () => api.queryFetch('q1', 2, 1, 500, 500), 'query_fetch', { queryId: 'q1', statementIndex: 2, setIndex: 1, offset: 500, limit: 500 }],
    ['queryCancel', () => api.queryCancel('q1'), 'query_cancel', { queryId: 'q1' }],
    ['queryClose', () => api.queryClose('q1'), 'query_close', { queryId: 'q1' }],
    ['changesPreview', () => api.changesPreview('s1', changes), 'changes_preview', { sessionId: 's1', changes }],
    ['changesApply', () => api.changesApply('s1', changes), 'changes_apply', { sessionId: 's1', changes }],
    ['exportStart', () => api.exportStart('s1', source, format, '/out'), 'export_start', { sessionId: 's1', source, format, path: '/out' }],
    ['csvPreview', () => api.csvPreview('/x.csv'), 'csv_preview', { path: '/x.csv', delimiter: null }],
    ['csvPreview+delimiter', () => api.csvPreview('/x.csv', ';'), 'csv_preview', { path: '/x.csv', delimiter: ';' }],
    ['importStart', () => api.importStart('s1', csvImport), 'import_start', { sessionId: 's1', import: csvImport }],
    ['jobStatus', () => api.jobStatus('j1'), 'job_status', { jobId: 'j1' }],
    ['jobCancel', () => api.jobCancel('j1'), 'job_cancel', { jobId: 'j1' }],
    ['queryLogSince', () => api.queryLogSince(12), 'query_log_since', { seq: 12 }],
    ['queryLogClear', () => api.queryLogClear(), 'query_log_clear', undefined],
    ['assistantAsk', () => api.assistantAsk(assistantReq), 'assistant_ask', { req: assistantReq }],
    ['assistantCancel', () => api.assistantCancel('r1'), 'assistant_cancel', { requestId: 'r1' }],
    ['assistantKeyStatus', () => api.assistantKeyStatus(), 'assistant_key_status', undefined],
    ['assistantKeySet', () => api.assistantKeySet('sk-x'), 'assistant_key_set', { key: 'sk-x' }],
  ]

  it.each(cases)('%s', async (_name, call, command, args) => {
    await call()
    expect(core.invoke).toHaveBeenCalledTimes(1)
    if (args === undefined) expect(core.invoke).toHaveBeenCalledWith(command)
    else expect(core.invoke).toHaveBeenCalledWith(command, args)
  })

  it('returns what invoke resolves with', async () => {
    core.invoke.mockResolvedValue({ version: '1.2.3', engines: ['sqlite'] })
    expect(await api.appInfo()).toEqual({ version: '1.2.3', engines: ['sqlite'] })
  })

  it('propagates a rejection unchanged (a CoreError object)', async () => {
    const err = coreError({ kind: 'connection', message: 'refused' })
    core.invoke.mockRejectedValue(err)
    await expect(api.connectionOpen('c1')).rejects.toBe(err)
  })

  it('covers every method of the api object', () => {
    const covered = new Set(cases.map(([name]) => name.split('+')[0]))
    for (const method of Object.keys(api)) expect(covered, `api.${method} has no wrapper test`).toContain(method)
  })
})
