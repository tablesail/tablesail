import type { ConnectionConfig, Engine, EnvTag } from '../types'

export const ENV_TAGS: EnvTag[] = ['local', 'development', 'staging', 'production']

export function envColor(config: Pick<ConnectionConfig, 'env' | 'color'>): string {
  return config.color || `var(--env-${config.env})`
}

export const ENGINE_LABEL: Record<Engine, string> = {
  sqlite: 'SQLite',
  postgres: 'PostgreSQL',
  mysql: 'MySQL',
  mariadb: 'MariaDB',
  mongodb: 'MongoDB',
}

export const DEFAULT_PORT: Record<Engine, number> = {
  sqlite: 0,
  postgres: 5432,
  mysql: 3306,
  mariadb: 3306,
  mongodb: 27017,
}

export function defaultUser(engine: Engine): string {
  return engine === 'postgres' ? 'postgres' : engine === 'mysql' || engine === 'mariadb' ? 'root' : ''
}

export function newConfig(engine: Engine = 'postgres'): ConnectionConfig {
  return {
    id: '',
    name: '',
    engine,
    host: engine === 'sqlite' ? '' : '127.0.0.1',
    port: DEFAULT_PORT[engine],
    user: defaultUser(engine),
    database: '',
    filePath: null,
    url: null,
    ssl: { mode: 'prefer', caCert: null, clientCert: null, clientKey: null },
    ssh: null,
    env: 'local',
    color: null,
    readOnly: false,
    options: {},
  }
}

export function describeConnection(c: ConnectionConfig): string {
  if (c.engine === 'sqlite') return c.filePath ?? ''
  if (c.engine === 'mongodb' && c.url) return c.url.replace(/\/\/[^@/]*@/, '//')
  const db = c.database ? `/${c.database}` : ''
  return `${c.user ? c.user + '@' : ''}${c.host}:${c.port}${db}`
}
