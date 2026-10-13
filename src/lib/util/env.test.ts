import { describe, expect, it } from 'vitest'
import type { ConnectionConfig, Engine } from '../types'
import { DEFAULT_PORT, ENGINE_LABEL, ENV_TAGS, defaultUser, describeConnection, envColor, newConfig } from './env'

const ENGINES: Engine[] = ['sqlite', 'postgres', 'mysql', 'mariadb', 'mongodb']

describe('constants', () => {
  it('lists environments from safest to most dangerous', () => {
    expect(ENV_TAGS).toEqual(['local', 'development', 'staging', 'production'])
  })

  it('has a label and a default port for every engine', () => {
    for (const e of ENGINES) {
      expect(ENGINE_LABEL[e]).toBeTruthy()
      expect(DEFAULT_PORT[e]).toBeGreaterThanOrEqual(0)
    }
  })

  it('uses the well-known default ports', () => {
    expect(DEFAULT_PORT).toEqual({ sqlite: 0, postgres: 5432, mysql: 3306, mariadb: 3306, mongodb: 27017 })
  })

  it('labels engines with their proper product names', () => {
    expect(ENGINE_LABEL.postgres).toBe('PostgreSQL')
    expect(ENGINE_LABEL.mongodb).toBe('MongoDB')
    expect(ENGINE_LABEL.sqlite).toBe('SQLite')
  })
})

describe('envColor', () => {
  it('uses the custom colour when set', () => {
    expect(envColor({ env: 'production', color: '#ff0000' })).toBe('#ff0000')
  })

  it('falls back to the environment CSS variable', () => {
    expect(envColor({ env: 'production', color: null })).toBe('var(--env-production)')
    expect(envColor({ env: 'staging', color: undefined })).toBe('var(--env-staging)')
    expect(envColor({ env: 'local', color: '' })).toBe('var(--env-local)')
  })
})

describe('defaultUser', () => {
  it('is postgres for Postgres, root for MySQL family, empty otherwise', () => {
    expect(defaultUser('postgres')).toBe('postgres')
    expect(defaultUser('mysql')).toBe('root')
    expect(defaultUser('mariadb')).toBe('root')
    expect(defaultUser('sqlite')).toBe('')
    expect(defaultUser('mongodb')).toBe('')
  })
})

describe('newConfig', () => {
  it('defaults to a local Postgres connection', () => {
    const c = newConfig()
    expect(c).toMatchObject({
      id: '',
      name: '',
      engine: 'postgres',
      host: '127.0.0.1',
      port: 5432,
      user: 'postgres',
      database: '',
      env: 'local',
      readOnly: false,
    })
    expect(c.ssl.mode).toBe('prefer')
    expect(c.ssh).toBeNull()
    expect(c.options).toEqual({})
  })

  it.each(ENGINES)('%s gets its own default port and user', (engine) => {
    const c = newConfig(engine)
    expect(c.engine).toBe(engine)
    expect(c.port).toBe(DEFAULT_PORT[engine])
    expect(c.user).toBe(defaultUser(engine))
  })

  it('sqlite has no host', () => {
    expect(newConfig('sqlite').host).toBe('')
    expect(newConfig('sqlite').port).toBe(0)
  })

  it('returns independent objects (no shared nested state)', () => {
    const a = newConfig()
    const b = newConfig()
    a.options.x = '1'
    a.ssl.mode = 'require'
    expect(b.options).toEqual({})
    expect(b.ssl.mode).toBe('prefer')
  })
})

describe('describeConnection', () => {
  const base = (over: Partial<ConnectionConfig>): ConnectionConfig => ({ ...newConfig('postgres'), ...over })

  it('sqlite shows the file path', () => {
    expect(describeConnection(base({ engine: 'sqlite', filePath: '/tmp/a.db' }))).toBe('/tmp/a.db')
    expect(describeConnection(base({ engine: 'sqlite', filePath: null }))).toBe('')
    expect(describeConnection(base({ engine: 'sqlite', filePath: undefined }))).toBe('')
  })

  it('server engines show user@host:port/database', () => {
    expect(describeConnection(base({ user: 'bob', host: 'db.example.com', port: 5433, database: 'shop' }))).toBe(
      'bob@db.example.com:5433/shop',
    )
  })

  it('omits the user and database when empty', () => {
    expect(describeConnection(base({ user: '', host: 'h', port: 1, database: '' }))).toBe('h:1')
  })

  it('works for MySQL', () => {
    expect(describeConnection(base({ engine: 'mysql', user: 'root', host: '127.0.0.1', port: 3306, database: 'app' }))).toBe(
      'root@127.0.0.1:3306/app',
    )
  })

  it('mongodb with a URL hides credentials', () => {
    const c = base({ engine: 'mongodb', url: 'mongodb://admin:s3cret@cluster0.example.net:27017/app?retryWrites=true' })
    const out = describeConnection(c)
    expect(out).toBe('mongodb://cluster0.example.net:27017/app?retryWrites=true')
    expect(out).not.toContain('s3cret')
    expect(out).not.toContain('admin')
  })

  it('mongodb+srv URLs hide credentials too', () => {
    expect(describeConnection(base({ engine: 'mongodb', url: 'mongodb+srv://u:p@cluster.mongodb.net/db' }))).toBe(
      'mongodb+srv://cluster.mongodb.net/db',
    )
  })

  it('mongodb URL without credentials is untouched', () => {
    expect(describeConnection(base({ engine: 'mongodb', url: 'mongodb://localhost:27017' }))).toBe('mongodb://localhost:27017')
  })

  it('an @ in the query string is not mistaken for credentials', () => {
    expect(describeConnection(base({ engine: 'mongodb', url: 'mongodb://host/db?x=a@b' }))).toBe('mongodb://host/db?x=a@b')
  })

  it('mongodb without a URL falls back to host:port', () => {
    expect(describeConnection(base({ engine: 'mongodb', url: null, user: '', host: 'h', port: 27017, database: '' }))).toBe('h:27017')
  })
})
