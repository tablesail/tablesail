import { describe, expect, it } from 'vitest'
import type { Engine, TableRef } from '../types'
import { plainName, qualifiedName, quoteIdent, selectStarter } from './sql'

const ENGINES: Engine[] = ['sqlite', 'postgres', 'mysql', 'mariadb', 'mongodb']
const t = (name: string, schema?: string | null, database = 'db'): TableRef => ({ database, schema, name })

describe('quoteIdent', () => {
  it('uses backticks for MySQL and MariaDB', () => {
    expect(quoteIdent('mysql', 'users')).toBe('`users`')
    expect(quoteIdent('mariadb', 'users')).toBe('`users`')
  })

  it('uses double quotes for everything else', () => {
    for (const e of ['sqlite', 'postgres', 'mongodb'] as Engine[]) expect(quoteIdent(e, 'users')).toBe('"users"')
  })

  it('doubles embedded backticks for MySQL', () => {
    expect(quoteIdent('mysql', 'we`ird')).toBe('`we``ird`')
    expect(quoteIdent('mariadb', '``')).toBe('````' + '``')
  })

  it('doubles embedded double quotes elsewhere', () => {
    expect(quoteIdent('postgres', 'we"ird')).toBe('"we""ird"')
    expect(quoteIdent('sqlite', '"')).toBe('""""')
  })

  it('leaves the other engine\'s quote character alone', () => {
    expect(quoteIdent('mysql', 'a"b')).toBe('`a"b`')
    expect(quoteIdent('postgres', 'a`b')).toBe('"a`b"')
  })

  it('quotes injection attempts so they stay one identifier', () => {
    expect(quoteIdent('postgres', 'x"; DROP TABLE users; --')).toBe('"x""; DROP TABLE users; --"')
    expect(quoteIdent('mysql', 'x`; DROP TABLE users; --')).toBe('`x``; DROP TABLE users; --`')
  })

  it('quotes empty names, spaces, unicode and reserved words', () => {
    expect(quoteIdent('postgres', '')).toBe('""')
    expect(quoteIdent('postgres', 'my table')).toBe('"my table"')
    expect(quoteIdent('mysql', 'select')).toBe('`select`')
    expect(quoteIdent('sqlite', 'naïve_表')).toBe('"naïve_表"')
  })
})

describe('qualifiedName', () => {
  it('postgres: schema.table when a schema is given', () => {
    expect(qualifiedName('postgres', t('users', 'public'))).toBe('"public"."users"')
  })

  it('postgres: bare table without schema (null, undefined, empty)', () => {
    expect(qualifiedName('postgres', t('users'))).toBe('"users"')
    expect(qualifiedName('postgres', t('users', null))).toBe('"users"')
    expect(qualifiedName('postgres', t('users', ''))).toBe('"users"')
  })

  it('postgres never prefixes the database', () => {
    expect(qualifiedName('postgres', t('users', 'public', 'mydb'))).not.toContain('mydb')
  })

  it('mysql/mariadb: database.table with backticks', () => {
    expect(qualifiedName('mysql', t('users', null, 'shop'))).toBe('`shop`.`users`')
    expect(qualifiedName('mariadb', t('users', null, 'shop'))).toBe('`shop`.`users`')
  })

  it('mysql ignores a schema and falls back to the bare table with no database', () => {
    expect(qualifiedName('mysql', t('users', 'ignored', 'shop'))).toBe('`shop`.`users`')
    expect(qualifiedName('mysql', t('users', null, ''))).toBe('`users`')
  })

  it('sqlite: bare table even when database/schema are set', () => {
    expect(qualifiedName('sqlite', t('users', 'main', 'main'))).toBe('"users"')
  })

  it('escapes every part', () => {
    expect(qualifiedName('postgres', t('a"b', 'c"d'))).toBe('"c""d"."a""b"')
    expect(qualifiedName('mysql', t('a`b', null, 'c`d'))).toBe('`c``d`.`a``b`')
  })
})

describe('selectStarter', () => {
  it('postgres', () => {
    expect(selectStarter('postgres', t('users', 'public'))).toBe('SELECT * FROM "public"."users" LIMIT 100;\n')
  })

  it('mysql and mariadb', () => {
    expect(selectStarter('mysql', t('orders', null, 'shop'))).toBe('SELECT * FROM `shop`.`orders` LIMIT 100;\n')
    expect(selectStarter('mariadb', t('orders', null, 'shop'))).toBe('SELECT * FROM `shop`.`orders` LIMIT 100;\n')
  })

  it('sqlite', () => {
    expect(selectStarter('sqlite', t('notes', null, 'main'))).toBe('SELECT * FROM "notes" LIMIT 100;\n')
  })

  it('mongodb has no SQL, so the starter is empty', () => {
    expect(selectStarter('mongodb', t('events'))).toBe('')
  })

  it('every SQL engine ends with a LIMIT and a trailing newline', () => {
    for (const e of ENGINES.filter((x) => x !== 'mongodb')) {
      const s = selectStarter(e, t('x', 'public'))
      expect(s.startsWith('SELECT * FROM ')).toBe(true)
      expect(s.endsWith('LIMIT 100;\n')).toBe(true)
    }
  })
})

describe('plainName', () => {
  it('joins schema and name', () => {
    expect(plainName(t('users', 'public'))).toBe('public.users')
  })
  it('returns just the name without a schema', () => {
    expect(plainName(t('users'))).toBe('users')
    expect(plainName(t('users', null))).toBe('users')
    expect(plainName(t('users', ''))).toBe('users')
  })
  it('does not quote', () => {
    expect(plainName(t('my table', 'my schema'))).toBe('my schema.my table')
  })
})
