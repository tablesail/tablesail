import type { Engine, TableRef } from '../types'

export function quoteIdent(engine: Engine, name: string): string {
  if (engine === 'mysql' || engine === 'mariadb') return '`' + name.replaceAll('`', '``') + '`'
  return '"' + name.replaceAll('"', '""') + '"'
}

export function qualifiedName(engine: Engine, table: TableRef): string {
  const q = (n: string) => quoteIdent(engine, n)
  if (engine === 'postgres') return table.schema ? `${q(table.schema)}.${q(table.name)}` : q(table.name)
  if ((engine === 'mysql' || engine === 'mariadb') && table.database) return `${q(table.database)}.${q(table.name)}`
  return q(table.name)
}

/** Starter text for a new query tab opened from a table. Empty for engines without SQL. */
export function selectStarter(engine: Engine, table: TableRef): string {
  if (engine === 'mongodb') return ''
  return `SELECT * FROM ${qualifiedName(engine, table)} LIMIT 100;\n`
}

export function plainName(table: TableRef): string {
  return table.schema ? `${table.schema}.${table.name}` : table.name
}
