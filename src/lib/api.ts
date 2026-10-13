// Typed wrappers for every Tauri command (src-tauri/src/commands.rs).
// Rejections carry a CoreError object; use `errorMessage` to display one.

import { invoke } from '@tauri-apps/api/core'
import type {
  ApplyResult,
  BrowseRequest,
  ChangeSet,
  ColumnValue,
  ConnectionConfig,
  CoreError,
  CoreInfo,
  CsvImport,
  CsvPreview,
  Engine,
  ExportFormat,
  ExportSource,
  JobProgress,
  Page,
  QueryRun,
  QueryStatus,
  RowCount,
  RowsPage,
  SchemaSnapshot,
  Secrets,
  SessionInfo,
  StatementSpan,
  TableInfo,
  TableRef,
  TableStructure,
  TestResult,
  Value,
} from './types'

export function isCoreError(e: unknown): e is CoreError {
  return typeof e === 'object' && e !== null && 'kind' in e && 'message' in e
}

export function errorMessage(e: unknown): string {
  if (isCoreError(e)) return e.message
  if (e instanceof Error) return e.message
  return String(e)
}

export const api = {
  appInfo: () => invoke<CoreInfo>('app_info'),

  // connections
  connectionsList: () => invoke<ConnectionConfig[]>('connections_list'),
  connectionSave: (config: ConnectionConfig, secrets: Secrets | null = null) =>
    invoke<ConnectionConfig>('connection_save', { config, secrets }),
  connectionDelete: (id: string) => invoke<void>('connection_delete', { id }),
  connectionTest: (config: ConnectionConfig, secrets: Secrets | null = null) =>
    invoke<TestResult>('connection_test', { config, secrets }),
  connectionOpen: (id: string, secrets: Secrets | null = null) =>
    invoke<SessionInfo>('connection_open', { id, secrets }),
  connectionClose: (sessionId: string) => invoke<void>('connection_close', { sessionId }),

  // schema
  schemaDatabases: (sessionId: string) => invoke<string[]>('schema_databases', { sessionId }),
  schemaSchemas: (sessionId: string, database: string) =>
    invoke<string[]>('schema_schemas', { sessionId, database }),
  schemaTables: (sessionId: string, database: string, schema: string | null) =>
    invoke<TableInfo[]>('schema_tables', { sessionId, database, schema }),
  schemaSnapshot: (sessionId: string, database: string, refresh: boolean) =>
    invoke<SchemaSnapshot>('schema_snapshot', { sessionId, database, refresh }),
  tableStructure: (sessionId: string, table: TableRef) =>
    invoke<TableStructure>('table_structure', { sessionId, table }),

  // browsing
  browsePage: (sessionId: string, request: BrowseRequest, offset: number, limit: number) =>
    invoke<Page>('browse_page', { sessionId, request, offset, limit }),
  browseCount: (sessionId: string, request: BrowseRequest) =>
    invoke<RowCount>('browse_count', { sessionId, request }),
  cellFetch: (sessionId: string, table: TableRef, key: ColumnValue[], column: string) =>
    invoke<Value>('cell_fetch', { sessionId, table, key, column }),

  // SQL editor
  sqlSplit: (sql: string, engine: Engine) => invoke<StatementSpan[]>('sql_split', { sql, engine }),
  queryRun: (sessionId: string, database: string | null, sql: string) =>
    invoke<QueryRun>('query_run', { sessionId, database, sql }),
  queryStatus: (queryId: string) => invoke<QueryStatus>('query_status', { queryId }),
  queryFetch: (queryId: string, statementIndex: number, setIndex: number, offset: number, limit: number) =>
    invoke<RowsPage>('query_fetch', { queryId, statementIndex, setIndex, offset, limit }),
  queryCancel: (queryId: string) => invoke<void>('query_cancel', { queryId }),
  queryClose: (queryId: string) => invoke<void>('query_close', { queryId }),

  // editing
  changesPreview: (sessionId: string, changes: ChangeSet) =>
    invoke<string[]>('changes_preview', { sessionId, changes }),
  changesApply: (sessionId: string, changes: ChangeSet) =>
    invoke<ApplyResult>('changes_apply', { sessionId, changes }),

  // export / import
  exportStart: (sessionId: string, source: ExportSource, format: ExportFormat, path: string) =>
    invoke<string>('export_start', { sessionId, source, format, path }),
  csvPreview: (path: string, delimiter: string | null = null) =>
    invoke<CsvPreview>('csv_preview', { path, delimiter }),
  importStart: (sessionId: string, importSpec: CsvImport) =>
    invoke<string>('import_start', { sessionId, import: importSpec }),
  jobStatus: (jobId: string) => invoke<JobProgress>('job_status', { jobId }),
  jobCancel: (jobId: string) => invoke<void>('job_cancel', { jobId }),
}
