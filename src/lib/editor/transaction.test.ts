import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage, routeInvoke, type InvokeHandlers } from '../../test/helpers'
import type { CoreError, QueryStatus, StatementState, StatementStatus } from '../types'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

const coreError = (kind: CoreError['kind'], message: string): CoreError => ({ kind, message, code: null, statement: null })

async function setup(handlers: InvokeHandlers = {}) {
  vi.resetModules()
  installStorage()
  const calls: { command: string; args: Record<string, unknown> }[] = []
  const routed = routeInvoke({ tx_begin: () => 'tx1', tx_commit: () => null, tx_rollback: () => null, ...handlers })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args })
    return routed(command, args)
  })
  const { tabs } = await import('../stores/tabs.svelte')
  const { ui } = await import('../stores/ui.svelte')
  const tx = await import('./transaction')
  const { editorRequests } = await import('./requests.svelte')
  const open = () => {
    const made = tabs.newQuery('s1', 'app', 'UPDATE t SET a = 1')
    // the store's reactive copy is what the app works with
    return tabs.list.find((t) => t.id === made.id) as import('../stores/tabs.svelte').QueryTab
  }
  const toasts = () => ui.toasts.map((t) => `${t.kind}: ${t.text}`)
  return { tabs, ui, tx, editorRequests, calls, open, toasts }
}

// The first import compiles the svelte runtime and the stores, which is slow on a busy machine.
beforeAll(async () => {
  await setup()
}, 60_000)

beforeEach(() => {
  vi.useRealTimers()
})

const stmt = (index: number, state: StatementState): StatementStatus => ({ index, sql: `S${index}`, state, error: null, affected: null, sets: [], elapsedMs: 1 })
const status = (...states: StatementState[]): QueryStatus => ({ queryId: 'q', finished: true, statements: states.map((s, i) => stmt(i, s)) })

describe('txSupported / ranStatements', () => {
  it('every SQL engine can run transactions, MongoDB cannot', async () => {
    const { tx } = await setup()
    expect(['postgres', 'mysql', 'mariadb', 'sqlite'].map((e) => tx.txSupported(e as never))).toEqual([true, true, true, true])
    expect(tx.txSupported('mongodb')).toBe(false)
  })

  it('counts the statements that ran, not the failed, cancelled or pending ones', async () => {
    const { tx } = await setup()
    expect(tx.ranStatements(status('done', 'done', 'error', 'pending'))).toBe(2)
    expect(tx.ranStatements(status('cancelled'))).toBe(0)
    expect(tx.ranStatements(null)).toBe(0)
  })
})

describe('txGone', () => {
  it('recognises errors that mean the backend lost the transaction', async () => {
    const { tx } = await setup()
    expect(tx.txGone(coreError('invalidInput', 'That transaction is no longer open.'))).toBe(true)
    expect(tx.txGone(coreError('notFound', 'transaction tx9'))).toBe(true)
    expect(tx.txGone(coreError('connection', 'connection reset by peer'))).toBe(true)
    expect(tx.txGone(coreError('ssh', 'tunnel closed'))).toBe(true)
    expect(tx.txGone(coreError('other', 'unknown transaction tx9'))).toBe(true)
    expect(tx.txGone('The transaction is no longer open')).toBe(true)
  })

  it('a failing statement or constraint is not "gone"', async () => {
    const { tx } = await setup()
    expect(tx.txGone(coreError('database', 'deferred constraint violated'))).toBe(false)
    expect(tx.txGone(new Error('disk full'))).toBe(false)
  })
})

describe('ensureTx', () => {
  it('begins a transaction on the tab\'s session and database and remembers it', async () => {
    const { tx, open, calls } = await setup()
    const tab = open()
    expect(await tx.ensureTx(tab)).toBe('tx1')
    expect(calls).toEqual([{ command: 'tx_begin', args: { sessionId: 's1', database: 'app' } }])
    expect(tab).toMatchObject({ txId: 'tx1', txStatements: 0 })
  })

  it('reuses the open transaction instead of beginning another', async () => {
    const { tx, open, calls } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    await tx.ensureTx(tab)
    expect(calls.filter((c) => c.command === 'tx_begin')).toHaveLength(1)
  })

  it('leaves the tab without a transaction when beginning fails', async () => {
    const { tx, open } = await setup({
      tx_begin: () => {
        throw coreError('unsupported', 'MongoDB has no transactions here')
      },
    })
    const tab = open()
    await expect(tx.ensureTx(tab)).rejects.toMatchObject({ kind: 'unsupported' })
    expect(tab.txId).toBeUndefined()
  })
})

describe('finishTx', () => {
  it('commits, clears the tab and says so', async () => {
    const { tx, open, calls, toasts } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    tab.txStatements = 3
    expect(await tx.finishTx(tab, 'commit')).toBe(true)
    expect(calls.at(-1)).toEqual({ command: 'tx_commit', args: { txId: 'tx1' } })
    expect(tab).toMatchObject({ txId: null, txStatements: 0 })
    expect(toasts()).toEqual(['success: Transaction committed'])
  })

  it('rolls back, clears the tab and says so', async () => {
    const { tx, open, calls, toasts } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    expect(await tx.finishTx(tab, 'rollback')).toBe(true)
    expect(calls.at(-1)).toEqual({ command: 'tx_rollback', args: { txId: 'tx1' } })
    expect(tab.txId).toBeNull()
    expect(toasts()).toEqual(['success: Transaction rolled back'])
  })

  it('does nothing without an open transaction', async () => {
    const { tx, open, calls, toasts } = await setup()
    expect(await tx.finishTx(open(), 'commit')).toBe(true)
    expect(calls).toEqual([])
    expect(toasts()).toEqual([])
  })

  it('a commit that finds the transaction gone clears the state and tells the user', async () => {
    const { tx, open, toasts } = await setup({
      tx_commit: () => {
        throw coreError('invalidInput', 'That transaction is no longer open.')
      },
    })
    const tab = open()
    await tx.ensureTx(tab)
    expect(await tx.finishTx(tab, 'commit')).toBe(false)
    expect(tab).toMatchObject({ txId: null, txStatements: 0 })
    expect(toasts()).toHaveLength(1)
    expect(toasts()[0]).toMatch(/^error: Commit failed: the transaction was no longer open/)
    expect(toasts()[0]).toMatch(/not saved/)
  })

  it('a rollback that finds the transaction gone clears the state too', async () => {
    const { tx, open, toasts } = await setup({
      tx_rollback: () => {
        throw coreError('connection', 'connection closed')
      },
    })
    const tab = open()
    await tx.ensureTx(tab)
    expect(await tx.finishTx(tab, 'rollback')).toBe(false)
    expect(tab.txId).toBeNull()
    expect(toasts()[0]).toMatch(/nothing to roll back/)
  })

  it('any other commit failure keeps the transaction open so it can be rolled back', async () => {
    const { tx, open, toasts } = await setup({
      tx_commit: () => {
        throw coreError('database', 'could not serialize access due to concurrent update')
      },
    })
    const tab = open()
    await tx.ensureTx(tab)
    tab.txStatements = 2
    expect(await tx.finishTx(tab, 'commit')).toBe(false)
    expect(tab).toMatchObject({ txId: 'tx1', txStatements: 2 })
    expect(toasts()).toEqual(['error: Commit failed: could not serialize access due to concurrent update'])
  })

  it('a commit the server rolled back (Postgres) clears the transaction', async () => {
    const { tx, open, toasts } = await setup({
      tx_commit: () => {
        throw coreError('database', 'insert or update violates foreign key constraint "fk" The transaction was rolled back.')
      },
    })
    const tab = open()
    await tx.ensureTx(tab)
    expect(await tx.finishTx(tab, 'commit')).toBe(false)
    expect(tab.txId).toBeNull()
    expect(toasts()[0]).toMatch(/^error: Commit failed: .*rolled back/)
  })

  it('a double click commits once', async () => {
    const { tx, open, calls } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    const [a, b] = await Promise.all([tx.finishTx(tab, 'commit'), tx.finishTx(tab, 'commit')])
    expect(calls.filter((c) => c.command === 'tx_commit')).toHaveLength(1)
    expect([a, b].sort()).toEqual([false, true])
  })
})

describe('setManual', () => {
  it('turns manual mode on', async () => {
    const { tx, open } = await setup()
    const tab = open()
    tx.setManual(tab, true)
    expect(tab.manual).toBe(true)
  })

  it('turns it off at once when no transaction is open', async () => {
    const { tx, open, editorRequests } = await setup()
    const tab = open()
    tx.setManual(tab, true)
    tx.setManual(tab, false)
    expect(tab.manual).toBeUndefined()
    expect(editorRequests.leaveManual).toBeNull()
  })

  it('with a transaction open, asks first and stays in manual mode', async () => {
    const { tx, open, editorRequests } = await setup()
    const tab = open()
    tx.setManual(tab, true)
    await tx.ensureTx(tab)
    tx.setManual(tab, false)
    expect(editorRequests.leaveManual).toBe(tab.id)
    expect(tab.manual).toBe(true)
  })
})

describe('finishTx while a query runs', () => {
  it('waits: a commit / rollback from the shortcut or palette would otherwise race the statement', async () => {
    const { tx, open, calls, toasts } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    tab.running = true
    expect(await tx.finishTx(tab, 'commit')).toBe(false)
    expect(await tx.finishTx(tab, 'rollback')).toBe(false)
    expect(calls.filter((c) => c.command === 'tx_commit' || c.command === 'tx_rollback')).toEqual([])
    expect(tab.txId).toBe('tx1')
    expect(toasts()).toHaveLength(2)
    expect(toasts()[0]).toMatch(/still running.*Stop it before you commit/)
    expect(toasts()[1]).toMatch(/Stop it before you roll back/)
    tab.running = false
    expect(await tx.finishTx(tab, 'commit')).toBe(true)
  })

  it('isEnding is true only while the commit is on its way', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    const { tx, open } = await setup({ tx_commit: () => gate.then(() => null) })
    const tab = open()
    await tx.ensureTx(tab)
    expect(tx.isEnding(tab)).toBe(false)
    const done = tx.finishTx(tab, 'commit')
    expect(tx.isEnding(tab)).toBe(true)
    release()
    await done
    expect(tx.isEnding(tab)).toBe(false)
  })
})

describe('disconnect', () => {
  it('disconnects at once when no tab has a transaction open', async () => {
    const { tx, open, editorRequests, calls } = await setup()
    open()
    tx.disconnect('s1')
    expect(editorRequests.disconnect).toBeNull()
    await vi.waitFor(() => expect(calls.some((c) => c.command === 'connection_close')).toBe(true))
  })

  it('asks first when a tab has a transaction open, and leaves the connection alone', async () => {
    const { tx, open, editorRequests, calls } = await setup()
    const tab = open()
    await tx.ensureTx(tab)
    tx.disconnect('s1')
    expect(editorRequests.disconnect).toBe('s1')
    expect(calls.some((c) => c.command === 'connection_close')).toBe(false)
  })

  it('openTxTab finds the session\'s tab with an open transaction, and only that session\'s', async () => {
    const { tx, tabs, open } = await setup()
    const tab = open()
    expect(tx.openTxTab('s1')).toBeUndefined()
    await tx.ensureTx(tab)
    expect(tx.openTxTab('s1')?.id).toBe(tab.id)
    expect(tx.openTxTab('s2')).toBeUndefined()
    tabs.updateQuery(tab.id, { txId: null })
    expect(tx.openTxTab('s1')).toBeUndefined()
  })
})
