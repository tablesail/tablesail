// Connection URLs: parse a pasted URL into connection-form fields, and build one back (without the password).
// Pure functions only, so the whole thing is unit-tested without a browser.
//
// Accepted: postgres:// postgresql:// mysql:// mariadb:// sqlite:// file: mongodb:// mongodb+srv://, a jdbc: prefix,
// SQLAlchemy driver suffixes (postgresql+psycopg2://), a bare *.db / *.sqlite / *.sqlite3 path, and URLs wrapped in
// quotes or a shell command / env assignment (psql "postgres://…", DATABASE_URL=postgres://…).

import type { ConnectionConfig, Engine, SslConfig, SslMode } from '../types'
import { DEFAULT_PORT, ENGINE_LABEL, defaultUser, describeConnection } from './env'
import { basename } from './format'

/** What a URL says about the connection. Only the pieces the URL actually names are present. */
export interface ParsedConnectionUrl {
  engine: Engine
  host?: string
  /** Only when written in the URL; the engine's default port is applied when a host is given without one. */
  port?: number
  user?: string
  password?: string
  database?: string
  /** SQLite database file. */
  filePath?: string
  /** SSL mode and certificate paths named by query parameters. */
  ssl?: Partial<Pick<SslConfig, 'mode' | 'caCert' | 'clientCert' | 'clientKey'>>
  /** SQLite opened read-only (`mode=ro`). */
  readOnly?: boolean
  /** MongoDB keeps the URL itself (minus the password, which goes to the keychain). */
  url?: string
  /** MongoDB: what the URL says about TLS (`mongodb+srv`, `tls=` / `ssl=`); undefined when it says nothing. */
  tls?: boolean
  /** "db @ host", or the file name for SQLite. */
  name?: string
}

export type ParseResult = { ok: true; value: ParsedConnectionUrl } | { ok: false; error: string }

const SCHEMES_HELP = 'postgresql://, mysql://, mariadb://, sqlite://, file: or mongodb://'

const EXAMPLE = 'postgresql://user:password@host:5432/database'

const SQLITE_EXT = /\.(?:db|sqlite3?|db3|s3db)$/i

// Where a URL starts: scheme:// for anything, or the schemes that are written without the slashes.
const URL_START = /^(?:jdbc:)?(?:(?:sqlite3?|file):|[a-z][a-z0-9+.-]+:\/\/)/i
const URL_INSIDE = /(?:jdbc:)?[a-z][a-z0-9+.-]+:\/\/[^\s'"]+/i

const ok = (value: ParsedConnectionUrl): ParseResult => ({ ok: true, value })
const fail = (error: string): ParseResult => ({ ok: false, error })

function decode(s: string): string {
  try {
    return decodeURIComponent(s)
  } catch {
    return s
  }
}

function unquote(s: string): string {
  const m = /^(["'`])(.*)\1$/s.exec(s)
  return m ? m[2].trim() : s
}

/** The URL (or SQLite path) inside whatever was pasted, or null when there is none. */
export function findUrl(input: string): string | null {
  const text = unquote(input.trim())
  if (!text) return null
  // A URL starts the text: whatever follows on later lines (a pasted .env file, a command's output) is not part of it.
  if (URL_START.test(text)) return text.split(/\r?\n/, 1)[0].trim()
  const inside = URL_INSIDE.exec(text)
  if (inside) return inside[0]
  if (SQLITE_EXT.test(text) && !/[\r\n]/.test(text)) return text
  return null
}

/** True for pasted text that is a URL of any scheme (used to catch a URL pasted into the Host field). */
export function looksLikeUrl(input: string): boolean {
  const url = findUrl(input)
  return !!url && URL_START.test(url)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

export function parseConnectionUrl(input: string): ParseResult {
  const text = findUrl(input)
  if (!text) {
    return fail(input.trim() ? `That does not look like a connection URL. Example: ${EXAMPLE}` : 'Paste a connection URL first.')
  }
  // A bare path to a database file.
  if (!URL_START.test(text)) return sqlite(text, false)

  const m = /^([a-z][a-z0-9+.-]*):(.*)$/is.exec(text.replace(/^jdbc:/i, ''))
  if (!m) return fail(`That does not look like a connection URL. Example: ${EXAMPLE}`)
  const scheme = m[1]
  const rest = m[2]
  const lower = scheme.toLowerCase()
  // postgresql+psycopg2 -> postgresql; mongodb+srv is its own scheme.
  const base = lower === 'mongodb+srv' ? lower : lower.split('+')[0]
  switch (base) {
    case 'postgres':
    case 'postgresql':
      return network('postgres', scheme, rest)
    case 'mysql':
      return network('mysql', scheme, rest)
    case 'mariadb':
      return network('mariadb', scheme, rest)
    case 'mongodb':
    case 'mongodb+srv':
      return network('mongodb', scheme, rest)
    case 'sqlite':
    case 'sqlite3':
      return sqlite(rest, true)
    case 'file':
      return sqlite(rest, true, true)
    default:
      return fail(`Unsupported URL scheme "${scheme}". Use ${SCHEMES_HELP}.`)
  }
}

/**
 * A SQLite database file from `sqlite:` / `file:` URL text (everything after the scheme) or a bare path.
 * `sqlite:///abs/x.db`, `sqlite:////abs/x.db`, `sqlite:/abs/x.db` and `file:///abs/x.db` all mean /abs/x.db;
 * `sqlite://data/x.db` and `file:x.db` are relative paths.
 */
function sqlite(rest: string, asUrl: boolean, isFile = false): ParseResult {
  let path = rest
  let query = ''
  if (asUrl) {
    const q = path.indexOf('?')
    if (q >= 0) {
      query = path.slice(q + 1)
      path = path.slice(0, q)
    }
    if (path.startsWith('//')) {
      path = path.slice(2)
      if (isFile && /^localhost\//i.test(path)) path = path.slice('localhost'.length)
      path = path.replace(/^\/{2,}/, '/')
    }
    // sqlite:///C:/Users/me/x.db
    path = decode(path.replace(/^\/([A-Za-z]:[\\/])/, '$1'))
  }
  path = path.trim()
  if (!path) return fail('The URL has no database file path.')
  if (/^\/?:memory:$/.test(path)) return fail('In-memory SQLite databases cannot be opened. Give the path of a database file.')
  const value: ParsedConnectionUrl = { engine: 'sqlite', filePath: path, name: basename(path) }
  if (parseQuery(query).get('mode')?.toLowerCase() === 'ro') value.readOnly = true
  return ok(value)
}

function parseQuery(query: string): Map<string, string> {
  const out = new Map<string, string>()
  for (const pair of query.split('&')) {
    if (!pair) continue
    const eq = pair.indexOf('=')
    const key = decode(eq < 0 ? pair : pair.slice(0, eq)).toLowerCase()
    out.set(key, eq < 0 ? '' : decode(pair.slice(eq + 1)))
  }
  return out
}

/** "host", "host:port", "[::1]:port" or a bare IPv6 address. */
function splitHostPort(hp: string): { host: string; port: string } | string {
  if (hp.startsWith('[')) {
    const end = hp.indexOf(']')
    if (end < 0) return `Invalid host "${hp}": missing "]".`
    const after = hp.slice(end + 1)
    if (after && !after.startsWith(':')) return `Invalid host "${hp}".`
    return { host: hp.slice(1, end), port: after.slice(1) }
  }
  const colons = hp.split(':').length - 1
  if (colons > 1) return { host: hp, port: '' } // unbracketed IPv6, no port
  const at = hp.indexOf(':')
  return at < 0 ? { host: hp, port: '' } : { host: hp.slice(0, at), port: hp.slice(at + 1) }
}

const SSL_WORDS: Record<string, SslMode> = {
  disable: 'disable',
  disabled: 'disable',
  off: 'disable',
  no: 'disable',
  false: 'disable',
  '0': 'disable',
  allow: 'prefer',
  prefer: 'prefer',
  preferred: 'prefer',
  require: 'require',
  required: 'require',
  on: 'require',
  yes: 'require',
  true: 'require',
  '1': 'require',
  trust: 'require',
  'no-verify': 'require',
  'skip-verify': 'require',
  'verify-ca': 'verifyCa',
  'verify-full': 'verifyFull',
  'verify-identity': 'verifyFull',
}

const sslWord = (v: string): SslMode | undefined => SSL_WORDS[v.trim().toLowerCase().replaceAll('_', '-')]

/** sslmode / ssl-mode / ssl / useSSL and certificate paths, for Postgres, MySQL and MariaDB. */
function sslFrom(params: Map<string, string>): ParsedConnectionUrl['ssl'] | string | undefined {
  const ssl: NonNullable<ParsedConnectionUrl['ssl']> = {}
  for (const key of ['sslmode', 'ssl-mode']) {
    const raw = params.get(key)
    if (raw === undefined) continue
    const mode = sslWord(raw)
    // A typo in the SSL mode must not quietly connect with a weaker one.
    if (!mode) return `Unknown SSL mode "${raw}". Use disable, prefer, require, verify-ca or verify-full.`
    ssl.mode = mode
  }
  if (!ssl.mode) {
    // JDBC style flags; values that are not plain booleans are left alone.
    for (const key of ['ssl', 'usessl']) {
      const raw = params.get(key)
      const mode = raw === undefined ? undefined : sslWord(raw)
      if (mode) ssl.mode = mode
    }
    if (!ssl.mode && sslWord(params.get('requiressl') ?? '') === 'require') ssl.mode = 'require'
  }
  const path = (...keys: string[]) => {
    for (const k of keys) {
      const v = params.get(k)?.trim()
      if (v && v !== 'system') return v
    }
  }
  const ca = path('sslrootcert', 'ssl-ca', 'sslca')
  const cert = path('sslcert', 'ssl-cert')
  const key = path('sslkey', 'ssl-key')
  if (ca) ssl.caCert = ca
  if (cert) ssl.clientCert = cert
  if (key) ssl.clientKey = key
  return Object.keys(ssl).length ? ssl : undefined
}

function network(engine: Exclude<Engine, 'sqlite'>, scheme: string, rest: string): ParseResult {
  const body = rest.slice(2) // rest is "//…": URL_START only lets scheme:// through to here
  const end = body.search(/[/?#]/)
  const authority = end < 0 ? body : body.slice(0, end)
  let tail = end < 0 ? '' : body.slice(end)
  const hash = tail.indexOf('#')
  if (hash >= 0) tail = tail.slice(0, hash)
  const qAt = tail.indexOf('?')
  const path = qAt < 0 ? tail : tail.slice(0, qAt)
  const params = parseQuery(qAt < 0 ? '' : tail.slice(qAt + 1))

  // libpq / JDBC also take user, password, host, port and dbname as query parameters.
  const extra = (key: string, ...alt: string[]) => (engine === 'mongodb' ? undefined : [key, ...alt].map((k) => params.get(k)).find(Boolean))

  // The last "@" separates the credentials, so an unescaped "@" in a password still works.
  const at = authority.lastIndexOf('@')
  const userinfo = at < 0 ? '' : authority.slice(0, at)
  const hosts = authority.slice(at + 1)
  const colon = userinfo.indexOf(':')
  const rawUser = colon < 0 ? userinfo : userinfo.slice(0, colon)
  const user = decode(rawUser) || extra('user', 'username') || undefined
  const password = (colon < 0 ? '' : decode(userinfo.slice(colon + 1))) || extra('password') || undefined

  // Several hosts (failover lists) are not supported by the form: the first one is used.
  const hp = splitHostPort(hosts.split(',')[0].trim())
  if (typeof hp === 'string') return fail(hp)
  const hostInUrl = decode(hp.host).trim()
  const host = hostInUrl || extra('host') || undefined
  if (engine === 'mongodb' && !host) return fail('The URL has no host.')
  const portText = (hp.port || extra('port') || '').trim()
  let port: number | undefined
  if (portText) {
    if (!/^\d+$/.test(portText) || +portText < 1 || +portText > 65535) {
      return fail(`Invalid port "${portText}". Special characters in a password must be percent-encoded (@ is %40, / is %2F).`)
    }
    port = +portText
  }
  const database = decode(path.replace(/^\//, '').split('/')[0]) || extra('dbname', 'database') || undefined

  const value: ParsedConnectionUrl = { engine }
  if (host) value.host = host
  if (port !== undefined) value.port = port
  if (user) value.user = user
  if (password) value.password = password
  if (database) value.database = database
  if (host) value.name = database ? `${database} @ ${host}` : host

  if (engine === 'mongodb') {
    // Kept as the URL, minus the password (it travels to the keychain instead).
    value.url = splitUrlPassword(`${scheme}:${rest}`).url
    const tls = sslWord(params.get('tls') ?? params.get('ssl') ?? '')
    if (tls) value.tls = tls !== 'disable'
    else if (scheme.toLowerCase() === 'mongodb+srv') value.tls = true
    return ok(value)
  }
  const ssl = sslFrom(params)
  if (typeof ssl === 'string') return fail(ssl)
  if (ssl) value.ssl = ssl
  return ok(value)
}

/**
 * A URL with the password taken out of its credentials, and the (percent-decoded) password itself.
 * The credentials end at the last "@" of the authority, so an unescaped "@" in a password still works.
 */
export function splitUrlPassword(url: string): { url: string; password: string | null } {
  const m = /^([a-z][a-z0-9+.-]*:\/\/)([^/?#]*)([\s\S]*)$/i.exec(url)
  if (!m) return { url, password: null }
  const authority = m[2]
  const at = authority.lastIndexOf('@')
  const userinfo = authority.slice(0, at)
  const colon = userinfo.indexOf(':')
  const password = at < 0 || colon < 0 ? '' : decode(userinfo.slice(colon + 1))
  if (!password) return { url, password: null }
  const user = userinfo.slice(0, colon)
  return { url: `${m[1]}${user ? `${user}@` : ''}${authority.slice(at + 1)}${m[3]}`, password }
}

// ---------------------------------------------------------------------------
// Applying a parsed URL to the form
// ---------------------------------------------------------------------------

/**
 * Returns `cfg` with the URL's pieces filled in (pieces the URL does not name stay as they were) and
 * the password to put in the form's password field. The engine switches the way the engine tiles do.
 * `setName` fills the name from the URL; the form passes it while the name is empty or still auto-filled.
 */
export function applyConnectionUrl(
  cfg: ConnectionConfig,
  p: ParsedConnectionUrl,
  opts: { setName?: boolean } = {},
): { config: ConnectionConfig; password: string | null } {
  const prev = cfg.engine
  // MariaDB speaks mysql:// too: a MariaDB form stays a MariaDB form.
  const engine: Engine = p.engine === 'mysql' && prev === 'mariadb' ? 'mariadb' : p.engine
  const out: ConnectionConfig = { ...cfg, ssl: { ...cfg.ssl } }

  if (engine !== prev) {
    if (out.port === DEFAULT_PORT[prev] || !out.port) out.port = DEFAULT_PORT[engine]
    if (out.user === defaultUser(prev)) out.user = defaultUser(engine)
    if (engine !== 'sqlite' && !out.host) out.host = '127.0.0.1'
    out.engine = engine
  }
  out.url = engine === 'mongodb' ? (p.url ?? null) : null

  if (engine === 'sqlite') {
    if (p.filePath !== undefined) out.filePath = p.filePath
    if (p.readOnly) out.readOnly = true
  } else {
    if (p.host !== undefined) {
      out.host = p.host
      out.port = p.port ?? DEFAULT_PORT[engine]
    } else if (p.port !== undefined) {
      out.port = p.port
    }
    if (p.user !== undefined) out.user = p.user
    if (p.database !== undefined) out.database = p.database
    if (p.ssl) out.ssl = { ...out.ssl, ...p.ssl }
    // The form's SSL mode overrides the URL's tls setting, so it must not contradict what the URL says.
    if (p.tls === false) out.ssl.mode = 'disable'
    else if (p.tls && out.ssl.mode === 'disable') out.ssl.mode = 'prefer'
  }
  if (opts.setName && p.name) out.name = p.name
  return { config: out, password: engine === 'sqlite' ? null : (p.password ?? null) }
}

/** One line saying what an import filled in, e.g. "PostgreSQL · app@db.example.com:5432/orders". */
export function describeImport(config: ConnectionConfig, password: string | null): string {
  const where = describeConnection(config)
  return `${ENGINE_LABEL[config.engine]}${where ? ` · ${where}` : ''}${password ? ' · password set' : ''}`
}

// ---------------------------------------------------------------------------
// Building a URL (never with the password)
// ---------------------------------------------------------------------------

const SCHEME_OF: Record<Exclude<Engine, 'sqlite' | 'mongodb'>, string> = {
  postgres: 'postgresql',
  mysql: 'mysql',
  mariadb: 'mariadb',
}

const PG_SSL: Record<SslMode, string> = { disable: 'disable', prefer: 'prefer', require: 'require', verifyCa: 'verify-ca', verifyFull: 'verify-full' }
const MYSQL_SSL: Record<SslMode, string> = { disable: 'DISABLED', prefer: 'PREFERRED', require: 'REQUIRED', verifyCa: 'VERIFY_CA', verifyFull: 'VERIFY_IDENTITY' }

const encodePath = (path: string) => encodeURI(path).replace(/[?#]/g, (c) => encodeURIComponent(c))

const hostText = (host: string) => (host.includes(':') && !host.startsWith('[') ? `[${host}]` : host)

/** The connection as a URL without the password, or null when it has no host / file yet. */
export function connectionUrl(cfg: ConnectionConfig): string | null {
  if (cfg.engine === 'sqlite') {
    const path = cfg.filePath?.trim().replaceAll('\\', '/')
    if (!path) return null
    // sqlite:///Users/me/x.db, sqlite:///C:/x.db and, for relative paths, sqlite://data/x.db
    return `sqlite://${/^[A-Za-z]:\//.test(path) ? '/' : ''}${encodePath(path)}`
  }
  if (cfg.engine === 'mongodb' && cfg.url?.trim()) {
    // Saved URLs may still carry a password.
    return splitUrlPassword(cfg.url.trim()).url
  }
  const host = cfg.host.trim()
  if (!host) return null

  let url = cfg.engine === 'mongodb' ? 'mongodb://' : `${SCHEME_OF[cfg.engine]}://`
  if (cfg.user) url += `${encodeURIComponent(cfg.user)}@`
  url += hostText(host)
  if (cfg.port) url += `:${cfg.port}`
  if (cfg.database) url += `/${encodeURIComponent(cfg.database)}`

  const mode = cfg.ssl.mode
  let query = ''
  if (cfg.engine === 'postgres') {
    if (mode !== 'prefer') query = `sslmode=${PG_SSL[mode]}`
  } else if (cfg.engine === 'mongodb') {
    if (mode === 'disable') query = 'tls=false'
    else if (mode !== 'prefer') query = 'tls=true'
  } else if (mode !== 'prefer') {
    query = `ssl-mode=${MYSQL_SSL[mode]}`
  }
  if (query) url += `${cfg.database ? '' : '/'}?${query}`
  return url
}
