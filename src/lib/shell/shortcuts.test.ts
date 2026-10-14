import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'

// The file and transaction actions have their own tests; here we only check which key triggers what.
const actions = vi.hoisted(() => ({ openSqlFile: vi.fn(), saveTab: vi.fn(), saveTabAs: vi.fn(), finishTx: vi.fn() }))
vi.mock('../editor/sqlFiles', () => ({ openSqlFile: actions.openSqlFile, saveTab: actions.saveTab, saveTabAs: actions.saveTabAs }))
vi.mock('../editor/transaction', () => ({ finishTx: actions.finishTx }))

type Mods = Partial<Pick<KeyboardEvent, 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'repeat'>>
const ev = (key: string, mods: Mods = {}) => ({ key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods }) as KeyboardEvent

async function setup(platform: 'MacIntel' | 'Win32') {
  vi.resetModules()
  installStorage()
  vi.stubGlobal('navigator', { platform })
  const setAttribute = vi.fn()
  vi.stubGlobal('document', { documentElement: { setAttribute } })
  vi.stubGlobal('window', { matchMedia: () => ({ matches: false, addEventListener() {} }) })
  const { handleShortcut } = await import('./shortcuts')
  const { sessions } = await import('../stores/sessions.svelte')
  const { tabs } = await import('../stores/tabs.svelte')
  const { theme } = await import('../stores/theme.svelte')
  const { ui } = await import('../stores/ui.svelte')
  /** The platform's "command" modifier. */
  const mod: Mods = platform === 'MacIntel' ? { metaKey: true } : { ctrlKey: true }
  const notMod: Mods = platform === 'MacIntel' ? { ctrlKey: true } : { metaKey: true }
  const enterWorkspace = (id = 's1', db = 'app') => {
    sessions.activeId = id
    sessions.view = 'workspace'
    sessions.currentDb[id] = db
  }
  return { handleShortcut, sessions, tabs, theme, ui, mod, notMod, enterWorkspace, setAttribute }
}

// The first import compiles the svelte runtime and the stores, which is slow on a busy machine.
beforeAll(async () => {
  await setup('MacIntel')
  vi.unstubAllGlobals()
}, 60_000)

afterEach(() => {
  vi.unstubAllGlobals()
})

describe.each(['MacIntel', 'Win32'] as const)('global shortcuts (%s)', (platform) => {
  it('Mod-K toggles the command palette', async () => {
    const { handleShortcut, ui, mod } = await setup(platform)
    expect(handleShortcut(ev('k', mod))).toBe(true)
    expect(ui.palette).toBe(true)
    expect(handleShortcut(ev('k', mod))).toBe(true)
    expect(ui.palette).toBe(false)
  })

  it('works with caps lock / uppercase letters', async () => {
    const { handleShortcut, ui, mod } = await setup(platform)
    expect(handleShortcut(ev('K', mod))).toBe(true)
    expect(ui.palette).toBe(true)
  })

  it('Mod-K works outside a workspace too', async () => {
    const { handleShortcut, ui, mod, sessions } = await setup(platform)
    expect(sessions.view).toBe('connections')
    expect(handleShortcut(ev('k', mod))).toBe(true)
    expect(ui.palette).toBe(true)
  })

  it('Mod-Shift-L toggles the theme', async () => {
    const { handleShortcut, theme, mod } = await setup(platform)
    theme.init()
    expect(theme.isDark).toBe(false)
    expect(handleShortcut(ev('L', { ...mod, shiftKey: true }))).toBe(true)
    expect(theme.isDark).toBe(true)
    expect(handleShortcut(ev('l', { ...mod, shiftKey: true }))).toBe(true)
    expect(theme.isDark).toBe(false)
  })

  it('Mod-L without shift is not a shortcut', async () => {
    const { handleShortcut, theme, mod } = await setup(platform)
    theme.init()
    expect(handleShortcut(ev('l', mod))).toBe(false)
    expect(theme.isDark).toBe(false)
  })

  it('Mod-/ and Mod-? toggle the shortcut help', async () => {
    const { handleShortcut, ui, mod } = await setup(platform)
    expect(handleShortcut(ev('/', mod))).toBe(true)
    expect(ui.shortcutsHelp).toBe(true)
    expect(handleShortcut(ev('?', { ...mod, shiftKey: true }))).toBe(true)
    expect(ui.shortcutsHelp).toBe(false)
  })

  it('Mod-Shift-K is not the palette shortcut', async () => {
    const { handleShortcut, ui, mod } = await setup(platform)
    expect(handleShortcut(ev('k', { ...mod, shiftKey: true }))).toBe(false)
    expect(ui.palette).toBe(false)
  })

  it('ignores a plain key press', async () => {
    const { handleShortcut, ui } = await setup(platform)
    expect(handleShortcut(ev('k'))).toBe(false)
    expect(handleShortcut(ev('/'))).toBe(false)
    expect(ui.palette).toBe(false)
    expect(ui.shortcutsHelp).toBe(false)
  })

  it('ignores Alt combinations (AltGr / option characters)', async () => {
    const { handleShortcut, ui, mod, enterWorkspace, tabs } = await setup(platform)
    enterWorkspace()
    expect(handleShortcut(ev('k', { ...mod, altKey: true }))).toBe(false)
    expect(handleShortcut(ev('t', { ...mod, altKey: true }))).toBe(false)
    expect(handleShortcut(ev('1', { ...mod, altKey: true }))).toBe(false)
    expect(ui.palette).toBe(false)
    expect(tabs.list).toEqual([])
  })

  it('the other platform\'s modifier does not count', async () => {
    const { handleShortcut, ui, notMod } = await setup(platform)
    expect(handleShortcut(ev('k', notMod))).toBe(false)
    expect(ui.palette).toBe(false)
  })
})

describe.each(['MacIntel', 'Win32'] as const)('workspace shortcuts (%s)', (platform) => {
  it('Mod-T opens a query tab on the active session\'s current database', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace('s1', 'analytics')
    expect(handleShortcut(ev('t', mod))).toBe(true)
    const [tab] = tabs.forSession('s1')
    expect(tab).toMatchObject({ kind: 'query', database: 'analytics', title: 'Query 1' })
    expect(tabs.active('s1')?.id).toBe(tab.id)
  })

  it('Mod-T twice opens two tabs', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    handleShortcut(ev('t', mod))
    handleShortcut(ev('t', mod))
    expect(tabs.forSession('s1').map((t) => t.title)).toEqual(['Query 1', 'Query 2'])
  })

  it('Mod-Shift-T is not handled', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    expect(handleShortcut(ev('t', { ...mod, shiftKey: true }))).toBe(false)
    expect(tabs.list).toEqual([])
  })

  it('Mod-W closes the active tab', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    const a = tabs.newQuery('s1', 'app')
    tabs.newQuery('s1', 'app')
    expect(handleShortcut(ev('w', mod))).toBe(true)
    expect(tabs.list.map((t) => t.id)).toEqual([a.id])
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('Mod-W with no tabs is handled and harmless', async () => {
    const { handleShortcut, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    expect(handleShortcut(ev('w', mod))).toBe(true)
  })

  it('Mod-P opens the palette (and does not toggle it shut)', async () => {
    const { handleShortcut, ui, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    expect(handleShortcut(ev('p', mod))).toBe(true)
    expect(ui.palette).toBe(true)
    expect(handleShortcut(ev('p', mod))).toBe(true)
    expect(ui.palette).toBe(true)
  })

  it('Mod-1..8 select the nth tab, Mod-9 the last', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    const made = Array.from({ length: 4 }, () => tabs.newQuery('s1', 'app'))
    expect(handleShortcut(ev('1', mod))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(made[0].id)
    expect(handleShortcut(ev('3', mod))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(made[2].id)
    expect(handleShortcut(ev('9', mod))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(made[3].id)
  })

  it('a digit beyond the tab count is handled but keeps the selection', async () => {
    const { handleShortcut, tabs, mod, enterWorkspace } = await setup(platform)
    enterWorkspace()
    const a = tabs.newQuery('s1', 'app')
    expect(handleShortcut(ev('7', mod))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(a.id)
  })

  it('Mod-0 and Shift-digit are not handled', async () => {
    const { handleShortcut, mod, enterWorkspace, tabs } = await setup(platform)
    enterWorkspace()
    tabs.newQuery('s1', 'app')
    expect(handleShortcut(ev('0', mod))).toBe(false)
    expect(handleShortcut(ev('1', { ...mod, shiftKey: true }))).toBe(false)
  })

  it('Ctrl-Tab cycles forward and Ctrl-Shift-Tab backward on every platform', async () => {
    const { handleShortcut, tabs, enterWorkspace } = await setup(platform)
    enterWorkspace()
    const a = tabs.newQuery('s1', 'app')
    const b = tabs.newQuery('s1', 'app')
    const c = tabs.newQuery('s1', 'app')
    tabs.select(a.id)
    expect(handleShortcut(ev('Tab', { ctrlKey: true }))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(b.id)
    expect(handleShortcut(ev('Tab', { ctrlKey: true }))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(c.id)
    expect(handleShortcut(ev('Tab', { ctrlKey: true, shiftKey: true }))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(b.id)
  })

  it('Ctrl-Tab outside a workspace is swallowed (true) without effect', async () => {
    const { handleShortcut, tabs, sessions } = await setup(platform)
    tabs.newQuery('s1', 'app')
    tabs.newQuery('s1', 'app')
    const before = tabs.active('s1')?.id
    expect(sessions.view).toBe('connections')
    expect(handleShortcut(ev('Tab', { ctrlKey: true }))).toBe(true)
    expect(tabs.active('s1')?.id).toBe(before)
  })
})

describe.each(['MacIntel', 'Win32'] as const)('workspace shortcuts need an open workspace (%s)', (platform) => {
  it('are not handled on the welcome screen', async () => {
    const { handleShortcut, tabs, mod, sessions } = await setup(platform)
    expect(sessions.view).toBe('connections')
    for (const key of ['t', 'w', 'p', '1', '9']) expect(handleShortcut(ev(key, mod))).toBe(false)
    expect(tabs.list).toEqual([])
  })

  it('are not handled when the workspace view has no active session', async () => {
    const { handleShortcut, tabs, mod, sessions } = await setup(platform)
    sessions.view = 'workspace'
    sessions.activeId = null
    expect(handleShortcut(ev('t', mod))).toBe(false)
    expect(tabs.list).toEqual([])
  })

  it('are not handled when a session is active but the welcome screen is showing', async () => {
    const { handleShortcut, tabs, mod, sessions } = await setup(platform)
    sessions.activeId = 's1'
    sessions.view = 'connections'
    expect(handleShortcut(ev('t', mod))).toBe(false)
    expect(tabs.list).toEqual([])
  })
})

describe('platform detection', () => {
  it('on macOS Ctrl-K is not Mod', async () => {
    const { handleShortcut, ui } = await setup('MacIntel')
    expect(handleShortcut(ev('k', { ctrlKey: true }))).toBe(false)
    expect(ui.palette).toBe(false)
  })

  it('on Windows/Linux Meta-K is not Mod', async () => {
    const { handleShortcut, ui } = await setup('Win32')
    expect(handleShortcut(ev('k', { metaKey: true }))).toBe(false)
    expect(ui.palette).toBe(false)
  })
})

describe.each(['MacIntel', 'Win32'] as const)('SQL file and transaction shortcuts (%s)', (platform) => {
  beforeEach(() => {
    for (const fn of Object.values(actions)) fn.mockReset()
  })

  const session = { sessionId: 's1', config: { engine: 'postgres' } }
  /** A workspace on session s1 whose active tab is a new query tab. */
  async function inQuery() {
    const ctx = await setup(platform)
    ctx.enterWorkspace()
    ctx.sessions.list = [session as never]
    const made = ctx.tabs.newQuery('s1', 'app', 'SELECT 1')
    return { ...ctx, id: made.id, tab: () => ctx.tabs.active('s1') as import('../stores/tabs.svelte').QueryTab }
  }

  it('Mod-O opens the file picker for the active session', async () => {
    const { handleShortcut, mod, sessions } = await inQuery()
    expect(handleShortcut(ev('o', mod))).toBe(true)
    expect(actions.openSqlFile).toHaveBeenCalledWith(sessions.list[0])
  })

  it('Mod-O works from a table tab too, but not without a session', async () => {
    const { handleShortcut, mod, tabs, sessions } = await inQuery()
    tabs.openTable('s1', { database: 'app', schema: null, name: 'users' })
    expect(handleShortcut(ev('o', mod))).toBe(true)
    sessions.list = []
    expect(handleShortcut(ev('o', mod))).toBe(false)
    expect(actions.openSqlFile).toHaveBeenCalledTimes(1)
  })

  it('Mod-Shift-O is not Open', async () => {
    const { handleShortcut, mod } = await inQuery()
    expect(handleShortcut(ev('o', { ...mod, shiftKey: true }))).toBe(false)
    expect(actions.openSqlFile).not.toHaveBeenCalled()
  })

  it('Mod-S in a query tab without a file saves it too (saveTab asks for the path)', async () => {
    const { handleShortcut, mod, tab } = await inQuery()
    expect(handleShortcut(ev('s', mod))).toBe(true)
    expect(actions.saveTab).toHaveBeenCalledWith(tab())
    expect(actions.saveTabAs).not.toHaveBeenCalled()
  })

  it('a held key does not repeat the action, but is still swallowed', async () => {
    const { handleShortcut, mod, tabs, id } = await inQuery()
    tabs.updateQuery(id, { txId: 'tx1' })
    for (const [key, extra] of [['o', {}], ['s', {}], ['s', { shiftKey: true }], ['c', { shiftKey: true }]] as const) {
      expect(handleShortcut(ev(key, { ...mod, ...extra, repeat: true }))).toBe(true)
    }
    for (const fn of Object.values(actions)) expect(fn).not.toHaveBeenCalled()
  })

  it('Mod-S in a query tab with a file saves it', async () => {
    const { handleShortcut, mod, tabs, id, tab } = await inQuery()
    tabs.updateQuery(id, { file: '/q/a.sql' })
    expect(handleShortcut(ev('s', mod))).toBe(true)
    expect(actions.saveTab).toHaveBeenCalledWith(tab())
  })

  it('Mod-S in a table tab is left to the table view (it commits pending changes)', async () => {
    const { handleShortcut, mod, tabs } = await inQuery()
    tabs.openTable('s1', { database: 'app', schema: null, name: 'users' })
    expect(handleShortcut(ev('s', mod))).toBe(false)
    expect(handleShortcut(ev('s', { ...mod, shiftKey: true }))).toBe(false)
    expect(actions.saveTab).not.toHaveBeenCalled()
    expect(actions.saveTabAs).not.toHaveBeenCalled()
  })

  it('Mod-Shift-S asks where to save the query, file or not', async () => {
    const { handleShortcut, mod, tab, tabs, id } = await inQuery()
    expect(handleShortcut(ev('S', { ...mod, shiftKey: true }))).toBe(true)
    expect(actions.saveTabAs).toHaveBeenCalledWith(tab())
    tabs.updateQuery(id, { file: '/q/a.sql' })
    expect(handleShortcut(ev('s', { ...mod, shiftKey: true }))).toBe(true)
    expect(actions.saveTabAs).toHaveBeenCalledTimes(2)
    expect(actions.saveTab).not.toHaveBeenCalled()
  })

  it('Mod-Shift-C commits and Mod-Shift-R rolls back the open transaction', async () => {
    const { handleShortcut, mod, tabs, id, tab } = await inQuery()
    tabs.updateQuery(id, { txId: 'tx1' })
    expect(handleShortcut(ev('C', { ...mod, shiftKey: true }))).toBe(true)
    expect(actions.finishTx).toHaveBeenLastCalledWith(tab(), 'commit')
    expect(handleShortcut(ev('r', { ...mod, shiftKey: true }))).toBe(true)
    expect(actions.finishTx).toHaveBeenLastCalledWith(tab(), 'rollback')
  })

  it('Mod-Shift-C in a data grid stays "copy with headers" and never commits the transaction', async () => {
    const { handleShortcut, mod, tabs, id } = await inQuery()
    tabs.updateQuery(id, { txId: 'tx1' })
    const grid = { closest: (selector: string) => (selector === '[role="grid"]' ? {} : null) }
    const inGrid = (key: string) => ({ ...ev(key, { ...mod, shiftKey: true }), target: grid }) as unknown as KeyboardEvent
    expect(handleShortcut(inGrid('C'))).toBe(false)
    expect(actions.finishTx).not.toHaveBeenCalled()
    // rolling back is not a copy key, so it still works from anywhere
    expect(handleShortcut(inGrid('r'))).toBe(true)
    expect(actions.finishTx).toHaveBeenLastCalledWith(expect.anything(), 'rollback')
  })

  it('the transaction keys are free when nothing is open, so other features can use them', async () => {
    const { handleShortcut, mod } = await inQuery()
    expect(handleShortcut(ev('c', { ...mod, shiftKey: true }))).toBe(false)
    expect(handleShortcut(ev('r', { ...mod, shiftKey: true }))).toBe(false)
    expect(actions.finishTx).not.toHaveBeenCalled()
  })

  it('the transaction keys are free in a table tab even if a query tab has a transaction', async () => {
    const { handleShortcut, mod, tabs, id } = await inQuery()
    tabs.updateQuery(id, { txId: 'tx1' })
    tabs.openTable('s1', { database: 'app', schema: null, name: 'users' })
    expect(handleShortcut(ev('c', { ...mod, shiftKey: true }))).toBe(false)
    expect(actions.finishTx).not.toHaveBeenCalled()
  })

  it('plain Mod-R still refreshes the schema on a query tab', async () => {
    const { handleShortcut, mod, tabs, id } = await inQuery()
    tabs.updateQuery(id, { txId: 'tx1' })
    // Mod-R is not the rollback key (that is Mod-Shift-R)
    expect(handleShortcut(ev('r', mod))).toBe(true)
    expect(actions.finishTx).not.toHaveBeenCalled()
  })

  it('none of them are handled on the welcome screen', async () => {
    const { handleShortcut, mod } = await setup(platform)
    for (const key of ['o', 's']) expect(handleShortcut(ev(key, mod))).toBe(false)
    expect(actions.openSqlFile).not.toHaveBeenCalled()
  })
})
