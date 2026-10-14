import { describe, expect, it } from 'vitest'
import type { ConnectionConfig } from '../types'
import {
  applyConnectionUrl,
  connectionUrl,
  describeImport,
  findUrl,
  looksLikeUrl,
  parseConnectionUrl,
  splitUrlPassword,
  type ParsedConnectionUrl,
} from './connUrl'
import { describeConnection, newConfig } from './env'

function parsed(url: string): ParsedConnectionUrl {
  const r = parseConnectionUrl(url)
  if (!r.ok) throw new Error(`expected ${url} to parse, got: ${r.error}`)
  return r.value
}

function error(url: string): string {
  const r = parseConnectionUrl(url)
  if (r.ok) throw new Error(`expected ${url} to fail, got: ${JSON.stringify(r.value)}`)
  return r.error
}

describe('PostgreSQL', () => {
  it('reads every part of a full URL', () => {
    expect(parsed('postgres://app:s3cret@db.example.com:5433/orders')).toEqual({
      engine: 'postgres',
      host: 'db.example.com',
      port: 5433,
      user: 'app',
      password: 's3cret',
      database: 'orders',
      name: 'orders @ db.example.com',
    })
  })

  it('accepts postgresql:// as well', () => {
    expect(parsed('postgresql://u@h/d').engine).toBe('postgres')
  })

  it('is case-insensitive about the scheme', () => {
    expect(parsed('POSTGRES://u@h/d').engine).toBe('postgres')
    expect(parsed('PostgreSQL://u@h/d').host).toBe('h')
  })

  it('leaves out what the URL does not say', () => {
    expect(parsed('postgres://db.example.com')).toEqual({ engine: 'postgres', host: 'db.example.com', name: 'db.example.com' })
    expect(parsed('postgres://db.example.com/')).toEqual({ engine: 'postgres', host: 'db.example.com', name: 'db.example.com' })
    expect(parsed('postgres://app@db.example.com')).toMatchObject({ user: 'app' })
    expect(parsed('postgres://app@db.example.com').password).toBeUndefined()
    expect(parsed('postgres://app@db.example.com').port).toBeUndefined()
    expect(parsed('postgres://app@db.example.com').database).toBeUndefined()
  })

  it('does not invent a port: the default is applied when filling the form', () => {
    expect(parsed('postgres://h/d').port).toBeUndefined()
    expect(parsed('postgres://h:/d').port).toBeUndefined()
  })

  it('percent-decodes user, password and database', () => {
    const p = parsed('postgres://me%40corp:p%40ss%3Aw%2Frd%25@h/my%20db')
    expect(p.user).toBe('me@corp')
    expect(p.password).toBe('p@ss:w/rd%')
    expect(p.database).toBe('my db')
  })

  it('keeps a malformed escape as typed instead of throwing', () => {
    expect(parsed('postgres://u:100%@h/d').password).toBe('100%')
    expect(parsed('postgres://u@h/d%zz').database).toBe('d%zz')
  })

  it('splits the credentials at the first colon, so a password may contain colons', () => {
    expect(parsed('postgres://u:a:b:c@h/d')).toMatchObject({ user: 'u', password: 'a:b:c' })
  })

  it('splits the credentials at the last @, so a raw @ in a password still works', () => {
    expect(parsed('postgres://u:p@ss@h/d')).toMatchObject({ user: 'u', password: 'p@ss', host: 'h' })
  })

  it('ignores an empty user or password', () => {
    expect(parsed('postgres://:pw@h/d')).toMatchObject({ password: 'pw' })
    expect(parsed('postgres://:pw@h/d').user).toBeUndefined()
    expect(parsed('postgres://u:@h/d').password).toBeUndefined()
  })

  it('reads bracketed IPv6 hosts, with and without a port', () => {
    expect(parsed('postgres://u:p@[::1]:5433/d')).toMatchObject({ host: '::1', port: 5433, database: 'd' })
    expect(parsed('postgres://[2001:db8::7]/d')).toMatchObject({ host: '2001:db8::7' })
    expect(parsed('postgres://[2001:db8::7]/d').port).toBeUndefined()
  })

  it('reads an unbracketed IPv6 host as a whole host with no port', () => {
    expect(parsed('postgres://::1/d')).toMatchObject({ host: '::1' })
    expect(parsed('postgres://::1/d').port).toBeUndefined()
  })

  it('rejects an unterminated IPv6 bracket and junk after it', () => {
    expect(error('postgres://[::1/d')).toMatch(/missing "\]"/)
    expect(error('postgres://[::1]x/d')).toMatch(/Invalid host/)
  })

  it('reads IPv4 hosts and uses the first of a host list', () => {
    expect(parsed('postgres://u@10.0.0.5:6432/d')).toMatchObject({ host: '10.0.0.5', port: 6432 })
    expect(parsed('postgres://u@a:5432,b:5433,c/d')).toMatchObject({ host: 'a', port: 5432 })
  })

  it('reads a unix socket directory given as an encoded host or as ?host=', () => {
    expect(parsed('postgres://%2Fvar%2Frun%2Fpostgresql/d').host).toBe('/var/run/postgresql')
    expect(parsed('postgresql:///d?host=/var/run/postgresql&port=5433')).toMatchObject({
      host: '/var/run/postgresql',
      port: 5433,
      database: 'd',
    })
  })

  it('keeps a port written without a host', () => {
    expect(parsed('postgres://:5433/d')).toMatchObject({ port: 5433, database: 'd' })
    expect(parsed('postgres://:5433/d').host).toBeUndefined()
  })

  it('takes user, password and dbname from the query (libpq / JDBC), but the URL part wins', () => {
    expect(parsed('postgresql://h/?user=a&password=b&dbname=c')).toMatchObject({ user: 'a', password: 'b', database: 'c' })
    expect(parsed('postgresql://x:y@h/d?user=a&password=b&dbname=c')).toMatchObject({ user: 'x', password: 'y', database: 'd' })
  })

  it('ignores options such as application_name', () => {
    const p = parsed('postgres://u@h/d?application_name=psql&connect_timeout=10&options=-c%20x%3Dy')
    expect(p).toEqual({ engine: 'postgres', host: 'h', user: 'u', database: 'd', name: 'd @ h' })
  })

  it('ignores a fragment', () => {
    expect(parsed('postgres://u@h/d#section').database).toBe('d')
  })

  it('names the connection "db @ host", or the host alone', () => {
    expect(parsed('postgres://u@h.example/d').name).toBe('d @ h.example')
    expect(parsed('postgres://u@h.example').name).toBe('h.example')
  })

  it('accepts SQLAlchemy driver suffixes', () => {
    expect(parsed('postgresql+psycopg2://u:p@h/d')).toMatchObject({ engine: 'postgres', host: 'h' })
    expect(parsed('postgresql+asyncpg://u@h/d').engine).toBe('postgres')
  })

  it('rejects ports that are not numbers or out of range', () => {
    expect(error('postgres://h:abc/d')).toMatch(/Invalid port "abc"/)
    expect(error('postgres://h:0/d')).toMatch(/Invalid port "0"/)
    expect(error('postgres://h:65536/d')).toMatch(/Invalid port "65536"/)
    expect(error('postgres://h:-5/d')).toMatch(/Invalid port/)
    expect(parsed('postgres://h:65535/d').port).toBe(65535)
    expect(parsed('postgres://h:1/d').port).toBe(1)
  })

  it('points at percent-encoding when an unescaped / in the password breaks the port', () => {
    expect(error('postgres://u:pa/ss@h/d')).toMatch(/percent-encoded/)
  })

  it('needs :// after the scheme', () => {
    expect(error('postgres:h/d')).toMatch(/does not look like a connection URL/)
    expect(error('jdbc:postgresql:mydb')).toMatch(/does not look like a connection URL/)
  })
})

describe('PostgreSQL SSL', () => {
  const mode = (q: string) => parsed(`postgres://h/d?${q}`).ssl

  it('maps every sslmode to the form modes', () => {
    expect(mode('sslmode=disable')).toEqual({ mode: 'disable' })
    expect(mode('sslmode=allow')).toEqual({ mode: 'prefer' })
    expect(mode('sslmode=prefer')).toEqual({ mode: 'prefer' })
    expect(mode('sslmode=require')).toEqual({ mode: 'require' })
    expect(mode('sslmode=verify-ca')).toEqual({ mode: 'verifyCa' })
    expect(mode('sslmode=verify-full')).toEqual({ mode: 'verifyFull' })
    expect(mode('sslmode=no-verify')).toEqual({ mode: 'require' })
  })

  it('is case- and underscore-insensitive', () => {
    expect(mode('SSLMODE=Verify_Full')).toEqual({ mode: 'verifyFull' })
  })

  it('reads the JDBC ssl flag', () => {
    expect(mode('ssl=true')).toEqual({ mode: 'require' })
    expect(mode('ssl=false')).toEqual({ mode: 'disable' })
    expect(mode('ssl=1')).toEqual({ mode: 'require' })
  })

  it('prefers sslmode over the ssl flag', () => {
    expect(mode('ssl=true&sslmode=verify-ca')).toEqual({ mode: 'verifyCa' })
    expect(mode('sslmode=disable&ssl=true')).toEqual({ mode: 'disable' })
  })

  it('reads the certificate paths', () => {
    expect(mode('sslmode=verify-full&sslrootcert=/etc/ca.pem&sslcert=/etc/c.crt&sslkey=/etc/c.key')).toEqual({
      mode: 'verifyFull',
      caCert: '/etc/ca.pem',
      clientCert: '/etc/c.crt',
      clientKey: '/etc/c.key',
    })
    expect(mode('sslrootcert=%2Fhome%2Fme%2Fca%20x.pem')).toEqual({ caCert: '/home/me/ca x.pem' })
  })

  it('ignores sslrootcert=system', () => {
    expect(mode('sslmode=verify-full&sslrootcert=system')).toEqual({ mode: 'verifyFull' })
  })

  it('refuses an unknown sslmode rather than quietly connecting with a weaker one', () => {
    expect(error('postgres://h/d?sslmode=verify-fulll')).toMatch(/Unknown SSL mode "verify-fulll"/)
  })

  it('leaves SSL alone when the URL says nothing', () => {
    expect(parsed('postgres://h/d').ssl).toBeUndefined()
    expect(parsed('postgres://h/d?application_name=x').ssl).toBeUndefined()
  })

  it('ignores a flag it cannot read (ssl={...} from other tools)', () => {
    expect(parsed('postgres://h/d?ssl=%7B%7D').ssl).toBeUndefined()
  })
})

describe('MySQL and MariaDB', () => {
  it('reads mysql:// with the default port left to the form', () => {
    expect(parsed('mysql://root:pw@db.local/shop')).toEqual({
      engine: 'mysql',
      host: 'db.local',
      user: 'root',
      password: 'pw',
      database: 'shop',
      name: 'shop @ db.local',
    })
  })

  it('reads mariadb://', () => {
    expect(parsed('mariadb://app@m.local:3307/d')).toMatchObject({ engine: 'mariadb', host: 'm.local', port: 3307, user: 'app' })
  })

  it('accepts SQLAlchemy driver suffixes', () => {
    expect(parsed('mysql+pymysql://u:p@h/d').engine).toBe('mysql')
    expect(parsed('mariadb+mariadbconnector://u:p@h/d').engine).toBe('mariadb')
  })

  it('maps ssl-mode values', () => {
    const ssl = (v: string) => parsed(`mysql://h/d?ssl-mode=${v}`).ssl
    expect(ssl('DISABLED')).toEqual({ mode: 'disable' })
    expect(ssl('PREFERRED')).toEqual({ mode: 'prefer' })
    expect(ssl('REQUIRED')).toEqual({ mode: 'require' })
    expect(ssl('VERIFY_CA')).toEqual({ mode: 'verifyCa' })
    expect(ssl('VERIFY_IDENTITY')).toEqual({ mode: 'verifyFull' })
    expect(ssl('required')).toEqual({ mode: 'require' })
  })

  it('maps the other spellings: sslMode, sslmode, ssl, useSSL, requireSSL', () => {
    expect(parsed('mysql://h/d?sslMode=VERIFY_IDENTITY').ssl).toEqual({ mode: 'verifyFull' })
    expect(parsed('mysql://h/d?sslmode=required').ssl).toEqual({ mode: 'require' })
    expect(parsed('mysql://h/d?ssl=true').ssl).toEqual({ mode: 'require' })
    expect(parsed('mysql://h/d?useSSL=false').ssl).toEqual({ mode: 'disable' })
    expect(parsed('mysql://h/d?useSSL=true&requireSSL=true').ssl).toEqual({ mode: 'require' })
    expect(parsed('mysql://h/d?requireSSL=true').ssl).toEqual({ mode: 'require' })
    expect(parsed('mariadb://h/d?sslMode=trust').ssl).toEqual({ mode: 'require' })
    expect(parsed('mariadb://h/d?sslMode=verify-full').ssl).toEqual({ mode: 'verifyFull' })
  })

  it('does not read requireSSL=false as "disable"', () => {
    expect(parsed('mysql://h/d?requireSSL=false').ssl).toBeUndefined()
  })

  it('reads ssl-ca, ssl-cert and ssl-key', () => {
    expect(parsed('mysql://h/d?ssl-mode=VERIFY_CA&ssl-ca=/ca.pem&ssl-cert=/c.pem&ssl-key=/k.pem').ssl).toEqual({
      mode: 'verifyCa',
      caCert: '/ca.pem',
      clientCert: '/c.pem',
      clientKey: '/k.pem',
    })
  })

  it('refuses an unknown ssl-mode', () => {
    expect(error('mysql://h/d?ssl-mode=maybe')).toMatch(/Unknown SSL mode "maybe"/)
  })

  it('ignores charset, timezone and other options', () => {
    expect(parsed('mysql://u@h/d?charset=utf8mb4&serverTimezone=UTC').ssl).toBeUndefined()
  })
})

describe('JDBC', () => {
  it('strips the jdbc: prefix for every network engine', () => {
    expect(parsed('jdbc:postgresql://h:5433/d')).toMatchObject({ engine: 'postgres', host: 'h', port: 5433, database: 'd' })
    expect(parsed('jdbc:mysql://h:3307/d')).toMatchObject({ engine: 'mysql', host: 'h', port: 3307 })
    expect(parsed('jdbc:mariadb://h/d')).toMatchObject({ engine: 'mariadb' })
    expect(parsed('jdbc:mongodb://h/d')).toMatchObject({ engine: 'mongodb', host: 'h', url: 'mongodb://h/d' })
    expect(parsed('JDBC:postgresql://h/d').engine).toBe('postgres')
  })

  it('reads user and password from the query', () => {
    expect(parsed('jdbc:postgresql://h/d?user=u&password=p%40ss&ssl=true')).toMatchObject({
      user: 'u',
      password: 'p@ss',
      ssl: { mode: 'require' },
    })
    expect(parsed('jdbc:mysql://h/d?user=u&password=p&useSSL=false').ssl).toEqual({ mode: 'disable' })
  })

  it('reads jdbc:sqlite:path', () => {
    expect(parsed('jdbc:sqlite:/Users/me/app.db')).toMatchObject({ engine: 'sqlite', filePath: '/Users/me/app.db' })
    expect(parsed('jdbc:sqlite:app.db').filePath).toBe('app.db')
  })
})

describe('SQLite', () => {
  it('reads sqlite:///absolute/path as an absolute path', () => {
    expect(parsed('sqlite:///Users/me/data.db')).toEqual({ engine: 'sqlite', filePath: '/Users/me/data.db', name: 'data.db' })
  })

  it('treats four slashes (SQLAlchemy absolute) the same way', () => {
    expect(parsed('sqlite:////Users/me/data.db').filePath).toBe('/Users/me/data.db')
  })

  it('reads sqlite:/path and sqlite:relative.db', () => {
    expect(parsed('sqlite:/tmp/x.db').filePath).toBe('/tmp/x.db')
    expect(parsed('sqlite:x.db').filePath).toBe('x.db')
    expect(parsed('sqlite:./dir/x.db').filePath).toBe('./dir/x.db')
  })

  it('reads sqlite://relative/path and sqlite://~/path', () => {
    expect(parsed('sqlite://data/x.db').filePath).toBe('data/x.db')
    expect(parsed('sqlite://./x.db').filePath).toBe('./x.db')
    expect(parsed('sqlite://~/data/x.db').filePath).toBe('~/data/x.db')
  })

  it('reads sqlite3:// and driver suffixes', () => {
    expect(parsed('sqlite3:///tmp/x.db').filePath).toBe('/tmp/x.db')
    expect(parsed('sqlite+pysqlite:///tmp/x.db').filePath).toBe('/tmp/x.db')
    expect(parsed('sqlite+aiosqlite:////tmp/x.db').filePath).toBe('/tmp/x.db')
  })

  it('reads file: URLs in all their spellings', () => {
    expect(parsed('file:///Users/me/x.db').filePath).toBe('/Users/me/x.db')
    expect(parsed('file:/Users/me/x.db').filePath).toBe('/Users/me/x.db')
    expect(parsed('file://localhost/Users/me/x.db').filePath).toBe('/Users/me/x.db')
    expect(parsed('file:./dev.db').filePath).toBe('./dev.db')
    expect(parsed('file:dev.db').filePath).toBe('dev.db')
    expect(parsed('FILE:///x/y.sqlite').filePath).toBe('/x/y.sqlite')
  })

  it('percent-decodes the path', () => {
    expect(parsed('sqlite:///Users/me/My%20Data/x%23.db').filePath).toBe('/Users/me/My Data/x#.db')
    expect(parsed('file:///Users/me/My%20Data/x.db').filePath).toBe('/Users/me/My Data/x.db')
  })

  it('reads Windows drive paths', () => {
    expect(parsed('sqlite:///C:/Users/me/x.db').filePath).toBe('C:/Users/me/x.db')
    expect(parsed('file:///C:/Users/me/x.db').filePath).toBe('C:/Users/me/x.db')
    expect(parsed('sqlite:///D:\\data\\x.db').filePath).toBe('D:\\data\\x.db')
  })

  it('names the connection after the file', () => {
    expect(parsed('sqlite:///a/b/orders.sqlite3').name).toBe('orders.sqlite3')
    expect(parsed('sqlite:///C:/a/b/orders.db').name).toBe('orders.db')
  })

  it('drops the query, and mode=ro turns on read-only', () => {
    expect(parsed('file:x.db?cache=shared').filePath).toBe('x.db')
    expect(parsed('file:x.db?cache=shared').readOnly).toBeUndefined()
    expect(parsed('file:x.db?mode=ro')).toMatchObject({ filePath: 'x.db', readOnly: true })
    expect(parsed('sqlite:///x.db?mode=RO').readOnly).toBe(true)
    expect(parsed('file:x.db?mode=rwc').readOnly).toBeUndefined()
  })

  it('refuses in-memory databases and empty paths', () => {
    expect(error('sqlite://:memory:')).toMatch(/In-memory/)
    expect(error('sqlite:///:memory:')).toMatch(/In-memory/)
    expect(error('file::memory:')).toMatch(/In-memory/)
    expect(error('sqlite://')).toMatch(/no database file path/)
    expect(error('sqlite:')).toMatch(/no database file path/)
    expect(error('file:')).toMatch(/no database file path/)
  })

  it('reads a bare path to a database file', () => {
    expect(parsed('/Users/me/data.db')).toEqual({ engine: 'sqlite', filePath: '/Users/me/data.db', name: 'data.db' })
    expect(parsed('~/data/app.sqlite').filePath).toBe('~/data/app.sqlite')
    expect(parsed('./app.sqlite3').filePath).toBe('./app.sqlite3')
    expect(parsed('app.db').filePath).toBe('app.db')
    expect(parsed('C:\\Users\\me\\app.db')).toMatchObject({ engine: 'sqlite', filePath: 'C:\\Users\\me\\app.db', name: 'app.db' })
    expect(parsed('/x/y.DB').filePath).toBe('/x/y.DB')
    expect(parsed('/x/y.db3').filePath).toBe('/x/y.db3')
    expect(parsed('/x/y.s3db').filePath).toBe('/x/y.s3db')
  })

  it('does not decode percent signs in a bare path', () => {
    expect(parsed('/Users/me/100%25.db').filePath).toBe('/Users/me/100%25.db')
  })

  it('keeps spaces in a bare path and trims the ends', () => {
    expect(parsed('  /Users/me/My Data/app.db \n').filePath).toBe('/Users/me/My Data/app.db')
  })

  it('does not take other paths for databases', () => {
    expect(error('/Users/me/notes.txt')).toMatch(/does not look like a connection URL/)
    expect(error('/Users/me/app.database')).toMatch(/does not look like a connection URL/)
  })
})

describe('MongoDB', () => {
  it('keeps the URL and prefills host, port, user and database', () => {
    expect(parsed('mongodb://app@mongo.local:27018/shop?replicaSet=rs0')).toEqual({
      engine: 'mongodb',
      host: 'mongo.local',
      port: 27018,
      user: 'app',
      database: 'shop',
      url: 'mongodb://app@mongo.local:27018/shop?replicaSet=rs0',
      name: 'shop @ mongo.local',
    })
  })

  it('moves the password out of the URL, percent-decoded', () => {
    const p = parsed('mongodb://app:p%40ss%2F@mongo.local/shop?authSource=admin')
    expect(p.password).toBe('p@ss/')
    expect(p.url).toBe('mongodb://app@mongo.local/shop?authSource=admin')
    expect(p.url).not.toContain('p%40ss')
  })

  it('moves a raw @ password out too', () => {
    const p = parsed('mongodb://app:p@ss@mongo.local/shop')
    expect(p).toMatchObject({ user: 'app', password: 'p@ss', host: 'mongo.local' })
    expect(p.url).toBe('mongodb://app@mongo.local/shop')
  })

  it('moves a password with replacement-pattern characters out intact', () => {
    const p = parsed('mongodb://ap$&p:pa$&$1ss@mongo.local/shop')
    expect(p.password).toBe('pa$&$1ss')
    expect(p.url).toBe('mongodb://ap$&p@mongo.local/shop')
  })

  it('keeps a URL without credentials as it was', () => {
    expect(parsed('mongodb://127.0.0.1:27017').url).toBe('mongodb://127.0.0.1:27017')
    expect(parsed('mongodb://127.0.0.1:27017/?replicaSet=rs0&tls=true').url).toBe('mongodb://127.0.0.1:27017/?replicaSet=rs0&tls=true')
  })

  it('keeps a user without a password as it was', () => {
    expect(parsed('mongodb://app@h/db').url).toBe('mongodb://app@h/db')
  })

  it('reads mongodb+srv', () => {
    const p = parsed('mongodb+srv://app:pw@cluster0.abc12.mongodb.net/shop?retryWrites=true&w=majority')
    expect(p).toMatchObject({ engine: 'mongodb', host: 'cluster0.abc12.mongodb.net', user: 'app', password: 'pw', database: 'shop' })
    expect(p.port).toBeUndefined()
    expect(p.url).toBe('mongodb+srv://app@cluster0.abc12.mongodb.net/shop?retryWrites=true&w=majority')
  })

  it('uses the first host of a replica set but keeps the whole URL', () => {
    const p = parsed('mongodb://u:p@a.local:27017,b.local:27018,c.local/db?replicaSet=rs0')
    expect(p).toMatchObject({ host: 'a.local', port: 27017 })
    expect(p.url).toBe('mongodb://u@a.local:27017,b.local:27018,c.local/db?replicaSet=rs0')
  })

  it('reads IPv6 hosts', () => {
    expect(parsed('mongodb://[::1]:27018/db')).toMatchObject({ host: '::1', port: 27018, url: 'mongodb://[::1]:27018/db' })
  })

  it('leaves the SSL mode alone: the URL carries tls itself', () => {
    expect(parsed('mongodb://h/db?tls=true&tlsCAFile=%2Fca.pem').ssl).toBeUndefined()
  })

  it('reads what the URL says about TLS (the URL itself is kept)', () => {
    expect(parsed('mongodb+srv://c0.mongodb.net/db').tls).toBe(true)
    expect(parsed('mongodb://h/db?tls=true').tls).toBe(true)
    expect(parsed('mongodb://h/db?ssl=true').tls).toBe(true)
    expect(parsed('mongodb://h/db?tls=false').tls).toBe(false)
    expect(parsed('mongodb://h/db').tls).toBeUndefined()
    expect(parsed('mongodb://h/db?tls=maybe').tls).toBeUndefined()
    expect(parsed('postgres://h/db?tls=true').tls).toBeUndefined()
  })

  it('does not read query parameters as credentials', () => {
    const p = parsed('mongodb://h/db?user=x&password=y')
    expect(p.user).toBeUndefined()
    expect(p.password).toBeUndefined()
  })

  it('needs a host', () => {
    expect(error('mongodb:///db')).toMatch(/no host/)
    expect(error('mongodb://')).toMatch(/no host/)
  })

  it('has no database for a bare host URL', () => {
    expect(parsed('mongodb://h').database).toBeUndefined()
    expect(parsed('mongodb://h/').database).toBeUndefined()
    expect(parsed('mongodb://h/?x=1').database).toBeUndefined()
  })
})

describe('errors', () => {
  it('names an unknown scheme and lists the supported ones', () => {
    const e = error('redis://localhost:6379/0')
    expect(e).toContain('Unsupported URL scheme "redis"')
    expect(e).toMatch(/postgresql:\/\/.*mysql:\/\/.*sqlite:\/\/.*mongodb:\/\//)
    expect(error('https://example.com')).toContain('"https"')
    expect(error('mssql://sa@h/db')).toContain('"mssql"')
    expect(error('jdbc:oracle:thin://h')).not.toBe('')
  })

  it('asks for a URL when there is nothing', () => {
    expect(error('')).toBe('Paste a connection URL first.')
    expect(error('   \n ')).toBe('Paste a connection URL first.')
  })

  it('says what it expects when the text is not a URL', () => {
    expect(error('just some words')).toMatch(/does not look like a connection URL/)
    expect(error('localhost:5432')).toMatch(/does not look like a connection URL/)
    expect(error('db.example.com')).toMatch(/does not look like a connection URL/)
    expect(error('C:\\Users\\me\\notes.txt')).toMatch(/does not look like a connection URL/)
  })

  it('shows an example in the message', () => {
    expect(error('nonsense')).toContain('postgresql://user:password@host:5432/database')
  })
})

describe('pasted wrappers', () => {
  it('takes a URL out of quotes', () => {
    expect(parsed('"postgres://u:p@h/d"').host).toBe('h')
    expect(parsed("'postgres://u:p@h/d'").host).toBe('h')
    expect(parsed('`mysql://u@h/d`').engine).toBe('mysql')
    expect(parsed('"/Users/me/x.db"').filePath).toBe('/Users/me/x.db')
  })

  it('takes a URL out of a psql / mongosh command', () => {
    expect(parsed('psql "postgresql://u:p@h:5433/d?sslmode=require"')).toMatchObject({ host: 'h', port: 5433, ssl: { mode: 'require' } })
    expect(parsed("psql 'postgres://u@h/d'").database).toBe('d')
    expect(parsed('mongosh "mongodb+srv://c0.mongodb.net/db" --apiVersion 1')).toMatchObject({ host: 'c0.mongodb.net', url: 'mongodb+srv://c0.mongodb.net/db' })
  })

  it('takes a URL out of an env assignment', () => {
    expect(parsed('DATABASE_URL=postgres://u:p@h/d')).toMatchObject({ host: 'h', user: 'u' })
    expect(parsed('export DATABASE_URL="postgres://u:p@h/d"').host).toBe('h')
    expect(parsed('DB_URL=mysql://root@h:3307/d').port).toBe(3307)
  })

  it('trims surrounding whitespace and newlines', () => {
    expect(parsed('\n  postgres://u@h/d  \n').host).toBe('h')
  })

  it('stops at the end of the first line when more text follows', () => {
    const p = parsed('postgres://u:p@h:5433/d\nREDIS_URL=redis://cache\n')
    expect(p).toMatchObject({ host: 'h', port: 5433, database: 'd' })
    expect(parsed('sqlite:///a/b.db\r\nnext line').filePath).toBe('/a/b.db')
    expect(findUrl('mysql://u@h/d\nsecond')).toBe('mysql://u@h/d')
  })
})

describe('findUrl and looksLikeUrl', () => {
  it('finds the URL part', () => {
    expect(findUrl('postgres://u@h/d')).toBe('postgres://u@h/d')
    expect(findUrl('psql "postgres://u@h/d" -c x')).toBe('postgres://u@h/d')
    expect(findUrl('sqlite:x.db')).toBe('sqlite:x.db')
    expect(findUrl('/a/b.db')).toBe('/a/b.db')
    expect(findUrl('hello')).toBeNull()
    expect(findUrl('')).toBeNull()
  })

  it('recognises URLs of any scheme, so a wrong one can be reported', () => {
    expect(looksLikeUrl('postgres://u@h/d')).toBe(true)
    expect(looksLikeUrl('redis://h')).toBe(true)
    expect(looksLikeUrl('jdbc:mysql://h/d')).toBe(true)
    expect(looksLikeUrl('sqlite:///x.db')).toBe(true)
    expect(looksLikeUrl('file:x.db')).toBe(true)
    expect(looksLikeUrl('psql "postgres://u@h/d"')).toBe(true)
  })

  it('does not take host names, ports or plain paths for URLs', () => {
    expect(looksLikeUrl('db.example.com')).toBe(false)
    expect(looksLikeUrl('db.example.com:5432')).toBe(false)
    expect(looksLikeUrl('127.0.0.1')).toBe(false)
    expect(looksLikeUrl('::1')).toBe(false)
    expect(looksLikeUrl('/Users/me/x.db')).toBe(false)
    expect(looksLikeUrl('C:\\x.db')).toBe(false)
    expect(looksLikeUrl('')).toBe(false)
    expect(looksLikeUrl('postgres')).toBe(false)
  })
})

describe('applyConnectionUrl', () => {
  const fresh = (): ConnectionConfig => newConfig()
  const apply = (cfg: ConnectionConfig, url: string, setName = true) => applyConnectionUrl(cfg, parsed(url), { setName })

  it('fills the form from a Postgres URL', () => {
    const { config, password } = apply(fresh(), 'postgres://app:pw@db.example.com:5433/orders?sslmode=require')
    expect(config).toMatchObject({
      engine: 'postgres',
      host: 'db.example.com',
      port: 5433,
      user: 'app',
      database: 'orders',
      name: 'orders @ db.example.com',
      url: null,
    })
    expect(config.ssl).toMatchObject({ mode: 'require' })
    expect(password).toBe('pw')
  })

  it('never puts the password into the config', () => {
    const { config } = apply(fresh(), 'postgres://app:topsecret@h/d')
    expect(JSON.stringify(config)).not.toContain('topsecret')
    const mysql = apply(fresh(), 'mysql://app:topsecret@h/d')
    expect(JSON.stringify(mysql.config)).not.toContain('topsecret')
    const mongo = apply(fresh(), 'mongodb://app:topsecret@h/d')
    expect(JSON.stringify(mongo.config)).not.toContain('topsecret')
    expect(mongo.password).toBe('topsecret')
  })

  it('applies the engine default port when the URL names a host without one', () => {
    expect(apply(fresh(), 'postgres://h/d').config.port).toBe(5432)
    expect(apply(fresh(), 'mysql://h/d').config.port).toBe(3306)
    expect(apply(fresh(), 'mariadb://h/d').config.port).toBe(3306)
    expect(apply(fresh(), 'mongodb://h/d').config.port).toBe(27017)
    const typed = { ...fresh(), port: 6543 }
    expect(apply(typed, 'postgres://h/d').config.port).toBe(5432)
    expect(apply(typed, 'postgres://h:7000/d').config.port).toBe(7000)
  })

  it('leaves the port alone when the URL names no host and no port', () => {
    const typed = { ...fresh(), port: 6543 }
    expect(apply(typed, 'postgres:///d').config.port).toBe(6543)
    expect(apply(typed, 'postgresql:///d?port=7000').config.port).toBe(7000)
  })

  it('leaves pieces the URL does not name as they were', () => {
    const cfg: ConnectionConfig = { ...fresh(), host: 'old.host', user: 'olduser', database: 'olddb', name: 'Mine' }
    const { config, password } = apply(cfg, 'postgres://new.host', false)
    expect(config).toMatchObject({ host: 'new.host', user: 'olduser', database: 'olddb', name: 'Mine' })
    expect(config.ssl).toEqual(cfg.ssl)
    expect(password).toBeNull()
  })

  it('switches the engine like the engine tiles: default port and user follow', () => {
    const { config } = apply(fresh(), 'mysql://h/d')
    expect(config).toMatchObject({ engine: 'mysql', port: 3306, user: 'root', host: 'h', database: 'd' })
    const mongo = apply(fresh(), 'mongodb://h')
    expect(mongo.config).toMatchObject({ engine: 'mongodb', port: 27017, user: '' })
  })

  it('keeps a typed user when switching engines', () => {
    const cfg = { ...fresh(), user: 'me' }
    expect(apply(cfg, 'mysql://h/d').config.user).toBe('me')
  })

  it('keeps a MariaDB form on MariaDB for a mysql:// URL, and takes mariadb:// anywhere', () => {
    const maria: ConnectionConfig = { ...newConfig('mariadb') }
    expect(apply(maria, 'mysql://h/d').config.engine).toBe('mariadb')
    expect(apply(newConfig('mysql'), 'mariadb://h/d').config.engine).toBe('mariadb')
    expect(apply(newConfig('postgres'), 'mysql://h/d').config.engine).toBe('mysql')
    expect(apply(maria, 'postgres://h/d').config.engine).toBe('postgres')
  })

  it('switches to SQLite and sets the file', () => {
    const { config, password } = apply(fresh(), 'sqlite:///Users/me/app.db')
    expect(config).toMatchObject({ engine: 'sqlite', filePath: '/Users/me/app.db', name: 'app.db', url: null, port: 0, user: '' })
    expect(password).toBeNull()
  })

  it('switches from SQLite to a network engine with a usable host', () => {
    const { config } = apply(newConfig('sqlite'), 'postgres:///d')
    expect(config).toMatchObject({ engine: 'postgres', host: '127.0.0.1', port: 5432, user: 'postgres', database: 'd' })
  })

  it('turns on read-only for mode=ro, but never turns it off', () => {
    expect(apply(fresh(), 'file:x.db?mode=ro').config.readOnly).toBe(true)
    expect({ ...fresh(), readOnly: true }).toMatchObject({ readOnly: true })
    expect(apply({ ...fresh(), readOnly: true }, 'file:x.db').config.readOnly).toBe(true)
  })

  it('keeps the URL for MongoDB and clears it for everyone else', () => {
    const mongo = apply(fresh(), 'mongodb+srv://app:pw@c0.mongodb.net/shop').config
    expect(mongo.url).toBe('mongodb+srv://app@c0.mongodb.net/shop')
    expect(mongo).toMatchObject({ engine: 'mongodb', host: 'c0.mongodb.net', user: 'app', database: 'shop' })
    expect(apply(mongo, 'postgres://h/d').config.url).toBeNull()
  })

  it('merges SSL settings over the current ones', () => {
    const cfg: ConnectionConfig = { ...fresh(), ssl: { mode: 'prefer', caCert: '/old-ca.pem', clientCert: null, clientKey: null } }
    expect(apply(cfg, 'postgres://h/d?sslmode=verify-full').config.ssl).toEqual({
      mode: 'verifyFull',
      caCert: '/old-ca.pem',
      clientCert: null,
      clientKey: null,
    })
    expect(apply(cfg, 'postgres://h/d?sslrootcert=/new.pem').config.ssl).toMatchObject({ mode: 'prefer', caCert: '/new.pem' })
    expect(apply(cfg, 'postgres://h/d').config.ssl).toEqual(cfg.ssl)
  })

  it('keeps TLS on for a MongoDB URL that asks for it, even when the form had SSL disabled', () => {
    const off: ConnectionConfig = { ...fresh(), ssl: { mode: 'disable', caCert: null, clientCert: null, clientKey: null } }
    expect(apply(off, 'mongodb+srv://c0.mongodb.net/db').config.ssl.mode).toBe('prefer')
    expect(apply(off, 'mongodb://h/db?tls=true').config.ssl.mode).toBe('prefer')
    // A stricter mode the person chose stays.
    const strict: ConnectionConfig = { ...off, ssl: { ...off.ssl, mode: 'verifyFull' } }
    expect(apply(strict, 'mongodb+srv://c0.mongodb.net/db').config.ssl.mode).toBe('verifyFull')
    // And a URL that switches TLS off is not overridden by an SSL mode that switches it on.
    expect(apply(strict, 'mongodb://h/db?tls=false').config.ssl.mode).toBe('disable')
    expect(apply(fresh(), 'mongodb://h/db').config.ssl.mode).toBe('prefer')
  })

  it('fills the name only when asked to', () => {
    expect(apply({ ...fresh(), name: 'Mine' }, 'postgres://h/d', true).config.name).toBe('d @ h')
    expect(apply({ ...fresh(), name: 'Mine' }, 'postgres://h/d', false).config.name).toBe('Mine')
    expect(applyConnectionUrl({ ...fresh(), name: 'Mine' }, parsed('postgres://h/d')).config.name).toBe('Mine')
  })

  it('does not change the config it was given', () => {
    const cfg = fresh()
    const before = structuredClone(cfg)
    apply(cfg, 'mysql://u:p@h:3307/d?ssl-mode=REQUIRED')
    expect(cfg).toEqual(before)
  })

  it('imports IPv6 hosts without brackets', () => {
    expect(apply(fresh(), 'postgres://[::1]:5433/d').config).toMatchObject({ host: '::1', port: 5433 })
  })

  it('describes what was imported', () => {
    const a = apply(fresh(), 'postgres://app:pw@db.example.com:5433/orders')
    expect(describeImport(a.config, a.password)).toBe('PostgreSQL · app@db.example.com:5433/orders · password set')
    const b = apply(fresh(), 'sqlite:///tmp/x.db')
    expect(describeImport(b.config, b.password)).toBe('SQLite · /tmp/x.db')
    const c = apply(fresh(), 'mongodb://app:pw@h:27017/db')
    expect(describeImport(c.config, c.password)).toBe('MongoDB · mongodb://h:27017/db · password set')
  })
})

describe('connectionUrl', () => {
  const pg = (over: Partial<ConnectionConfig> = {}): ConnectionConfig => ({
    ...newConfig('postgres'),
    host: 'db.example.com',
    port: 5433,
    user: 'app',
    database: 'orders',
    ...over,
  })

  it('builds a Postgres URL without a password', () => {
    expect(connectionUrl(pg())).toBe('postgresql://app@db.example.com:5433/orders')
  })

  it('builds MySQL and MariaDB URLs', () => {
    expect(connectionUrl({ ...pg(), engine: 'mysql', port: 3306, user: 'root', database: 'shop' })).toBe('mysql://root@db.example.com:3306/shop')
    expect(connectionUrl({ ...pg(), engine: 'mariadb', port: 3307 })).toBe('mariadb://app@db.example.com:3307/orders')
  })

  it('leaves out the user and database when empty', () => {
    expect(connectionUrl(pg({ user: '', database: '' }))).toBe('postgresql://db.example.com:5433')
  })

  it('percent-encodes the user and database', () => {
    expect(connectionUrl(pg({ user: 'me@corp', database: 'my db/x' }))).toBe('postgresql://me%40corp@db.example.com:5433/my%20db%2Fx')
  })

  it('brackets IPv6 hosts', () => {
    expect(connectionUrl(pg({ host: '::1' }))).toBe('postgresql://app@[::1]:5433/orders')
    expect(connectionUrl(pg({ host: '[::1]' }))).toBe('postgresql://app@[::1]:5433/orders')
  })

  it('adds the SSL mode when it is not the default', () => {
    const ssl = (mode: ConnectionConfig['ssl']['mode']) => connectionUrl(pg({ ssl: { mode } }))
    expect(ssl('prefer')).toBe('postgresql://app@db.example.com:5433/orders')
    expect(ssl('disable')).toBe('postgresql://app@db.example.com:5433/orders?sslmode=disable')
    expect(ssl('require')).toBe('postgresql://app@db.example.com:5433/orders?sslmode=require')
    expect(ssl('verifyCa')).toBe('postgresql://app@db.example.com:5433/orders?sslmode=verify-ca')
    expect(ssl('verifyFull')).toBe('postgresql://app@db.example.com:5433/orders?sslmode=verify-full')
    const my = (mode: ConnectionConfig['ssl']['mode']) => connectionUrl({ ...pg({ ssl: { mode } }), engine: 'mysql', port: 3306 })
    expect(my('prefer')).toBe('mysql://app@db.example.com:3306/orders')
    expect(my('disable')).toBe('mysql://app@db.example.com:3306/orders?ssl-mode=DISABLED')
    expect(my('require')).toBe('mysql://app@db.example.com:3306/orders?ssl-mode=REQUIRED')
    expect(my('verifyCa')).toBe('mysql://app@db.example.com:3306/orders?ssl-mode=VERIFY_CA')
    expect(my('verifyFull')).toBe('mysql://app@db.example.com:3306/orders?ssl-mode=VERIFY_IDENTITY')
  })

  it('puts the query after a slash when there is no database', () => {
    expect(connectionUrl(pg({ database: '', ssl: { mode: 'require' } }))).toBe('postgresql://app@db.example.com:5433/?sslmode=require')
  })

  it('returns null while there is nothing to point at', () => {
    expect(connectionUrl(pg({ host: '' }))).toBeNull()
    expect(connectionUrl(pg({ host: '   ' }))).toBeNull()
    expect(connectionUrl({ ...newConfig('sqlite'), filePath: null })).toBeNull()
    expect(connectionUrl({ ...newConfig('sqlite'), filePath: '  ' })).toBeNull()
  })

  it('builds SQLite URLs for absolute, relative, tilde and Windows paths', () => {
    const lite = (filePath: string) => connectionUrl({ ...newConfig('sqlite'), filePath })
    expect(lite('/Users/me/app.db')).toBe('sqlite:///Users/me/app.db')
    expect(lite('~/data/nasty.db')).toBe('sqlite://~/data/nasty.db')
    expect(lite('data/app.db')).toBe('sqlite://data/app.db')
    expect(lite('C:\\Users\\me\\app.db')).toBe('sqlite:///C:/Users/me/app.db')
    expect(lite('/Users/me/My Data/a#b?.db')).toBe('sqlite:///Users/me/My%20Data/a%23b%3F.db')
  })

  it('builds MongoDB URLs from the fields, with tls from the SSL mode', () => {
    const mongo: ConnectionConfig = { ...newConfig('mongodb'), host: 'mongo.local', port: 27117, user: 'app', database: 'shop' }
    expect(connectionUrl(mongo)).toBe('mongodb://app@mongo.local:27117/shop')
    expect(connectionUrl({ ...mongo, database: '' })).toBe('mongodb://app@mongo.local:27117')
    expect(connectionUrl({ ...mongo, ssl: { mode: 'require' } })).toBe('mongodb://app@mongo.local:27117/shop?tls=true')
    expect(connectionUrl({ ...mongo, database: '', ssl: { mode: 'verifyFull' } })).toBe('mongodb://app@mongo.local:27117/?tls=true')
    expect(connectionUrl({ ...mongo, ssl: { mode: 'disable' } })).toBe('mongodb://app@mongo.local:27117/shop?tls=false')
  })

  it('uses the saved MongoDB URL and strips any password it still carries', () => {
    const mongo: ConnectionConfig = { ...newConfig('mongodb'), url: 'mongodb+srv://app:hunter2@c0.mongodb.net/shop?w=majority' }
    expect(connectionUrl(mongo)).toBe('mongodb+srv://app@c0.mongodb.net/shop?w=majority')
    expect(connectionUrl({ ...mongo, url: ' mongodb://h:27017/db ' })).toBe('mongodb://h:27017/db')
    expect(connectionUrl({ ...mongo, url: 'mongodb://app:p%40ss@a:1,b:2/db' })).toBe('mongodb://app@a:1,b:2/db')
    expect(connectionUrl({ ...mongo, url: 'mongodb://a:1,b:2/db?x=a:b@c' })).toBe('mongodb://a:1,b:2/db?x=a:b@c')
  })

  it('strips a password that has a raw @ or replacement-pattern characters', () => {
    const mongo: ConnectionConfig = { ...newConfig('mongodb'), url: 'mongodb://app:p@ss$&@mongo.local/shop' }
    expect(connectionUrl(mongo)).toBe('mongodb://app@mongo.local/shop')
  })

  it('never contains a password', () => {
    for (const c of [pg(), { ...pg(), engine: 'mysql' as const }, { ...newConfig('mongodb'), host: 'h', url: 'mongodb://u:topsecret@h/d' }]) {
      expect(connectionUrl(c)).not.toContain('topsecret')
    }
  })

  describe('round trip', () => {
    // The URL of `c` applied to a form holding something else entirely.
    const roundTrip = (c: ConnectionConfig) => {
      const scrambled: ConnectionConfig = { ...newConfig(c.engine), user: 'x', database: 'y', host: 'z', port: 1, filePath: '/nope.db' }
      return applyConnectionUrl(scrambled, parsed(connectionUrl(c)!), { setName: false }).config
    }

    it('Postgres', () => {
      const c = pg({ user: 'me@corp', database: 'my db', ssl: { mode: 'verifyFull' } })
      const back = roundTrip(c)
      expect(back).toMatchObject({ engine: 'postgres', host: c.host, port: c.port, user: c.user, database: c.database })
      expect(back.ssl.mode).toBe('verifyFull')
    })

    it('MySQL, MariaDB', () => {
      for (const engine of ['mysql', 'mariadb'] as const) {
        const c: ConnectionConfig = { ...pg({ ssl: { mode: 'require' } }), engine, port: 3307 }
        expect(roundTrip(c)).toMatchObject({ engine, host: c.host, port: 3307, user: c.user, database: c.database, ssl: { mode: 'require' } })
      }
    })

    it('IPv6', () => {
      expect(roundTrip(pg({ host: '2001:db8::7' }))).toMatchObject({ host: '2001:db8::7', port: 5433 })
    })

    it('SQLite paths', () => {
      for (const filePath of ['/Users/me/My Data/app.db', '~/data/nasty.db', 'data/app.db', 'C:/Users/me/app.db', '/tmp/a#b.db']) {
        expect(roundTrip({ ...newConfig('sqlite'), filePath })).toMatchObject({ engine: 'sqlite', filePath })
      }
    })

    it('MongoDB', () => {
      const c: ConnectionConfig = { ...newConfig('mongodb'), host: 'mongo.local', port: 27117, user: 'app', database: 'shop', ssl: { mode: 'require' } }
      const url = connectionUrl(c)!
      expect(parsed(url)).toMatchObject({ host: 'mongo.local', port: 27117, user: 'app', database: 'shop', url })
    })
  })
})

describe('splitUrlPassword', () => {
  it('takes the password out of the credentials and decodes it', () => {
    expect(splitUrlPassword('mongodb://app:p%40ss@h:27017/db?x=1')).toEqual({ url: 'mongodb://app@h:27017/db?x=1', password: 'p@ss' })
  })

  it('ends the credentials at the last @ of the authority, not at one in the query', () => {
    expect(splitUrlPassword('mongodb://app:p@ss@h/db?authSource=a@b')).toEqual({ url: 'mongodb://app@h/db?authSource=a@b', password: 'p@ss' })
  })

  it('drops the @ when there is no user', () => {
    expect(splitUrlPassword('mongodb://:pw@h/db')).toEqual({ url: 'mongodb://h/db', password: 'pw' })
  })

  it('leaves URLs without a password exactly as they are', () => {
    for (const url of ['mongodb://h/db', 'mongodb://app@h/db', 'mongodb://app:@h/db', 'mongodb://a:1,b:2/db?x=a:b@c', 'not a url']) {
      expect(splitUrlPassword(url)).toEqual({ url, password: null })
    }
  })

  it('works on mongodb+srv and multi-host URLs', () => {
    expect(splitUrlPassword('mongodb+srv://u:pw@c0.net/db')).toEqual({ url: 'mongodb+srv://u@c0.net/db', password: 'pw' })
    expect(splitUrlPassword('mongodb://u:pw@a:1,b:2/db?replicaSet=r')).toEqual({ url: 'mongodb://u@a:1,b:2/db?replicaSet=r', password: 'pw' })
  })
})

describe('describeConnection with a MongoDB URL', () => {
  it('shows no part of a password that has a raw @', () => {
    const c: ConnectionConfig = { ...newConfig('mongodb'), url: 'mongodb://app:p@ss@mongo.local:27017/shop' }
    expect(describeConnection(c)).toBe('mongodb://mongo.local:27017/shop')
  })
})
