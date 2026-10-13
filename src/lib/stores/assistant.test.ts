import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage, routeInvoke, settle, type InvokeHandlers } from '../../test/helpers'
import type { AssistantRequest, CoreError, SchemaSnapshot, SessionInfo, TableInfo, TableKind } from '../types'
import { newConfig } from '../util/env'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

const coreError = (kind: CoreError['kind'], message: string): CoreError => ({ kind, message, code: null, statement: null })

function table(name: string, schema: string | null = 'public', kind: TableKind = 'table'): TableInfo {
  return { database: 'db', schema, name, kind, estimatedRows: null, comment: null }
}

function snap(tables: TableInfo[], columns: Record<string, string[]> = {}): SchemaSnapshot {
  const schemas: Record<string, TableInfo[]> = {}
  for (const t of tables) (schemas[t.schema ?? ''] ??= []).push(t)
  return { database: 'db', schemas, columns, fetchedAt: 0 }
}

async function pureFns() {
  vi.resetModules()
  const m = await import('./assistant.svelte')
  return { buildSchemaContext: m.buildSchemaContext, firstTableName: m.firstTableName }
}

describe('buildSchemaContext', () => {
  let buildSchemaContext: Awaited<ReturnType<typeof pureFns>>['buildSchemaContext']
  beforeEach(async () => {
    buildSchemaContext = (await pureFns()).buildSchemaContext
  })

  it('is empty without a snapshot', () => {
    expect(buildSchemaContext(null)).toBe('')
  })

  it('is empty for a database with no tables', () => {
    expect(buildSchemaContext(snap([]))).toBe('')
  })

  it('lists one `schema.table(col, col)` per line', () => {
    const s = snap([table('users'), table('orders')], { 'public.users': ['id', 'email'], 'public.orders': ['id', 'user_id', 'total'] })
    expect(buildSchemaContext(s)).toBe('public.users(id, email)\npublic.orders(id, user_id, total)')
  })

  it('has no trailing newline', () => {
    expect(buildSchemaContext(snap([table('a')], { 'public.a': ['x'] })).endsWith('\n')).toBe(false)
  })

  it('uses the bare table name for engines without schemas', () => {
    const s = snap([table('notes', null)], { notes: ['id', 'body'] })
    expect(buildSchemaContext(s)).toBe('notes(id, body)')
  })

  it('falls back to columns keyed by the bare table name', () => {
    const s = snap([table('users', 'public')], { users: ['id'] })
    expect(buildSchemaContext(s)).toBe('public.users(id)')
  })

  it('prefers the qualified key when both exist', () => {
    const s = snap([table('users', 'public')], { 'public.users': ['a'], users: ['b'] })
    expect(buildSchemaContext(s)).toBe('public.users(a)')
  })

  it('lists a table without column info by name only', () => {
    expect(buildSchemaContext(snap([table('mystery')]))).toBe('public.mystery')
  })

  it('a table with zero columns prints empty parentheses', () => {
    expect(buildSchemaContext(snap([table('empty')], { 'public.empty': [] }))).toBe('public.empty()')
  })

  it('marks views and materialized views', () => {
    const s = snap([table('v', 'public', 'view'), table('mv', 'public', 'materializedView')], { 'public.v': ['a'], 'public.mv': ['b'] })
    expect(buildSchemaContext(s)).toBe('public.v(a) -- view\npublic.mv(b) -- materialized view')
  })

  it('skips sequences and system tables', () => {
    const s = snap([table('t'), table('t_id_seq', 'public', 'sequence'), table('pg_class', 'pg_catalog', 'system')])
    expect(buildSchemaContext(s)).toBe('public.t')
  })

  it('keeps other kinds (foreign tables, collections, virtual, time series)', () => {
    const kinds: TableKind[] = ['foreignTable', 'collection', 'virtual', 'timeSeries']
    const s = snap(kinds.map((k, i) => table(`t${i}`, null, k)))
    expect(buildSchemaContext(s).split('\n')).toEqual(['t0', 't1', 't2', 't3'])
  })

  it('walks schemas in their listed order', () => {
    const s = snap([table('a', 'zeta'), table('b', 'alpha')])
    expect(buildSchemaContext(s)).toBe('zeta.a\nalpha.b')
  })

  describe('size cap', () => {
    const CAP = 30_000
    const big = (n: number, colCount = 20) => {
      const tables = Array.from({ length: n }, (_, i) => table(`table_number_${i}`))
      const columns = Object.fromEntries(tables.map((t) => [`public.${t.name}`, Array.from({ length: colCount }, (_, c) => `column_${c}`)]))
      return snap(tables, columns)
    }

    it('truncates huge schemas and says how many tables were left out', () => {
      const s = big(2000)
      const out = buildSchemaContext(s)
      const lines = out.split('\n')
      const note = lines[lines.length - 1]
      const listed = lines.length - 1
      expect(note).toBe(`-- ${2000 - listed} more tables not listed (schema too large)`)
      expect(listed).toBeGreaterThan(0)
      expect(listed).toBeLessThan(2000)
    })

    it('the listed part stays within the cap', () => {
      const out = buildSchemaContext(big(2000))
      const lines = out.split('\n')
      const listed = lines.slice(0, -1).join('\n')
      expect(listed.length + 1).toBeLessThanOrEqual(CAP)
    })

    it('never cuts a table line in half', () => {
      const out = buildSchemaContext(big(2000))
      for (const line of out.split('\n').slice(0, -1)) expect(line).toMatch(/^public\.table_number_\d+\(column_0, .*column_19\)$/)
    })

    it('keeps the first tables, not a random subset', () => {
      const lines = buildSchemaContext(big(2000)).split('\n')
      expect(lines[0].startsWith('public.table_number_0(')).toBe(true)
      expect(lines[1].startsWith('public.table_number_1(')).toBe(true)
    })

    it('does not add a note when everything fits', () => {
      const out = buildSchemaContext(big(10))
      expect(out).not.toContain('more tables')
      expect(out.split('\n')).toHaveLength(10)
    })

    it('counts omitted tables among the usable ones only (sequences are not "left out")', () => {
      const s = big(1500)
      s.schemas.public.push(table('seq_a', 'public', 'sequence'), table('seq_b', 'public', 'sequence'))
      const lines = buildSchemaContext(s).split('\n')
      const listed = lines.length - 1
      expect(lines[lines.length - 1]).toBe(`-- ${1500 - listed} more tables not listed (schema too large)`)
    })

    it('a single enormous table line yields only the note', () => {
      const cols = Array.from({ length: 5000 }, (_, i) => `a_very_long_column_name_${i}`)
      const out = buildSchemaContext(snap([table('wide'), table('other')], { 'public.wide': cols }))
      expect(out).toBe('-- 2 more tables not listed (schema too large)')
    })

    it('stops at the first table that does not fit, even if a later one is short', () => {
      const cols = Array.from({ length: 5000 }, (_, i) => `a_very_long_column_name_${i}`)
      const out = buildSchemaContext(snap([table('ok'), table('wide'), table('tiny')], { 'public.wide': cols }))
      expect(out).toBe('public.ok\n-- 2 more tables not listed (schema too large)')
    })
  })
})

describe('firstTableName', () => {
  let firstTableName: Awaited<ReturnType<typeof pureFns>>['firstTableName']
  beforeEach(async () => {
    firstTableName = (await pureFns()).firstTableName
  })

  it('is null without a snapshot or tables', () => {
    expect(firstTableName(null)).toBeNull()
    expect(firstTableName(snap([]))).toBeNull()
  })

  it('returns the bare name for the public schema', () => {
    expect(firstTableName(snap([table('users', 'public')]))).toBe('users')
  })

  it('prefers a table in the public schema over earlier ones', () => {
    expect(firstTableName(snap([table('audit_log', 'audit'), table('users', 'public')]))).toBe('users')
  })

  it('qualifies tables outside public', () => {
    expect(firstTableName(snap([table('events', 'analytics'), table('clicks', 'analytics')]))).toBe('analytics.events')
  })

  it('returns the bare name for schema-less engines', () => {
    expect(firstTableName(snap([table('notes', null)]))).toBe('notes')
  })

  it('ignores views, sequences and system tables', () => {
    const s = snap([table('v', 'public', 'view'), table('seq', 'public', 'sequence'), table('sys', 'public', 'system'), table('real', 'public')])
    expect(firstTableName(s)).toBe('real')
  })

  it('is null when only non-table objects exist', () => {
    expect(firstTableName(snap([table('v', 'public', 'view'), table('mv', 'public', 'materializedView')]))).toBeNull()
  })

  it('accepts collections (MongoDB)', () => {
    expect(firstTableName(snap([table('events', null, 'collection')]))).toBe('events')
  })
})

// ---------------------------------------------------------------------------------------------

function session(id = 's1', engine: 'postgres' | 'sqlite' = 'postgres'): SessionInfo {
  return { sessionId: id, serverVersion: '16.2', databases: ['app'], config: { ...newConfig(engine), id: `c-${id}`, database: 'app' } }
}

interface Ctx {
  assistant: (typeof import('./assistant.svelte'))['assistant']
  mod: typeof import('./assistant.svelte')
  calls: { command: string; args: Record<string, unknown> }[]
  asks: AssistantRequest[]
}

async function setup(handlers: InvokeHandlers = {}, storage: Record<string, unknown> = {}): Promise<Ctx> {
  vi.resetModules()
  installStorage(storage)
  const calls: Ctx['calls'] = []
  const asks: AssistantRequest[] = []
  const routed = routeInvoke({
    assistant_key_status: () => true,
    assistant_cancel: () => undefined,
    assistant_key_set: () => undefined,
    assistant_ask: () => ({ text: `reply ${asks.length}` }),
    ...handlers,
  })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args })
    if (command === 'assistant_ask') asks.push(args.req as AssistantRequest)
    return routed(command, args)
  })
  const mod = await import('./assistant.svelte')
  return { assistant: mod.assistant, mod, calls, asks }
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('assistant: model', () => {
  it('defaults to Sonnet 5.5 and lists three models', async () => {
    const { assistant, mod } = await setup()
    expect(assistant.model).toBe('claude-sonnet-5-5')
    expect(mod.DEFAULT_MODEL).toBe('claude-sonnet-5-5')
    expect(mod.MODELS.map((m) => m.id)).toContain(mod.DEFAULT_MODEL)
    expect(new Set(mod.MODELS.map((m) => m.id)).size).toBe(mod.MODELS.length)
  })

  it('loads the saved model at startup', async () => {
    const { assistant } = await setup({}, { 'tablesoft.assistant.model': 'claude-opus-5-5' })
    expect(assistant.model).toBe('claude-opus-5-5')
  })

  it('setModel updates and persists', async () => {
    const { assistant } = await setup()
    assistant.setModel('claude-haiku-5-5')
    expect(assistant.model).toBe('claude-haiku-5-5')
    expect(localStorage.getItem('tablesoft.assistant.model')).toBe('claude-haiku-5-5')
  })

  it('works without localStorage', async () => {
    const { assistant } = await setup()
    vi.stubGlobal('localStorage', undefined)
    expect(() => assistant.setModel('claude-opus-5-5')).not.toThrow()
    expect(assistant.model).toBe('claude-opus-5-5')
  })

  it('the model chosen is the one sent with the request', async () => {
    const { assistant, asks } = await setup()
    assistant.setModel('claude-opus-5-5')
    assistant.send(session(), 'app', 'ctx', 'hello')
    await settle()
    expect(asks[0].model).toBe('claude-opus-5-5')
  })
})

describe('assistant: key', () => {
  it('hasKey starts unknown', async () => {
    const { assistant } = await setup()
    expect(assistant.hasKey).toBeNull()
  })

  it('refreshKey reads the backend flag', async () => {
    let has = true
    const { assistant } = await setup({ assistant_key_status: () => has })
    await assistant.refreshKey()
    expect(assistant.hasKey).toBe(true)
    has = false
    await assistant.refreshKey()
    expect(assistant.hasKey).toBe(false)
  })

  it('a failing key check means "no key"', async () => {
    const { assistant } = await setup({
      assistant_key_status: () => {
        throw new Error('keychain locked')
      },
    })
    await assistant.refreshKey()
    expect(assistant.hasKey).toBe(false)
  })

  it('setKey stores it and then re-reads the status', async () => {
    const { assistant, calls } = await setup({ assistant_key_status: () => true })
    await assistant.setKey('sk-test')
    expect(calls.map((c) => c.command)).toEqual(['assistant_key_set', 'assistant_key_status'])
    expect(calls[0].args).toEqual({ key: 'sk-test' })
    expect(assistant.hasKey).toBe(true)
  })

  it('setKey throws when the keychain refuses, without touching the flag', async () => {
    const { assistant } = await setup({
      assistant_key_set: () => {
        throw coreError('io', 'keychain error')
      },
    })
    await expect(assistant.setKey('x')).rejects.toMatchObject({ message: 'keychain error' })
    expect(assistant.hasKey).toBeNull()
  })

  it('an empty key is passed through (removes it)', async () => {
    const { assistant, calls } = await setup({ assistant_key_status: () => false })
    await assistant.setKey('')
    expect(calls[0].args).toEqual({ key: '' })
    expect(assistant.hasKey).toBe(false)
  })
})

describe('assistant: drafts and empty chats', () => {
  it('an unknown session has an empty chat and draft', async () => {
    const { assistant } = await setup()
    expect(assistant.chat('nope')).toEqual({ messages: [], requestId: null, error: null })
    expect(assistant.draft('nope')).toBe('')
  })

  it('drafts are kept per session', async () => {
    const { assistant } = await setup()
    assistant.setDraft('a', 'half-typed')
    assistant.setDraft('b', 'other')
    expect(assistant.draft('a')).toBe('half-typed')
    expect(assistant.draft('b')).toBe('other')
  })
})

describe('assistant.send', () => {
  it('adds the question, asks the backend and appends the reply', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', 'public.users(id)', '  show me users  ')
    expect(assistant.chat('s1').messages.map((m) => [m.role, m.content])).toEqual([['user', 'show me users']])
    expect(assistant.chat('s1').requestId).toBeTruthy()
    await settle()
    expect(assistant.chat('s1').messages.map((m) => [m.role, m.content])).toEqual([
      ['user', 'show me users'],
      ['assistant', 'reply 1'],
    ])
    expect(assistant.chat('s1').requestId).toBeNull()
    expect(assistant.chat('s1').error).toBeNull()
    expect(asks).toHaveLength(1)
  })

  it('sends engine, version, database, schema context and the whole conversation', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session('s1', 'sqlite'), 'main', 'notes(id)', 'first')
    await settle()
    assistant.send(session('s1', 'sqlite'), 'main', 'notes(id)', 'second')
    await settle()
    expect(asks[1]).toMatchObject({ engine: 'sqlite', serverVersion: '16.2', database: 'main', schemaContext: 'notes(id)' })
    expect(asks[1].messages).toEqual([
      { role: 'user', content: 'first' },
      { role: 'assistant', content: 'reply 1' },
      { role: 'user', content: 'second' },
    ])
  })

  it('strips local ids from the messages sent to the backend', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', '', 'hi')
    await settle()
    for (const m of asks[0].messages) expect(Object.keys(m).sort()).toEqual(['content', 'role'])
  })

  it('sends a request id that cancel can target', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', '', 'hi')
    await settle()
    expect(asks[0].requestId).toMatch(/\S+/)
  })

  it('gives every message a unique increasing id', async () => {
    const { assistant } = await setup()
    assistant.send(session(), 'app', '', 'a')
    await settle()
    assistant.send(session(), 'app', '', 'b')
    await settle()
    const ids = assistant.chat('s1').messages.map((m) => m.id)
    expect(new Set(ids).size).toBe(4)
    expect([...ids].sort((x, y) => x - y)).toEqual(ids)
  })

  it('clears the draft', async () => {
    const { assistant } = await setup()
    assistant.setDraft('s1', 'typed')
    assistant.send(session(), 'app', '', 'typed')
    expect(assistant.draft('s1')).toBe('')
  })

  it('ignores empty and whitespace-only questions', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', '', '')
    assistant.send(session(), 'app', '', '   \n\t ')
    await settle()
    expect(asks).toEqual([])
    expect(assistant.chat('s1').messages).toEqual([])
  })

  it('ignores a second question while a reply is pending', async () => {
    let release: (() => void) | undefined
    const { assistant } = await setup({
      assistant_ask: () => new Promise((res) => (release = () => res({ text: 'late' }))),
    })
    assistant.send(session(), 'app', '', 'first')
    assistant.send(session(), 'app', '', 'second')
    expect(assistant.chat('s1').messages.map((m) => m.content)).toEqual(['first'])
    release?.()
    await settle()
    expect(assistant.chat('s1').messages.map((m) => m.content)).toEqual(['first', 'late'])
  })

  it('keeps conversations of different sessions apart', async () => {
    const { assistant } = await setup()
    assistant.send(session('a'), 'app', '', 'for a')
    assistant.send(session('b'), 'app', '', 'for b')
    await settle()
    expect(assistant.chat('a').messages[0].content).toBe('for a')
    expect(assistant.chat('b').messages[0].content).toBe('for b')
    expect(assistant.chat('a').messages).toHaveLength(2)
  })

  describe('failures', () => {
    it('shows the error and keeps the question', async () => {
      const { assistant } = await setup({
        assistant_ask: () => {
          throw coreError('other', 'rate limited')
        },
      })
      assistant.send(session(), 'app', '', 'hi')
      await settle()
      const chat = assistant.chat('s1')
      expect(chat.error).toBe('rate limited')
      expect(chat.requestId).toBeNull()
      expect(chat.messages.map((m) => m.role)).toEqual(['user'])
    })

    it('re-checks the key after an error (a missing key switches to the "add key" state)', async () => {
      let has = true
      const { assistant } = await setup({
        assistant_key_status: () => has,
        assistant_ask: () => {
          has = false
          throw coreError('invalidInput', 'no API key')
        },
      })
      await assistant.refreshKey()
      expect(assistant.hasKey).toBe(true)
      assistant.send(session(), 'app', '', 'hi')
      await settle(10)
      expect(assistant.hasKey).toBe(false)
    })

    it('a cancelled error is not shown', async () => {
      const { assistant } = await setup({
        assistant_ask: () => {
          throw coreError('cancelled', 'cancelled')
        },
      })
      assistant.send(session(), 'app', '', 'hi')
      await settle()
      expect(assistant.chat('s1').error).toBeNull()
      expect(assistant.chat('s1').requestId).toBeNull()
    })

    it('plain errors are shown too', async () => {
      const { assistant } = await setup({
        assistant_ask: () => {
          throw new Error('socket hang up')
        },
      })
      assistant.send(session(), 'app', '', 'hi')
      await settle()
      expect(assistant.chat('s1').error).toBe('socket hang up')
    })

    it('a new question clears the previous error', async () => {
      let fail = true
      const { assistant } = await setup({
        assistant_ask: () => {
          if (fail) throw new Error('boom')
          return { text: 'ok' }
        },
      })
      assistant.send(session(), 'app', '', 'one')
      await settle()
      expect(assistant.chat('s1').error).toBe('boom')
      fail = false
      assistant.send(session(), 'app', '', 'two')
      expect(assistant.chat('s1').error).toBeNull()
      await settle()
      expect(assistant.chat('s1').messages.at(-1)?.content).toBe('ok')
    })
  })
})

describe('assistant.retry', () => {
  async function failedOnce() {
    let fail = true
    const ctx = await setup({
      assistant_ask: () => {
        if (fail) throw new Error('boom')
        return { text: 'second try' }
      },
    })
    ctx.assistant.send(session(), 'app', 'ctx', 'question')
    await settle()
    return { ...ctx, succeedNext: () => (fail = false) }
  }

  it('asks again without duplicating the question', async () => {
    const { assistant, asks, succeedNext } = await failedOnce()
    expect(assistant.chat('s1').error).toBe('boom')
    succeedNext()
    assistant.retry(session(), 'app', 'ctx')
    expect(assistant.chat('s1').error).toBeNull()
    await settle()
    expect(assistant.chat('s1').messages.map((m) => [m.role, m.content])).toEqual([
      ['user', 'question'],
      ['assistant', 'second try'],
    ])
    expect(asks).toHaveLength(2)
    expect(asks[1].messages).toEqual([{ role: 'user', content: 'question' }])
  })

  it('does nothing when there is no conversation', async () => {
    const { assistant, asks } = await setup()
    assistant.retry(session(), 'app', '')
    await settle()
    expect(asks).toEqual([])
  })

  it('does nothing when the last message is already an answer', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', '', 'q')
    await settle()
    assistant.retry(session(), 'app', '')
    await settle()
    expect(asks).toHaveLength(1)
  })

  it('does nothing while a reply is pending', async () => {
    let release: (() => void) | undefined
    const { assistant, asks } = await setup({
      assistant_ask: () => new Promise((res) => (release = () => res({ text: 'x' }))),
    })
    assistant.send(session(), 'app', '', 'q')
    assistant.retry(session(), 'app', '')
    expect(asks).toHaveLength(1)
    release?.()
    await settle()
  })
})

describe('assistant.cancel / clear', () => {
  /** An `assistant_ask` handler whose replies the test settles by hand, in request order. */
  function pending() {
    const resolvers: ((text: string) => void)[] = []
    const rejecters: ((e: unknown) => void)[] = []
    return {
      resolvers,
      rejecters,
      handler: () =>
        new Promise((res, rej) => {
          resolvers.push((text) => res({ text }))
          rejecters.push(rej)
        }),
    }
  }

  it('stops waiting, tells the backend and keeps the question', async () => {
    const p = pending()
    const { assistant, calls } = await setup({ assistant_ask: p.handler })
    assistant.send(session(), 'app', '', 'long one')
    const id = assistant.chat('s1').requestId
    assistant.cancel('s1')
    expect(assistant.chat('s1').requestId).toBeNull()
    expect(assistant.chat('s1').messages.map((m) => m.content)).toEqual(['long one'])
    await settle()
    expect(calls.find((c) => c.command === 'assistant_cancel')?.args).toEqual({ requestId: id })
  })

  it('a reply that arrives after cancel is dropped', async () => {
    const p = pending()
    const { assistant } = await setup({ assistant_ask: p.handler })
    assistant.send(session(), 'app', '', 'q')
    assistant.cancel('s1')
    p.resolvers[0]('too late')
    await settle()
    expect(assistant.chat('s1').messages.map((m) => m.role)).toEqual(['user'])
  })

  it('an error that arrives after cancel is dropped', async () => {
    const p = pending()
    const { assistant } = await setup({ assistant_ask: p.handler })
    assistant.send(session(), 'app', '', 'q')
    assistant.cancel('s1')
    p.rejecters[0](new Error('late failure'))
    await settle()
    expect(assistant.chat('s1').error).toBeNull()
  })

  it('a stale reply does not land in a newer request', async () => {
    const p = pending()
    const { assistant } = await setup({ assistant_ask: p.handler })
    assistant.send(session(), 'app', '', 'first')
    assistant.cancel('s1')
    assistant.send(session(), 'app', '', 'second')
    expect(assistant.chat('s1').requestId).toBeTruthy()
    p.resolvers[0]('answer to first')
    await settle()
    expect(assistant.chat('s1').messages.map((m) => m.role)).toEqual(['user', 'user'])
    expect(assistant.chat('s1').requestId).toBeTruthy() // second still pending
    p.resolvers[1]('answer to second')
    await settle()
    expect(assistant.chat('s1').messages.at(-1)).toMatchObject({ role: 'assistant', content: 'answer to second' })
    expect(assistant.chat('s1').requestId).toBeNull()
  })

  it('cancel without a pending request does nothing', async () => {
    const { assistant, calls } = await setup()
    assistant.cancel('s1')
    assistant.cancel('never-seen')
    await settle()
    expect(calls.some((c) => c.command === 'assistant_cancel')).toBe(false)
  })

  it('a failing cancel call is swallowed', async () => {
    const p = pending()
    const { assistant } = await setup({
      assistant_ask: p.handler,
      assistant_cancel: () => {
        throw new Error('backend gone')
      },
    })
    assistant.send(session(), 'app', '', 'q')
    expect(() => assistant.cancel('s1')).not.toThrow()
    await settle()
  })

  it('clear forgets the conversation and cancels a pending reply', async () => {
    const p = pending()
    const { assistant, calls } = await setup({ assistant_ask: p.handler })
    assistant.send(session(), 'app', '', 'q')
    assistant.clear('s1')
    expect(assistant.chat('s1').messages).toEqual([])
    await settle()
    expect(calls.some((c) => c.command === 'assistant_cancel')).toBe(true)
    p.resolvers[0]('ghost reply')
    await settle()
    expect(assistant.chat('s1').messages).toEqual([])
  })

  it('clear on an idle chat just empties it', async () => {
    const { assistant, calls } = await setup()
    assistant.send(session(), 'app', '', 'q')
    await settle()
    assistant.clear('s1')
    expect(assistant.chat('s1').messages).toEqual([])
    expect(calls.some((c) => c.command === 'assistant_cancel')).toBe(false)
  })

  it('clear on an unknown session is harmless', async () => {
    const { assistant } = await setup()
    expect(() => assistant.clear('nope')).not.toThrow()
  })

  it('after clear the next question starts a fresh conversation', async () => {
    const { assistant, asks } = await setup()
    assistant.send(session(), 'app', '', 'old')
    await settle()
    assistant.clear('s1')
    assistant.send(session(), 'app', '', 'new')
    await settle()
    expect(asks.at(-1)?.messages).toEqual([{ role: 'user', content: 'new' }])
  })
})
