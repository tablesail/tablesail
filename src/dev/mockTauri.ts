// Dev-only stand-in for the Tauri runtime, so the UI can be opened in a plain
// browser (npm run dev → http://localhost:5173) with realistic sample data.
// Loaded by main.ts only when import.meta.env.DEV and no real Tauri runtime exists.

import type {
  BrowseRequest,
  ColumnInfo,
  ColumnMeta,
  ConnectionConfig,
  CoreError,
  LogEntry,
  QueryStatus,
  Row,
  SchemaSnapshot,
  SessionInfo,
  StatementSpan,
  StatementStatus,
  TableInfo,
  TableRef,
  TableStructure,
  TypeFamily,
  Value,
} from '../lib/types'

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms))

const baseConfig = {
  user: 'tablesail',
  database: 'tablesail',
  ssl: { mode: 'prefer' as const },
  ssh: null,
  color: null,
  readOnly: false,
  options: {},
}

let connections: ConnectionConfig[] = [
  { ...baseConfig, id: 'c-pg', name: 'Local Postgres', engine: 'postgres', host: '127.0.0.1', port: 25432, env: 'local' },
  { ...baseConfig, id: 'c-my', name: 'Local MySQL', engine: 'mysql', host: '127.0.0.1', port: 23306, env: 'local' },
  { ...baseConfig, id: 'c-maria', name: 'MariaDB', engine: 'mariadb', host: '127.0.0.1', port: 23307, env: 'development' },
  { ...baseConfig, id: 'c-mongo', name: 'Mongo', engine: 'mongodb', host: '127.0.0.1', port: 27117, env: 'development', user: '' },
  { ...baseConfig, id: 'c-stg', name: 'Staging API', engine: 'postgres', host: 'db.staging.internal', port: 5432, env: 'staging' },
  { ...baseConfig, id: 'c-prod', name: 'Production', engine: 'postgres', host: 'db.prod.internal', port: 5432, env: 'production', readOnly: true },
  { ...baseConfig, id: 'c-lite', name: 'nasty.db', engine: 'sqlite', host: '', port: 0, user: '', database: '', filePath: '~/data/nasty.db', env: 'local' },
]

const col = (name: string, dataType: string, family: TypeFamily, extra: Partial<ColumnInfo> = {}): ColumnInfo => ({
  name,
  dataType,
  family,
  nullable: true,
  default: null,
  isPrimaryKey: false,
  isAutoIncrement: false,
  isGenerated: false,
  enumValues: null,
  comment: null,
  ...extra,
})

const TABLE_COLUMNS: Record<string, ColumnInfo[]> = {
  big_table: [
    col('id', 'bigint', 'integer', { nullable: false, isPrimaryKey: true }),
    col('uuid', 'uuid', 'uuid', { nullable: false }),
    col('name', 'text', 'text', { nullable: false }),
    col('amount', 'numeric(12,2)', 'decimal'),
    col('is_active', 'boolean', 'boolean', { nullable: false }),
    col('created_at', 'timestamp with time zone', 'dateTime', { nullable: false }),
    col('payload', 'jsonb', 'json'),
  ],
  customers: [
    col('id', 'integer', 'integer', { nullable: false, isPrimaryKey: true, isAutoIncrement: true, default: "nextval('customers_id_seq')" }),
    col('email', 'citext', 'text', { nullable: false }),
    col('name', 'text', 'text'),
  ],
  orders: [
    col('id', 'integer', 'integer', { nullable: false, isPrimaryKey: true, isAutoIncrement: true }),
    col('customer_id', 'integer', 'integer', { nullable: false }),
    col('total', 'numeric(12,2)', 'decimal', { nullable: false, default: '0' }),
    col('status', 'mood', 'enum', { enumValues: ['sad', 'ok', 'happy'] }),
    col('placed_at', 'timestamp with time zone', 'dateTime', { nullable: false, default: 'now()' }),
  ],
}

const TABLES: TableInfo[] = [
  ['all_types', 'table', 4],
  ['big_table', 'table', 1_000_000],
  ['categories', 'table', 3],
  ['composite_pk', 'table', 3],
  ['computed', 'table', 1],
  ['customers', 'table', 2],
  ['empty_table', 'table', 0],
  ['events', 'table', 2],
  ['MixedCase', 'table', 1],
  ['no_pk', 'table', 5],
  ['orders', 'table', 3],
  ['select', 'table', 1],
  ['unique_only', 'table', 2],
  ['user', 'table', 1],
  ['wide_table', 'table', 1000],
  ['customer_totals', 'view', null],
  ['mood_counts', 'materializedView', null],
].map(([name, kind, rows]) => ({
  database: 'tablesail',
  schema: 'public',
  name: name as string,
  kind: kind as TableInfo['kind'],
  estimatedRows: rows as number | null,
  comment: null,
}))

const BILLING: TableInfo[] = [{ database: 'tablesail', schema: 'billing', name: 'invoices', kind: 'table', estimatedRows: 2, comment: null }]

function columnsFor(table: TableRef): ColumnInfo[] {
  return TABLE_COLUMNS[table.name] ?? TABLE_COLUMNS.big_table
}

function meta(columns: ColumnInfo[]): ColumnMeta[] {
  return columns.map((c) => ({ name: c.name, typeName: c.dataType, family: c.family }))
}

function hex(n: number, len: number) {
  return (Math.imul(n, 2654435761) >>> 0).toString(16).padStart(8, '0').repeat(4).slice(0, len)
}

function cell(c: ColumnInfo, i: number): Value {
  const n = i + 1
  switch (c.name) {
    case 'id':
      return n
    case 'customer_id':
      return (n % 2) + 1
    case 'uuid':
      return { $uuid: `${hex(n, 8)}-${hex(n + 1, 4)}-4${hex(n + 2, 3)}-a${hex(n + 3, 3)}-${hex(n + 4, 12)}` }
    case 'name':
      return c.family === 'text' && TABLE_COLUMNS.customers.includes(c) ? ['Alice', 'Bob'][i % 2] : `row ${n}`
    case 'email':
      return ['a@example.com', 'b@example.com'][i % 2]
    case 'amount':
    case 'total':
      return n % 10 === 0 ? null : { $dec: ((n % 100000) / 100).toFixed(2) }
    case 'is_active':
      return n % 2 === 0
    case 'status':
      return ['happy', 'ok', 'sad'][i % 3]
    case 'created_at':
    case 'placed_at':
      return { $datetime: new Date(Date.UTC(2020, 0, 1) + n * 60000).toISOString().replace('T', ' ').replace('Z', '+00') }
    case 'payload':
      return n % 3 === 0 ? { $json: { n, tags: ['a', 'b'] } } : null
    default:
      return `${c.name} ${n}`
  }
}

function rowCountFor(table: TableRef): number {
  return [...TABLES, ...BILLING].find((t) => t.name === table.name)?.estimatedRows ?? 3
}

// --- Filtering and sorting for small tables, so the e2e suite can see the effect of a filter or a sort.
// Big tables (more than SMALL_TABLE rows) are generated lazily and ignore both.
const SMALL_TABLE = 5000

function plain(v: Value | undefined): string {
  if (v === null || v === undefined) return ''
  if (typeof v === 'object') {
    const inner = Object.values(v as Record<string, unknown>)[0]
    return typeof inner === 'object' ? JSON.stringify(inner) : String(inner)
  }
  return String(v)
}

function compareValues(a: Value | undefined, b: Value | undefined): number {
  if (a === null || a === undefined) return b === null || b === undefined ? 0 : -1
  if (b === null || b === undefined) return 1
  const x = plain(a)
  const y = plain(b)
  const nx = Number(x)
  const ny = Number(y)
  return x !== '' && y !== '' && !Number.isNaN(nx) && !Number.isNaN(ny) ? nx - ny : x.localeCompare(y)
}

function matchesFilter(v: Value | undefined, f: { op: string; value?: string | null }): boolean {
  const text = plain(v).toLowerCase()
  const want = String(f.value ?? '').toLowerCase()
  switch (f.op) {
    case 'eq':
      return text === want
    case 'neq':
      return text !== want
    case 'lt':
      return compareValues(v, f.value as string) < 0
    case 'lte':
      return compareValues(v, f.value as string) <= 0
    case 'gt':
      return compareValues(v, f.value as string) > 0
    case 'gte':
      return compareValues(v, f.value as string) >= 0
    case 'contains':
      return text.includes(want)
    case 'notContains':
      return !text.includes(want)
    case 'startsWith':
      return text.startsWith(want)
    case 'endsWith':
      return text.endsWith(want)
    case 'like':
      return new RegExp('^' + want.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/%/g, '.*') + '$').test(text)
    case 'in':
      return want.split(',').map((x) => x.trim()).includes(text)
    case 'notIn':
      return !want.split(',').map((x) => x.trim()).includes(text)
    case 'isNull':
      return v === null || v === undefined
    case 'isNotNull':
      return v !== null && v !== undefined
    default:
      return true
  }
}

/** All rows of a small table after the request's filters and sort, or null for a big (lazy) table. */
function filteredRows(req: BrowseRequest): Row[] | null {
  const total = rowCountFor(req.table)
  if (total > SMALL_TABLE) return null
  const columns = columnsFor(req.table)
  let rows: Row[] = Array.from({ length: total }, (_, i) => columns.map((c) => cell(c, i)))
  const filters = (req.filters ?? []) as { column: string; op: string; value?: string | null }[]
  if (filters.length) {
    const test = (row: Row, f: (typeof filters)[number]) => matchesFilter(row[columns.findIndex((c) => c.name === f.column)], f)
    rows = rows.filter((row) => (req.matchAll ? filters.every((f) => test(row, f)) : filters.some((f) => test(row, f))))
  }
  const sort = req.sort?.[0]
  if (sort) {
    const i = columns.findIndex((c) => c.name === sort.column)
    if (i >= 0) rows = [...rows].sort((a, b) => (sort.descending ? -1 : 1) * compareValues(a[i], b[i]))
  }
  return rows
}

function structure(table: TableRef): TableStructure {
  const columns = columnsFor(table)
  const info = TABLES.find((t) => t.name === table.name)
  const isView = info?.kind === 'view' || info?.kind === 'materializedView'
  return {
    table,
    kind: info?.kind ?? 'table',
    columns,
    primaryKey: columns.filter((c) => c.isPrimaryKey).map((c) => c.name),
    indexes: [
      { name: `${table.name}_pkey`, columns: ['id'], unique: true, primary: true, method: 'btree' },
      { name: `${table.name}_created_at_idx`, columns: [columns[columns.length - 2]?.name ?? 'id'], unique: false, primary: false, method: 'btree' },
    ],
    foreignKeys:
      table.name === 'orders'
        ? [
            {
              name: 'orders_customer_id_fkey',
              columns: ['customer_id'],
              referenced: { database: 'tablesail', schema: 'public', name: 'customers' },
              referencedColumns: ['id'],
              onUpdate: 'NO ACTION',
              onDelete: 'CASCADE',
            },
          ]
        : [],
    editability: isView
      ? { kind: 'readOnly', reason: "Views can't be edited." }
      : table.name === 'no_pk'
        ? { kind: 'readOnly', reason: "This table has no primary key, so rows can't be identified safely." }
        : { kind: 'editable' },
    ddl: `CREATE TABLE public.${table.name} (\n${columns.map((c) => `    ${c.name} ${c.dataType}${c.nullable ? '' : ' NOT NULL'}`).join(',\n')}\n);`,
  }
}

function session(id: string): SessionInfo {
  const config = connections.find((c) => c.id === id) ?? connections[0]
  const version = {
    postgres: 'PostgreSQL 17.6',
    mysql: '8.4.6-MySQL',
    mariadb: '11.8.3-MariaDB',
    mongodb: 'MongoDB 8.2.1',
    sqlite: 'SQLite 3.50.4',
  }[config.engine]
  return { sessionId: config.id, config, serverVersion: version, databases: ['tablesail', 'tablesail_other', 'postgres'] }
}

function snapshot(database: string): SchemaSnapshot {
  const columns: Record<string, string[]> = {}
  for (const t of TABLES) columns[t.name] = columnsFor(t).map((c) => c.name)
  return { database, schemas: { public: TABLES, billing: BILLING, empty_schema: [] }, columns, fetchedAt: Date.now() }
}

function split(sql: string): StatementSpan[] {
  const spans: StatementSpan[] = []
  let start = 0
  for (const part of sql.split(';')) {
    const trimmed = part.trim()
    if (trimmed) {
      const s = sql.indexOf(trimmed, start)
      const word = trimmed.split(/\s+/)[0].toLowerCase()
      spans.push({
        start: s,
        end: s + trimmed.length,
        sql: trimmed,
        kind: ['select', 'with', 'show', 'explain'].includes(word) ? 'read' : ['create', 'alter', 'drop', 'truncate'].includes(word) ? 'ddl' : 'write',
        destructive: ['drop', 'truncate'].includes(word),
      })
    }
    start += part.length + 1
  }
  return spans
}

const queries = new Map<string, { statements: StatementSpan[]; started: number; cancelled?: boolean }>()

// Which table's columns/rows a statement "returns": the first table after FROM, big_table otherwise.
function resultFor(sql: string): { columns: ColumnInfo[]; total: number } {
  const name = /\bfrom\s+(?:"?\w+"?\.)?"?(\w+)"?/i.exec(sql)?.[1]
  const known = name && [...TABLES, ...BILLING].some((t) => t.name === name) ? name : 'big_table'
  const table = { database: 'tablesail', schema: 'public', name: known }
  return { columns: columnsFor(table), total: Math.min(rowCountFor(table), 5000) }
}

// Statements that fail (nonexistent table) or never finish until cancelled (pg_sleep).
const failsSql = (sql: string) => /nonexistent/i.test(sql)
const hangsSql = (sql: string) => /pg_sleep/i.test(sql)

function failure(kind: CoreError['kind'], message: string, code: string | null = null): CoreError {
  return { kind, message, code, statement: null }
}
let nextId = 1

// Console pane: statements a real server would have been sent.
const sqlLog: LogEntry[] = []
let logSeq = 0
function logSql(sql: string) {
  sqlLog.push({ seq: ++logSeq, atMs: Date.now(), sql })
  if (sqlLog.length > 2000) sqlLog.shift()
}

// AI assistant: a key is "saved" until the settings dialog removes it.
let assistantKey = true
const assistantCancels = new Map<string, () => void>()
const CANNED_REPLY = `Here is a query for that. It reads the most recent rows from \`customers\`, newest first.

\`\`\`sql
SELECT id, email, name
FROM public.customers
ORDER BY id DESC
LIMIT 10;
\`\`\`

Change \`LIMIT\` to see more rows, or add a \`WHERE\` clause to narrow them down.`

async function handle(cmd: string, args: Record<string, any>): Promise<unknown> {
  await delay(cmd.startsWith('browse') || cmd.startsWith('query') ? 40 : 15)
  switch (cmd) {
    case 'app_info':
      return { version: '0.1.0', engines: ['sqlite', 'postgres', 'mysql', 'mariadb', 'mongodb'] }
    case 'connections_list':
      return connections
    case 'connection_save': {
      const config = { ...args.config, id: args.config.id || `c-${nextId++}` }
      connections = [...connections.filter((c) => c.id !== config.id), config]
      return config
    }
    case 'connection_delete':
      connections = connections.filter((c) => c.id !== args.id)
      return null
    case 'connection_test':
      if (args.config?.host === 'down.internal') throw failure('connection', 'connection refused (os error 111)')
      return { serverVersion: 'PostgreSQL 17.6', elapsedMs: 4 }
    case 'connection_open': {
      // Hosts that misbehave on purpose, for the connection-error and password-prompt flows.
      const cfg = connections.find((c) => c.id === args.id)
      if (cfg?.host === 'down.internal') throw failure('connection', 'connection refused (os error 111)')
      if (cfg?.host === 'locked.internal' && args.secrets?.dbPassword !== 'secret') {
        throw failure('database', 'password authentication failed for user', '28P01')
      }
      logSql('SELECT current_setting(\'server_version\')')
      logSql('SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY datname')
      return session(args.id)
    }
    case 'connection_close':
      return null
    case 'schema_databases':
      return session(args.sessionId).databases
    case 'schema_schemas':
      return ['public', 'billing', 'empty_schema']
    case 'schema_tables':
      return args.schema === 'billing' ? BILLING : TABLES
    case 'schema_snapshot':
      logSql(`SELECT schema_name FROM information_schema.schemata WHERE catalog_name = '${args.database}' ORDER BY schema_name`)
      logSql(
        `SELECT c.relname, c.relkind, c.reltuples::bigint FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relkind IN ('r','v','m','f') ORDER BY c.relname`,
      )
      return snapshot(args.database)
    case 'table_structure':
      logSql(
        `SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = 'public' AND table_name = '${args.table.name}' ORDER BY ordinal_position`,
      )
      return structure(args.table)
    case 'browse_page': {
      const req = args.request as BrowseRequest
      const every = columnsFor(req.table)
      // Like the real drivers: an empty `columns` means all of them, otherwise only those, in table order.
      const wanted = req.columns?.length ? every.filter((c) => req.columns.includes(c.name)) : every
      const total = rowCountFor(req.table)
      const rows: Row[] = []
      const matched = filteredRows(req)
      if (matched) {
        const keep = wanted.map((c) => every.indexOf(c))
        rows.push(...matched.slice(args.offset, args.offset + args.limit).map((row) => keep.map((i) => row[i])))
      } else {
        for (let i = args.offset; i < Math.min(total, args.offset + args.limit); i++) rows.push(wanted.map((c) => cell(c, i)))
      }
      const list = req.columns?.length ? wanted.map((c) => `"${c.name}"`).join(', ') : '*'
      const query = `SELECT ${list} FROM "public"."${req.table.name}" ORDER BY "id" LIMIT ${args.limit} OFFSET ${args.offset}`
      logSql(query)
      return { columns: meta(wanted), rows, offset: args.offset, query, elapsedMs: 3 }
    }
    case 'browse_count': {
      const n = filteredRows(args.request as BrowseRequest)?.length ?? rowCountFor((args.request as BrowseRequest).table)
      logSql(`SELECT count(*) FROM "public"."${(args.request as BrowseRequest).table.name}"`)
      return n > 100_000 ? { exact: null, estimate: n } : { exact: n, estimate: n }
    }
    case 'cell_fetch':
      return 'full value'
    case 'sql_split':
      return split(args.sql)
    case 'query_run': {
      const id = `q${nextId++}`
      const statements = split(args.sql)
      for (const st of statements) logSql(st.sql)
      queries.set(id, { statements, started: Date.now() })
      return { queryId: id, statements }
    }
    case 'query_status': {
      const q = queries.get(args.queryId)!
      const elapsed = Date.now() - q.started
      const hung = q.statements.some((s) => hangsSql(s.sql)) && !q.cancelled
      const done = elapsed > 150 && !hung
      const failedAt = q.statements.findIndex((s) => failsSql(s.sql))
      const status: QueryStatus = {
        queryId: args.queryId,
        finished: done || q.cancelled === true,
        statements: q.statements.map((s, index): StatementStatus => {
          const base = { index, sql: s.sql, elapsedMs: done ? 12 : elapsed }
          if (q.cancelled) return { ...base, state: 'cancelled' as const, error: null, affected: null, sets: [] }
          if (failedAt >= 0 && index > failedAt) return { ...base, state: 'pending' as const, error: null, affected: null, sets: [] }
          if (index === failedAt) {
            const error = { ...failure('database', 'relation "nonexistent_table" does not exist', '42P01'), statement: s.sql }
            return done ? { ...base, state: 'error' as const, error, affected: null, sets: [] } : { ...base, state: 'running' as const, error: null, affected: null, sets: [] }
          }
          const r = resultFor(s.sql)
          return {
            ...base,
            state: done ? ('done' as const) : ('running' as const),
            error: null,
            affected: s.kind === 'read' ? null : 3,
            sets:
              s.kind === 'read' && !hangsSql(s.sql)
                ? [{ setIndex: 0, columns: meta(r.columns), loaded: done ? r.total : Math.min(r.total, 1000), complete: done, truncated: false }]
                : [],
          }
        }),
      }
      return status
    }
    case 'query_cancel': {
      const q = queries.get(args.queryId)
      if (q) q.cancelled = true
      return null
    }
    case 'query_fetch': {
      const q = queries.get(args.queryId)
      const r = resultFor(q?.statements[args.statementIndex]?.sql ?? '')
      const rows: Row[] = []
      for (let i = args.offset; i < Math.min(r.total, args.offset + args.limit); i++) rows.push(r.columns.map((c) => cell(c, i)))
      return { rows, offset: args.offset, loaded: r.total, complete: true }
    }
    case 'changes_preview':
      return ['UPDATE "public"."big_table" SET "name" = \'edited\' WHERE "id" = 1 AND "name" = \'row 1\';']
    case 'changes_apply':
      logSql('BEGIN')
      logSql('UPDATE "public"."big_table" SET "name" = \'edited\' WHERE "id" = 1 AND "name" = \'row 1\';')
      logSql('COMMIT')
      return { statements: 1, rowsAffected: 1 }
    case 'export_start':
    case 'import_start':
      return `job${nextId++}`
    case 'job_status':
      return { rows: 1000, bytes: 52000, done: true, cancelled: false, error: null, warning: null }
    case 'csv_preview':
      return { headers: ['id', 'name'], rows: [['1', 'a'], ['2', 'b']], delimiter: ',' }
    case 'query_log_since':
      return sqlLog.filter((e) => e.seq > args.seq)
    case 'query_log_clear':
      sqlLog.length = 0
      return null
    case 'assistant_key_status':
      return assistantKey
    case 'assistant_key_set':
      assistantKey = String(args.key).trim().length > 0
      return null
    case 'assistant_cancel':
      assistantCancels.get(args.requestId)?.()
      return null
    case 'assistant_ask': {
      const req = args.req as { requestId?: string | null }
      if (!assistantKey) {
        throw { kind: 'invalidInput', message: 'Add your Anthropic API key in Settings to use the assistant.', code: null, statement: null }
      }
      return new Promise((resolve, reject) => {
        const id = req.requestId ?? ''
        const timer = setTimeout(() => {
          assistantCancels.delete(id)
          resolve({ text: CANNED_REPLY })
        }, 600)
        assistantCancels.set(id, () => {
          clearTimeout(timer)
          assistantCancels.delete(id)
          reject({ kind: 'cancelled', message: 'cancelled', code: null, statement: null })
        })
      })
    }
    // Native file dialogs: tests set window.__TABLESAIL_MOCK_DIALOG__ = { open, save } (null = cancelled).
    case 'plugin:dialog|open':
      return (window as any).__TABLESAIL_MOCK_DIALOG__?.open ?? null
    case 'plugin:dialog|save':
      return (window as any).__TABLESAIL_MOCK_DIALOG__?.save ?? null
    default:
      if (cmd.startsWith('plugin:')) return null
      return null
  }
}

let callbackId = 1
;(window as any).__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args: Record<string, unknown> = {}) => handle(cmd, args as Record<string, any>),
  transformCallback: () => callbackId++,
  unregisterCallback: () => {},
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
}
// Lets the UI draw stand-in macOS traffic lights, so browser previews match the real window.
;(window as any).__TABLESAIL_MOCK__ = true
console.info('[mock] Tauri runtime mocked with sample data')
