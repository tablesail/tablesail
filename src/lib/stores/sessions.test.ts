import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage, routeInvoke, settle, type InvokeHandlers } from '../../test/helpers'
import type { CoreError, SchemaSnapshot, SessionInfo } from '../types'
import { newConfig } from '../util/env'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

const coreError = (kind: CoreError['kind'], message: string): CoreError => ({ kind, message, code: null, statement: null })

function snapshot(db: string, tables: string[] = ['users']): SchemaSnapshot {
  return {
    database: db,
    schemas: { public: tables.map((name) => ({ database: db, schema: 'public', name, kind: 'table', estimatedRows: null, comment: null })) },
    columns: Object.fromEntries(tables.map((t) => [`public.${t}`, ['id']])),
    fetchedAt: 1,
  }
}

function info(sessionId: string, over: { databases?: string[]; database?: string; connectionId?: string } = {}): SessionInfo {
  return {
    sessionId,
    serverVersion: '16.0',
    databases: over.databases ?? ['app', 'other'],
    config: { ...newConfig('postgres'), id: over.connectionId ?? `conn-${sessionId}`, name: sessionId, database: over.database ?? 'app' },
  }
}

interface Ctx {
  sessions: (typeof import('./sessions.svelte'))['sessions']
  isAuthError: (typeof import('./sessions.svelte'))['isAuthError']
  tabs: (typeof import('./tabs.svelte'))['tabs']
  calls: { command: string; args: Record<string, unknown> }[]
}

async function setup(handlers: InvokeHandlers = {}, storage: Record<string, unknown> = {}): Promise<Ctx> {
  vi.resetModules()
  installStorage(storage)
  const calls: Ctx['calls'] = []
  const routed = routeInvoke({
    connection_close: () => undefined,
    schema_snapshot: ({ database }) => snapshot(String(database)),
    ...handlers,
  })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args })
    return routed(command, args)
  })
  const mod = await import('./sessions.svelte')
  const tabs = (await import('./tabs.svelte')).tabs
  return { sessions: mod.sessions, isAuthError: mod.isAuthError, tabs, calls }
}

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('isAuthError', () => {
  let isAuthError: Ctx['isAuthError']
  beforeEach(async () => {
    isAuthError = (await setup()).isAuthError
  })

  it.each([
    ['database', 'password authentication failed for user "bob"'],
    ['database', 'Access denied for user \'root\'@\'localhost\' (using password: YES)'],
    ['connection', 'authentication failed'],
    ['connection', 'FATAL: password authentication failed'],
    ['ssh', 'Permission denied (publickey,password)'],
    ['ssh', 'bad passphrase for key'],
    ['other', 'invalid credentials'],
  ] as const)('is true for a %s error: %s', (kind, message) => {
    expect(isAuthError(coreError(kind, message))).toBe(true)
  })

  it.each([
    ['connection', 'connection refused'],
    ['connection', 'timed out'],
    ['database', 'relation "x" does not exist'],
    ['ssh', 'host key verification failed'],
    ['unsupported', 'engine not built in'],
    ['notFound', 'no such connection'],
  ] as const)('is false for a %s error: %s', (kind, message) => {
    expect(isAuthError(coreError(kind, message))).toBe(false)
  })

  it('other error kinds only match the narrower pattern', () => {
    expect(isAuthError(coreError('io', 'permission denied'))).toBe(false) // a file permission problem
    expect(isAuthError(coreError('io', 'wrong password'))).toBe(true)
    expect(isAuthError(coreError('cancelled', 'cancelled'))).toBe(false)
  })

  it('works for plain errors and strings', () => {
    expect(isAuthError(new Error('Authentication failed'))).toBe(true)
    expect(isAuthError(new Error('network unreachable'))).toBe(false)
    expect(isAuthError('ACCESS DENIED')).toBe(true)
    expect(isAuthError('nope')).toBe(false)
  })

  it('is case-insensitive', () => {
    expect(isAuthError(coreError('database', 'PASSWORD AUTHENTICATION FAILED'))).toBe(true)
  })

  it('handles non-error values', () => {
    expect(isAuthError(null)).toBe(false)
    expect(isAuthError(undefined)).toBe(false)
    expect(isAuthError(42)).toBe(false)
  })
})

describe('sessions: initial state', () => {
  it('starts on the welcome screen with nothing open', async () => {
    const { sessions } = await setup()
    expect(sessions.list).toEqual([])
    expect(sessions.activeId).toBeNull()
    expect(sessions.active).toBeNull()
    expect(sessions.view).toBe('connections')
  })

  it('schemaEntry is empty for no session or an unknown one', async () => {
    const { sessions } = await setup()
    expect(sessions.schemaEntry(null, 'db')).toEqual({ snapshot: null, loading: false, error: null })
    expect(sessions.schemaEntry('ghost', 'db')).toEqual({ snapshot: null, loading: false, error: null })
  })

  it('database() is empty for an unknown session', async () => {
    const { sessions } = await setup()
    expect(sessions.database('ghost')).toBe('')
  })
})

describe('sessions.open', () => {
  it('opens through the backend with the connection id and secrets', async () => {
    const { sessions, calls } = await setup({ connection_open: () => info('s1') })
    const secrets = { dbPassword: 'pw' }
    const result = await sessions.open('conn-1', secrets)
    expect(calls.find((c) => c.command === 'connection_open')?.args).toEqual({ id: 'conn-1', secrets })
    expect(result.sessionId).toBe('s1')
  })

  it('defaults secrets to null', async () => {
    const { sessions, calls } = await setup({ connection_open: () => info('s1') })
    await sessions.open('conn-1')
    expect(calls.find((c) => c.command === 'connection_open')?.args).toEqual({ id: 'conn-1', secrets: null })
  })

  it('adds the session, activates it and switches to the workspace', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1') })
    await sessions.open('conn-s1')
    expect(sessions.list.map((s) => s.sessionId)).toEqual(['s1'])
    expect(sessions.activeId).toBe('s1')
    expect(sessions.active?.sessionId).toBe('s1')
    expect(sessions.view).toBe('workspace')
  })

  it('the newest opened session becomes active, earlier ones stay open', async () => {
    let n = 0
    const { sessions } = await setup({ connection_open: () => info(`s${++n}`) })
    await sessions.open('a')
    await sessions.open('b')
    expect(sessions.list.map((s) => s.sessionId)).toEqual(['s1', 's2'])
    expect(sessions.activeId).toBe('s2')
  })

  it('starts on the configured database when the server lists it', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1', { databases: ['a', 'b', 'c'], database: 'b' }) })
    await sessions.open('x')
    expect(sessions.database('s1')).toBe('b')
  })

  it('falls back to the first listed database when the configured one is missing', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1', { databases: ['a', 'b'], database: 'gone' }) })
    await sessions.open('x')
    expect(sessions.database('s1')).toBe('a')
  })

  it('uses the configured database when the server lists none', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1', { databases: [], database: 'main' }) })
    await sessions.open('x')
    expect(sessions.database('s1')).toBe('main')
  })

  it('re-opening the same session replaces it (no duplicates) and keeps the chosen database', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1', { databases: ['app', 'other'] }) })
    await sessions.open('x')
    sessions.setDatabase('s1', 'other')
    await sessions.open('x')
    expect(sessions.list).toHaveLength(1)
    expect(sessions.database('s1')).toBe('other')
  })

  it('propagates a failed open and leaves the state untouched', async () => {
    const { sessions } = await setup({
      connection_open: () => {
        throw coreError('connection', 'refused')
      },
    })
    await expect(sessions.open('x')).rejects.toMatchObject({ message: 'refused' })
    expect(sessions.list).toEqual([])
    expect(sessions.activeId).toBeNull()
    expect(sessions.view).toBe('connections')
  })

  it('reopens the tabs saved for this connection', async () => {
    const saved = { tabs: [{ kind: 'table', table: { database: 'app', name: 'users' }, title: 'users' }], active: 0 }
    const { sessions, tabs } = await setup(
      { connection_open: () => info('s1', { connectionId: 'conn-1' }) },
      { 'tablesail.tabs.conn-1': saved },
    )
    await sessions.open('conn-1')
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['users'])
  })

  it('loads the schema for the starting database', async () => {
    const { sessions, calls } = await setup({ connection_open: () => info('s1') })
    await sessions.open('x')
    await settle(20)
    const snaps = calls.filter((c) => c.command === 'schema_snapshot')
    expect(snaps.map((c) => c.args)).toEqual([
      { sessionId: 's1', database: 'app', refresh: false },
      { sessionId: 's1', database: 'app', refresh: true },
    ])
    expect(sessions.schemaEntry('s1', 'app').snapshot?.database).toBe('app')
    expect(sessions.schemaEntry('s1', 'app').loading).toBe(false)
  })
})

describe('sessions.activate / byConnection', () => {
  it('activate switches the active session and shows the workspace', async () => {
    let n = 0
    const { sessions } = await setup({ connection_open: () => info(`s${++n}`) })
    await sessions.open('a')
    await sessions.open('b')
    sessions.view = 'connections'
    sessions.activate('s1')
    expect(sessions.activeId).toBe('s1')
    expect(sessions.view).toBe('workspace')
  })

  it('byConnection finds the session opened from a saved connection', async () => {
    let n = 0
    const { sessions } = await setup({ connection_open: () => info(`s${++n}`, { connectionId: `c${n}` }) })
    await sessions.open('c1')
    await sessions.open('c2')
    expect(sessions.byConnection('c2')?.sessionId).toBe('s2')
    expect(sessions.byConnection('nope')).toBeUndefined()
  })
})

describe('sessions.close', () => {
  async function two() {
    let n = 0
    const ctx = await setup({ connection_open: () => info(`s${++n}`) })
    await ctx.sessions.open('a')
    await ctx.sessions.open('b')
    return ctx
  }

  it('closes on the backend and removes the session', async () => {
    const { sessions, calls } = await two()
    await sessions.close('s2')
    expect(calls.some((c) => c.command === 'connection_close' && c.args.sessionId === 's2')).toBe(true)
    expect(sessions.list.map((s) => s.sessionId)).toEqual(['s1'])
    expect(sessions.database('s2')).toBe('')
  })

  it('closing the active session activates the first remaining one', async () => {
    const { sessions } = await two()
    await sessions.close('s2')
    expect(sessions.activeId).toBe('s1')
    expect(sessions.view).toBe('workspace')
  })

  it('closing the last session returns to the welcome screen', async () => {
    const { sessions } = await two()
    await sessions.close('s2')
    await sessions.close('s1')
    expect(sessions.activeId).toBeNull()
    expect(sessions.view).toBe('connections')
  })

  it('closing an inactive session keeps the active one', async () => {
    const { sessions } = await two()
    await sessions.close('s1')
    expect(sessions.activeId).toBe('s2')
  })

  it('drops the session\'s tabs', async () => {
    const { sessions, tabs } = await two()
    tabs.newQuery('s1', 'app')
    tabs.newQuery('s2', 'app')
    await sessions.close('s2')
    expect(tabs.forSession('s2')).toEqual([])
    expect(tabs.forSession('s1')).toHaveLength(1)
  })

  it('still closes the session in the UI when the backend call fails', async () => {
    let n = 0
    const { sessions } = await setup({
      connection_open: () => info(`s${++n}`),
      connection_close: () => {
        throw new Error('already gone')
      },
    })
    await sessions.open('a')
    await expect(sessions.close('s1')).resolves.toBeUndefined()
    expect(sessions.list).toEqual([])
  })

  it('closing an unknown session is harmless', async () => {
    const { sessions } = await two()
    await expect(sessions.close('ghost')).resolves.toBeUndefined()
    expect(sessions.list).toHaveLength(2)
  })

  it('remembers the tabs so reconnecting restores them', async () => {
    let n = 0
    const { sessions, tabs } = await setup({ connection_open: () => info(`s${++n}`, { connectionId: 'same' }) })
    await sessions.open('same')
    tabs.newQuery('s1', 'app', 'SELECT 1;')
    await sessions.close('s1')
    await sessions.open('same')
    expect(tabs.forSession('s2').map((t) => (t.kind === 'query' ? t.sql : ''))).toEqual(['SELECT 1;'])
  })
})

describe('sessions.setDatabase', () => {
  it('switches the database and loads its schema', async () => {
    const { sessions, calls } = await setup({ connection_open: () => info('s1') })
    await sessions.open('x')
    await settle(20)
    calls.length = 0
    sessions.setDatabase('s1', 'other')
    await settle(20)
    expect(sessions.database('s1')).toBe('other')
    expect(calls.filter((c) => c.command === 'schema_snapshot').map((c) => c.args.database)).toEqual(['other', 'other'])
    expect(sessions.schemaEntry('s1', 'other').snapshot?.database).toBe('other')
  })

  it('does not reload a schema that is already cached', async () => {
    const { sessions, calls } = await setup({ connection_open: () => info('s1') })
    await sessions.open('x')
    await settle(20)
    calls.length = 0
    sessions.setDatabase('s1', 'app')
    await settle(20)
    expect(calls.filter((c) => c.command === 'schema_snapshot')).toEqual([])
  })

  it('keeps schemas of different databases apart', async () => {
    const { sessions } = await setup({ connection_open: () => info('s1') })
    await sessions.open('x')
    sessions.setDatabase('s1', 'other')
    await settle(20)
    expect(sessions.schemaEntry('s1', 'app').snapshot?.database).toBe('app')
    expect(sessions.schemaEntry('s1', 'other').snapshot?.database).toBe('other')
  })
})

describe('sessions.ensureSchema / refreshSchema', () => {
  it('shows the cached snapshot first, then the fresh one', async () => {
    const seen: { loading: boolean; tables: string[] }[] = []
    let sessionsRef: Ctx['sessions'] | undefined
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database, refresh }) => {
        // record what the sidebar would see while each request is in flight
        const e = sessionsRef!.schemaEntry('s1', String(database))
        seen.push({ loading: e.loading, tables: e.snapshot ? Object.values(e.snapshot.schemas).flat().map((t) => t.name) : [] })
        return snapshot(String(database), refresh ? ['users', 'fresh_table'] : ['users'])
      },
    })
    sessionsRef = sessions
    await sessions.open('x')
    await settle(20)
    expect(seen[0]).toEqual({ loading: true, tables: [] }) // cache request: nothing yet
    expect(seen[1]).toEqual({ loading: true, tables: ['users'] }) // refresh: cached shown while loading
    const final = sessions.schemaEntry('s1', 'app')
    expect(final.loading).toBe(false)
    expect(Object.values(final.snapshot!.schemas).flat().map((t) => t.name)).toEqual(['users', 'fresh_table'])
  })

  it('a missing cache is not an error; the refresh still happens', async () => {
    let first = true
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database }) => {
        if (first) {
          first = false
          throw coreError('notFound', 'no cache')
        }
        return snapshot(String(database))
      },
    })
    await sessions.open('x')
    await settle(20)
    const e = sessions.schemaEntry('s1', 'app')
    expect(e.error).toBeNull()
    expect(e.snapshot).not.toBeNull()
    expect(e.loading).toBe(false)
  })

  it('reports a refresh error and keeps the old snapshot', async () => {
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database, refresh }) => {
        if (refresh) throw coreError('database', 'permission denied for schema')
        return snapshot(String(database))
      },
    })
    await sessions.open('x')
    await settle(20)
    const e = sessions.schemaEntry('s1', 'app')
    expect(e.error).toBe('permission denied for schema')
    expect(e.loading).toBe(false)
    expect(e.snapshot?.database).toBe('app')
  })

  it('a later successful refresh clears the error', async () => {
    let fail = true
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database, refresh }) => {
        if (refresh && fail) throw new Error('flaky')
        return snapshot(String(database))
      },
    })
    await sessions.open('x')
    await settle(20)
    expect(sessions.schemaEntry('s1', 'app').error).toBe('flaky')
    fail = false
    await sessions.refreshSchema('s1', 'app')
    expect(sessions.schemaEntry('s1', 'app').error).toBeNull()
  })

  it('ignores ensureSchema while a load is already running', async () => {
    let release: (() => void) | undefined
    const { sessions, calls } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database }) => new Promise((res) => (release = () => res(snapshot(String(database))))),
    })
    await sessions.open('x')
    await settle(5)
    const before = calls.filter((c) => c.command === 'schema_snapshot').length
    void sessions.ensureSchema('s1', 'app')
    await settle(5)
    expect(calls.filter((c) => c.command === 'schema_snapshot').length).toBe(before)
    release?.()
  })

  it('discards a result that arrives after the session was closed', async () => {
    let release: ((s: SchemaSnapshot) => void) | undefined
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: () => new Promise((res) => (release = res as (s: SchemaSnapshot) => void)),
    })
    await sessions.open('x')
    await settle(5)
    await sessions.close('s1')
    release?.(snapshot('app'))
    await settle(20)
    expect(sessions.schemaEntry('s1', 'app').snapshot).toBeNull()
  })

  it('refreshSchema keeps the current snapshot visible while loading', async () => {
    let hold = false
    let release: (() => void) | undefined
    const { sessions } = await setup({
      connection_open: () => info('s1'),
      schema_snapshot: ({ database }) => (hold ? new Promise((res) => (release = () => res(snapshot(String(database), ['t'])))) : snapshot(String(database))),
    })
    await sessions.open('x')
    await settle(20)
    hold = true
    const p = sessions.refreshSchema('s1', 'app')
    await settle(5)
    const mid = sessions.schemaEntry('s1', 'app')
    expect(mid.loading).toBe(true)
    expect(mid.snapshot).not.toBeNull()
    release?.()
    await p
    expect(sessions.schemaEntry('s1', 'app').loading).toBe(false)
  })
})
