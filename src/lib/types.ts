// TypeScript mirror of crates/dbcore/src/{model,value}.rs and src-tauri/src/commands.rs.
// Keep in sync with the Rust side; field names are camelCase on the wire.

export type Engine = 'sqlite' | 'postgres' | 'mysql' | 'mariadb' | 'mongodb'

// ---------------------------------------------------------------------------
// Values (see value.rs for the encoding table)
// ---------------------------------------------------------------------------

export type Value =
  | null
  | boolean
  | number
  | string
  | { $int: string }
  | { $float: 'NaN' | 'Infinity' | '-Infinity' }
  | { $dec: string }
  | { $text: string; len: number }
  | { $bytes: string; len: number }
  | { $json: unknown }
  | { $date: string }
  | { $time: string }
  | { $datetime: string }
  | { $uuid: string }
  | { $oid: string }
  | { $other: string; type: string }

export type Row = Value[]

export type CellInput =
  | { kind: 'null' }
  | { kind: 'default' }
  | { kind: 'text'; value: string }
  | { kind: 'bytes'; value: string } // base64
  | { kind: 'json'; value: unknown }
  | { kind: 'original'; value: Value }

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export type ErrorKind =
  | 'database'
  | 'connection'
  | 'ssh'
  | 'cancelled'
  | 'unsupported'
  | 'invalidInput'
  | 'notFound'
  | 'io'
  | 'other'

export interface CoreError {
  kind: ErrorKind
  message: string
  code: string | null
  statement: string | null
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

export type EnvTag = 'local' | 'development' | 'staging' | 'production'
export type SslMode = 'disable' | 'prefer' | 'require' | 'verifyCa' | 'verifyFull'

export interface SslConfig {
  mode: SslMode
  caCert?: string | null
  clientCert?: string | null
  clientKey?: string | null
}

export type SshAuth = { kind: 'password' } | { kind: 'key'; path: string } | { kind: 'agent' }

export interface SshConfig {
  host: string
  port: number
  user: string
  auth: SshAuth
  jump?: SshConfig | null
}

export interface ConnectionConfig {
  id: string
  name: string
  engine: Engine
  host: string
  port: number
  user: string
  database: string
  filePath?: string | null
  url?: string | null
  ssl: SslConfig
  ssh?: SshConfig | null
  env: EnvTag
  color?: string | null
  readOnly: boolean
  options: Record<string, string>
}

export interface Secrets {
  dbPassword?: string | null
  sshPassword?: string | null
  sshPassphrase?: string | null
  jumpPassword?: string | null
  jumpPassphrase?: string | null
}

export interface CoreInfo {
  version: string
  engines: Engine[]
}

export interface TestResult {
  serverVersion: string
  elapsedMs: number
}

export interface SessionInfo {
  sessionId: string
  config: ConnectionConfig
  serverVersion: string
  databases: string[]
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

export interface TableRef {
  database: string
  schema?: string | null
  name: string
}

export type TableKind =
  | 'table'
  | 'view'
  | 'materializedView'
  | 'foreignTable'
  | 'virtual'
  | 'sequence'
  | 'collection'
  | 'timeSeries'
  | 'system'

/** TableRef fields are flattened into TableInfo. */
export interface TableInfo extends TableRef {
  kind: TableKind
  estimatedRows: number | null
  comment: string | null
}

export type TypeFamily =
  | 'boolean'
  | 'integer'
  | 'float'
  | 'decimal'
  | 'text'
  | 'binary'
  | 'json'
  | 'date'
  | 'time'
  | 'dateTime'
  | 'uuid'
  | 'enum'
  | 'array'
  | 'spatial'
  | 'objectId'
  | 'document'
  | 'other'

export interface ColumnInfo {
  name: string
  dataType: string
  family: TypeFamily
  nullable: boolean
  default: string | null
  isPrimaryKey: boolean
  isAutoIncrement: boolean
  isGenerated: boolean
  enumValues: string[] | null
  comment: string | null
}

export interface IndexInfo {
  name: string
  columns: string[]
  unique: boolean
  primary: boolean
  method: string | null
}

export interface ForeignKeyInfo {
  name: string
  columns: string[]
  referenced: TableRef
  referencedColumns: string[]
  onUpdate: string | null
  onDelete: string | null
}

export type Editability = { kind: 'editable' } | { kind: 'readOnly'; reason: string }

export interface TableStructure {
  table: TableRef
  kind: TableKind
  columns: ColumnInfo[]
  primaryKey: string[]
  indexes: IndexInfo[]
  foreignKeys: ForeignKeyInfo[]
  editability: Editability
  ddl: string | null
}

export interface SchemaSnapshot {
  database: string
  /** schema name → tables; engines without schemas use the "" key */
  schemas: Record<string, TableInfo[]>
  /** table name (schema-qualified where applicable) → column names */
  columns: Record<string, string[]>
  fetchedAt: number
}

// ---------------------------------------------------------------------------
// Results and browsing
// ---------------------------------------------------------------------------

export interface ColumnMeta {
  name: string
  typeName: string
  family: TypeFamily
}

export interface Page {
  columns: ColumnMeta[]
  rows: Row[]
  offset: number
  query: string
  elapsedMs: number
}

export interface RowCount {
  exact: number | null
  estimate: number | null
}

export type FilterOp =
  | 'eq'
  | 'neq'
  | 'lt'
  | 'lte'
  | 'gt'
  | 'gte'
  | 'contains'
  | 'notContains'
  | 'startsWith'
  | 'endsWith'
  | 'like'
  | 'in'
  | 'notIn'
  | 'isNull'
  | 'isNotNull'

export interface Filter {
  column: string
  op: FilterOp
  value?: string | null
}

export interface Sort {
  column: string
  descending: boolean
}

export interface BrowseRequest {
  table: TableRef
  filters: Filter[]
  matchAll: boolean
  rawWhere?: string | null
  sort: Sort[]
  columns: string[]
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

export interface ColumnValue {
  column: string
  value: Value
}

export interface ColumnInput {
  column: string
  value: CellInput
}

export type RowChange =
  | { kind: 'insert'; values: ColumnInput[] }
  | { kind: 'update'; key: ColumnValue[]; original: ColumnValue[]; set: ColumnInput[] }
  | { kind: 'delete'; key: ColumnValue[] }

export interface ChangeSet {
  table: TableRef
  changes: RowChange[]
}

export interface ApplyResult {
  statements: number
  rowsAffected: number
}

// ---------------------------------------------------------------------------
// SQL editor
// ---------------------------------------------------------------------------

export type StatementKind = 'read' | 'write' | 'ddl' | 'other'

export interface StatementSpan {
  /** UTF-8 byte offsets into the script */
  start: number
  end: number
  sql: string
  kind: StatementKind
  destructive: boolean
}

export interface QueryRun {
  queryId: string
  statements: StatementSpan[]
}

export type StatementState = 'pending' | 'running' | 'done' | 'error' | 'cancelled'

export interface ResultSetStatus {
  setIndex: number
  columns: ColumnMeta[]
  loaded: number
  complete: boolean
  /** non-final statements of a script are read up to 10,000 rows, then closed */
  truncated: boolean
}

export interface StatementStatus {
  index: number
  sql: string
  state: StatementState
  error: CoreError | null
  affected: number | null
  sets: ResultSetStatus[]
  elapsedMs: number
}

export interface QueryStatus {
  queryId: string
  finished: boolean
  statements: StatementStatus[]
}

export interface RowsPage {
  rows: Row[]
  offset: number
  loaded: number
  complete: boolean
}

// ---------------------------------------------------------------------------
// Export / import
// ---------------------------------------------------------------------------

export type ExportSource =
  | { kind: 'table'; request: BrowseRequest }
  | { kind: 'query'; database: string | null; sql: string }

export type ExportFormat =
  | { kind: 'csv'; delimiter: string; header: boolean; nullAs: string }
  | { kind: 'json'; pretty: boolean }
  | { kind: 'jsonLines' }
  | { kind: 'sql'; table: string; rowsPerInsert: number }

export interface CsvPreview {
  headers: string[]
  rows: string[][]
  delimiter: string
}

export interface CsvImport {
  path: string
  table: TableRef
  delimiter: string
  hasHeader: boolean
  mapping: (string | null)[]
  nullAs?: string | null
}

export interface JobProgress {
  rows: number
  bytes: number
  done: boolean
  cancelled: boolean
  error: string | null
  /** Non-fatal note about the finished job (e.g. long values truncated in a query export). */
  warning?: string | null
}

// ---------------------------------------------------------------------------
// SQL files, dump import, manual transactions, table admin
// ---------------------------------------------------------------------------

/** Runs a .sql file (e.g. a dump) statement by statement as a background job.
 *  JobProgress.rows counts executed statements, .bytes the bytes of SQL consumed. */
export interface SqlImport {
  path: string
  /** Database to run in; null = the connection's default. */
  database: string | null
  /** false = keep going after a failed statement (errors are counted into JobProgress.warning). */
  stopOnError: boolean
}

/** Drop / truncate / rename a table (or view / collection). */
export type TableAction = { kind: 'drop' } | { kind: 'truncate' } | { kind: 'rename'; newName: string }

// ---------------------------------------------------------------------------
// Query log (console pane) and AI assistant
// ---------------------------------------------------------------------------

/** One statement the app sent to a server (crates/dbcore/src/querylog.rs). */
export interface LogEntry {
  /** Strictly increasing; poll with the last one you saw. */
  seq: number
  /** Unix time in milliseconds. */
  atMs: number
  sql: string
}

export interface AssistantMessage {
  role: 'user' | 'assistant'
  content: string
}

export interface AssistantRequest {
  /** Lets `assistantCancel` stop this request. */
  requestId?: string | null
  engine: Engine
  serverVersion: string
  database: string
  /** One `schema.table(col, col)` per line. */
  schemaContext: string
  messages: AssistantMessage[]
  /** Defaults to claude-sonnet-5-5 on the Rust side. */
  model?: string | null
}

export interface AssistantReply {
  text: string
}
