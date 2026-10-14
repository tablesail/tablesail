import { beforeAll, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'
import type { SessionInfo } from '../types'

const actions = vi.hoisted(() => ({
  openSqlFile: vi.fn(),
  saveTab: vi.fn(),
  saveTabAs: vi.fn(),
  pickSqlImport: vi.fn(),
  finishTx: vi.fn(),
  setManual: vi.fn(),
}))
vi.mock('./sqlFiles', () => ({
  openSqlFile: actions.openSqlFile,
  saveTab: actions.saveTab,
  saveTabAs: actions.saveTabAs,
  pickSqlImport: actions.pickSqlImport,
  importBlockedReason: (s: SessionInfo) => (s.config.engine === 'mongodb' || s.config.readOnly ? 'blocked' : null),
}))
vi.mock('./transaction', () => ({
  finishTx: actions.finishTx,
  setManual: actions.setManual,
  txSupported: (engine: string) => engine !== 'mongodb',
}))

const session = (over: { engine?: string; readOnly?: boolean } = {}) =>
  ({ sessionId: 's1', config: { id: 'c1', name: 'x', engine: over.engine ?? 'postgres', readOnly: over.readOnly ?? false } }) as unknown as SessionInfo

async function setup() {
  vi.resetModules()
  installStorage()
  for (const fn of Object.values(actions)) fn.mockReset()
  const { tabs } = await import('../stores/tabs.svelte')
  const { editorCommands } = await import('./commands')
  const live = (id: string) => tabs.list.find((t) => t.id === id) as import('../stores/tabs.svelte').QueryTab
  return { tabs, editorCommands, live }
}

// The first import compiles the svelte runtime and the stores, which is slow on a busy machine.
beforeAll(async () => {
  await setup()
}, 60_000)

const labels = (cmds: { label: string }[]) => cmds.map((c) => c.label)

describe('editorCommands', () => {
  it('without a query tab: open a file and import a dump', async () => {
    const { editorCommands } = await setup()
    expect(labels(editorCommands(session()))).toEqual(['Open SQL file…', 'Import SQL dump…'])
  })

  it('on a query tab: save to a file and the transaction mode as well', async () => {
    const { editorCommands, tabs } = await setup()
    tabs.newQuery('s1', 'app')
    expect(labels(editorCommands(session()))).toEqual(['Open SQL file…', 'Save to file…', 'Import SQL dump…', 'Switch to manual transactions'])
  })

  it('a file tab can also be saved in place; a manual tab can go back to auto-commit', async () => {
    const { editorCommands, tabs } = await setup()
    const tab = tabs.newQuery('s1', 'app', 'x', { file: '/a.sql' })
    tabs.updateQuery(tab.id, { manual: true })
    expect(labels(editorCommands(session()))).toEqual([
      'Open SQL file…',
      'Save SQL file',
      'Save to file…',
      'Import SQL dump…',
      'Switch to auto-commit',
    ])
  })

  it('commit and roll back are quick actions, offered only with an open transaction', async () => {
    const { editorCommands, tabs, live } = await setup()
    const tab = tabs.newQuery('s1', 'app')
    expect(editorCommands(session()).some((c) => c.label.includes('transaction') && c.group === 'Quick actions')).toBe(false)
    tabs.updateQuery(tab.id, { txId: 'tx1' })
    const cmds = editorCommands(session())
    expect(cmds.slice(0, 2).map((c) => [c.group, c.label])).toEqual([
      ['Quick actions', 'Commit transaction'],
      ['Quick actions', 'Roll back transaction'],
    ])
    cmds[0].run()
    cmds[1].run()
    expect(actions.finishTx.mock.calls).toEqual([
      [live(tab.id), 'commit'],
      [live(tab.id), 'rollback'],
    ])
  })

  it('they act on the active tab, not on a table tab', async () => {
    const { editorCommands, tabs } = await setup()
    const q = tabs.newQuery('s1', 'app')
    tabs.updateQuery(q.id, { txId: 'tx1' })
    tabs.openTable('s1', { database: 'app', schema: null, name: 'users' })
    expect(labels(editorCommands(session()))).toEqual(['Open SQL file…', 'Import SQL dump…'])
  })

  it('MongoDB has neither the dump import nor transactions', async () => {
    const { editorCommands, tabs } = await setup()
    tabs.newQuery('s1', 'app')
    expect(labels(editorCommands(session({ engine: 'mongodb' })))).toEqual(['Open SQL file…', 'Save to file…'])
  })

  it('a read-only connection cannot import a dump', async () => {
    const { editorCommands } = await setup()
    expect(labels(editorCommands(session({ readOnly: true })))).toEqual(['Open SQL file…'])
  })

  it('the file and mode commands call their actions', async () => {
    const { editorCommands, tabs, live } = await setup()
    const tab = tabs.newQuery('s1', 'app', 'x', { file: '/a.sql' })
    const s = session()
    const byLabel = (l: string) => editorCommands(s).find((c) => c.label === l)!
    byLabel('Open SQL file…').run()
    byLabel('Save SQL file').run()
    byLabel('Save to file…').run()
    byLabel('Import SQL dump…').run()
    byLabel('Switch to manual transactions').run()
    expect(actions.openSqlFile).toHaveBeenCalledWith(s)
    expect(actions.saveTab).toHaveBeenCalledWith(live(tab.id))
    expect(actions.saveTabAs).toHaveBeenCalledWith(live(tab.id))
    expect(actions.pickSqlImport).toHaveBeenCalledWith(s)
    expect(actions.setManual).toHaveBeenCalledWith(live(tab.id), true)
  })

  it('palette commands are grouped so the default palette stays short', async () => {
    const { editorCommands, tabs } = await setup()
    tabs.newQuery('s1', 'app')
    expect([...new Set(editorCommands(session()).map((c) => c.group))]).toEqual(['File', 'Transactions'])
  })
})
