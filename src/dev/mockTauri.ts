// Dev-only stand-in for the Tauri runtime, so the UI can be opened in a plain
// browser (npm run dev → http://localhost:5173) with realistic sample data.
// Loaded by main.ts only when import.meta.env.DEV and no real Tauri runtime exists.

import type {
  BrowseRequest,
  ColumnInfo,
  ColumnMeta,
  ConnectionConfig,
  QueryStatus,
  Row,
  SchemaSnapshot,
  SessionInfo,
  StatementSpan,
  TableInfo,
  TableRef,
  TableStructure,
  TypeFamily,
  Value,
} from '../lib/types'

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms))

const baseConfig = {
  user: 'tablesoft',
  database: 'tablesoft',
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
  database: 'tablesoft',
  schema: 'public',
  name: name as string,
  kind: kind as TableInfo['kind'],
  estimatedRows: rows as number | null,
  comment: null,
}))

const BILLING: TableInfo[] = [{ database: 'tablesoft', schema: 'billing', name: 'invoices', kind: 'table', estimatedRows: 2, comment: null }]

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
              referenced: { database: 'tablesoft', schema: 'public', name: 'customers' },
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
  return { sessionId: config.id, config, serverVersion: version, databases: ['tablesoft', 'tablesoft_other', 'postgres'] }
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

const queries = new Map<string, { statements: StatementSpan[]; started: number }>()
let nextId = 1

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
      return { serverVersion: 'PostgreSQL 17.6', elapsedMs: 4 }
    case 'connection_open':
      return session(args.id)
    case 'connection_close':
      return null
    case 'schema_databases':
      return session(args.sessionId).databases
    case 'schema_schemas':
      return ['public', 'billing', 'empty_schema']
    case 'schema_tables':
      return args.schema === 'billing' ? BILLING : TABLES
    case 'schema_snapshot':
      return snapshot(args.database)
    case 'table_structure':
      return structure(args.table)
    case 'browse_page': {
      const req = args.request as BrowseRequest
      const columns = columnsFor(req.table)
      const total = rowCountFor(req.table)
      const rows: Row[] = []
      for (let i = args.offset; i < Math.min(total, args.offset + args.limit); i++) rows.push(columns.map((c) => cell(c, i)))
      return {
        columns: meta(columns),
        rows,
        offset: args.offset,
        query: `SELECT * FROM "public"."${req.table.name}" ORDER BY "id" LIMIT ${args.limit} OFFSET ${args.offset}`,
        elapsedMs: 3,
      }
    }
    case 'browse_count': {
      const n = rowCountFor((args.request as BrowseRequest).table)
      return n > 100_000 ? { exact: null, estimate: n } : { exact: n, estimate: n }
    }
    case 'cell_fetch':
      return 'full value'
    case 'sql_split':
      return split(args.sql)
    case 'query_run': {
      const id = `q${nextId++}`
      const statements = split(args.sql)
      queries.set(id, { statements, started: Date.now() })
      return { queryId: id, statements }
    }
    case 'query_status': {
      const q = queries.get(args.queryId)!
      const done = Date.now() - q.started > 150
      const status: QueryStatus = {
        queryId: args.queryId,
        finished: done,
        statements: q.statements.map((s, index) => ({
          index,
          sql: s.sql,
          state: done ? 'done' : 'running',
          error: null,
          affected: s.kind === 'read' ? null : 3,
          sets: s.kind === 'read' ? [{ setIndex: 0, columns: meta(TABLE_COLUMNS.big_table), loaded: done ? 5000 : 1000, complete: done, truncated: false }] : [],
          elapsedMs: done ? 12 : Date.now() - q.started,
        })),
      }
      return status
    }
    case 'query_fetch': {
      const rows: Row[] = []
      for (let i = args.offset; i < Math.min(5000, args.offset + args.limit); i++) rows.push(TABLE_COLUMNS.big_table.map((c) => cell(c, i)))
      return { rows, offset: args.offset, loaded: 5000, complete: true }
    }
    case 'changes_preview':
      return ['UPDATE "public"."big_table" SET "name" = \'edited\' WHERE "id" = 1 AND "name" = \'row 1\';']
    case 'changes_apply':
      return { statements: 1, rowsAffected: 1 }
    case 'export_start':
    case 'import_start':
      return `job${nextId++}`
    case 'job_status':
      return { rows: 1000, bytes: 52000, done: true, cancelled: false, error: null, warning: null }
    case 'csv_preview':
      return { headers: ['id', 'name'], rows: [['1', 'a'], ['2', 'b']], delimiter: ',' }
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
;(window as any).__TABLESOFT_MOCK__ = true
console.info('[mock] Tauri runtime mocked with sample data')
