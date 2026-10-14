import { beforeAll, describe, expect, it, vi } from 'vitest'
import { installStorage, routeInvoke, type InvokeHandlers } from '../../test/helpers'
import type { SessionInfo } from '../types'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => dialog)

const session = (over: { engine?: string; readOnly?: boolean } = {}) =>
  ({
    sessionId: 's1',
    databases: ['app'],
    config: { id: 'c1', name: 'Local', engine: over.engine ?? 'postgres', readOnly: over.readOnly ?? false, env: 'local' },
  }) as unknown as SessionInfo

async function setup(files: Record<string, string> = {}, handlers: InvokeHandlers = {}) {
  vi.resetModules()
  installStorage()
  dialog.open.mockReset()
  dialog.save.mockReset()
  const written: Record<string, string> = {}
  const calls: string[] = []
  const routed = routeInvoke({
    file_read_text: ({ path }) => {
      if (!(String(path) in files)) throw { kind: 'io', message: `No such file: ${path}`, code: null, statement: null }
      return files[String(path)]
    },
    file_write_text: ({ path, contents }) => {
      written[String(path)] = String(contents)
      return null
    },
    ...handlers,
  })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    calls.push(command)
    return routed(command, args)
  })
  const { tabs, queryDirty } = await import('../stores/tabs.svelte')
  const { sessions } = await import('../stores/sessions.svelte')
  const { ui } = await import('../stores/ui.svelte')
  const { editorRequests } = await import('./requests.svelte')
  const f = await import('./sqlFiles')
  sessions.currentDb['s1'] = 'app'
  const live = (id: string) => tabs.list.find((t) => t.id === id) as import('../stores/tabs.svelte').QueryTab
  const toasts = () => ui.toasts.map((t) => `${t.kind}: ${t.text}`)
  return { tabs, queryDirty, ui, editorRequests, f, written, calls, live, toasts }
}

// The first import compiles the svelte runtime and the stores, which is slow on a busy machine.
beforeAll(async () => {
  await setup()
}, 60_000)

describe('suggestFileName', () => {
  it('uses the file the tab already has', async () => {
    const { f } = await setup()
    expect(f.suggestFileName({ title: 'x.sql', file: '/home/me/q/report.sql' })).toBe('report.sql')
  })

  it('names a new tab after its title', async () => {
    const { f } = await setup()
    expect(f.suggestFileName({ title: 'Query 3' })).toBe('Query 3.sql')
    expect(f.suggestFileName({ title: 'Revenue by month' })).toBe('Revenue by month.sql')
  })

  it('keeps a title that already has an extension, and cleans characters files cannot have', async () => {
    const { f } = await setup()
    expect(f.suggestFileName({ title: 'dump.SQL' })).toBe('dump.SQL')
    expect(f.suggestFileName({ title: 'a/b: c?' })).toBe('a_b_ c_.sql')
    expect(f.suggestFileName({ title: '  ' })).toBe('query.sql')
  })
})

describe('openSqlPath', () => {
  it('opens the file in a new tab titled with its name, remembering the path', async () => {
    const { f, tabs, queryDirty, live } = await setup({ '/q/report.sql': 'SELECT 1;\n' })
    const tab = await f.openSqlPath('s1', 'app', '/q/report.sql')
    expect(tab).toMatchObject({ title: 'report.sql', file: '/q/report.sql', sql: 'SELECT 1;\n', database: 'app' })
    expect(tabs.active('s1')?.id).toBe(tab?.id)
    expect(queryDirty(live(tab!.id))).toBe(false)
  })

  it('drops a UTF-8 byte order mark', async () => {
    const { f } = await setup({ '/q/a.sql': '\uFEFFSELECT 1' })
    expect((await f.openSqlPath('s1', 'app', '/q/a.sql'))?.sql).toBe('SELECT 1')
  })

  it('shows a CRLF file with LF line endings, clean, and remembers to write CRLF back', async () => {
    const { f, queryDirty, live } = await setup({ '/q/win.sql': 'SELECT 1;\r\nSELECT 2;\r\n' })
    const tab = live((await f.openSqlPath('s1', 'app', '/q/win.sql'))!.id)
    expect(tab).toMatchObject({ sql: 'SELECT 1;\nSELECT 2;\n', eol: '\r\n' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('leaves an LF file (or one that is mostly LF) as it is', async () => {
    const { f, live } = await setup({ '/q/a.sql': 'a\nb\nc\r\n', '/q/b.sql': 'a\nb' })
    expect(live((await f.openSqlPath('s1', 'app', '/q/a.sql'))!.id)).toMatchObject({ sql: 'a\nb\nc\n', eol: undefined })
    expect(live((await f.openSqlPath('s1', 'app', '/q/b.sql'))!.id).eol).toBeUndefined()
  })

  it('opening a file twice while the first read is still running gives one tab', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    const { f, tabs } = await setup({ '/q/a.sql': 'SELECT 1' }, { file_read_text: () => gate.then(() => 'SELECT 1') })
    const first = f.openSqlPath('s1', 'app', '/q/a.sql')
    const second = f.openSqlPath('s1', 'app', '/q/a.sql')
    release()
    const [a, b] = await Promise.all([first, second])
    expect(tabs.list).toHaveLength(1)
    expect(a?.id).toBe(b?.id)
  })

  it('selects the tab that already has the file instead of opening a second one', async () => {
    const { f, tabs, calls } = await setup({ '/q/a.sql': 'SELECT 1' })
    const first = await f.openSqlPath('s1', 'app', '/q/a.sql')
    tabs.newQuery('s1', 'app')
    const again = await f.openSqlPath('s1', 'app', '/q/a.sql')
    expect(again?.id).toBe(first?.id)
    expect(tabs.active('s1')?.id).toBe(first?.id)
    expect(tabs.list).toHaveLength(2)
    expect(calls.filter((c) => c === 'file_read_text')).toHaveLength(1)
  })

  it('the same file in another session is a separate tab', async () => {
    const { f, tabs } = await setup({ '/q/a.sql': 'SELECT 1' })
    await f.openSqlPath('s1', 'app', '/q/a.sql')
    await f.openSqlPath('s2', 'app', '/q/a.sql')
    expect(tabs.list).toHaveLength(2)
  })

  it('reports a file that cannot be read and opens nothing', async () => {
    const { f, tabs, toasts } = await setup({})
    expect(await f.openSqlPath('s1', 'app', '/q/gone.sql')).toBeNull()
    expect(tabs.list).toEqual([])
    expect(toasts()).toEqual(['error: Could not open gone.sql: No such file: /q/gone.sql'])
  })

  it('refuses a file too big for the editor and points at the dump import', async () => {
    const { f, tabs, toasts } = await setup({ '/q/big.sql': 'x'.repeat(5_000_001) })
    expect(await f.openSqlPath('s1', 'app', '/q/big.sql')).toBeNull()
    expect(tabs.list).toEqual([])
    expect(toasts()[0]).toMatch(/too large to edit.*Import SQL dump/)
  })
})

describe('openSqlFile', () => {
  it('asks for a .sql / .txt file and opens it on the current database', async () => {
    const { f, tabs } = await setup({ '/q/a.sql': 'SELECT 1' })
    dialog.open.mockResolvedValue('/q/a.sql')
    const tab = await f.openSqlFile(session())
    expect(dialog.open).toHaveBeenCalledWith({ multiple: false, directory: false, title: 'Open SQL file', filters: [{ name: 'SQL', extensions: ['sql', 'txt'] }] })
    expect(tab?.database).toBe('app')
    expect(tabs.list).toHaveLength(1)
  })

  it('does nothing when the dialog is cancelled', async () => {
    const { f, tabs, calls } = await setup()
    dialog.open.mockResolvedValue(null)
    expect(await f.openSqlFile(session())).toBeNull()
    expect(tabs.list).toEqual([])
    expect(calls).toEqual([])
  })

  it('ignores a second request while the picker is open', async () => {
    const { f, tabs } = await setup({ '/q/a.sql': 'SELECT 1' })
    let pick: (path: string) => void = () => {}
    dialog.open.mockReturnValue(new Promise<string>((r) => (pick = r)))
    const first = f.openSqlFile(session())
    expect(await f.openSqlFile(session())).toBeNull()
    pick('/q/a.sql')
    await first
    expect(dialog.open).toHaveBeenCalledTimes(1)
    expect(tabs.list).toHaveLength(1)
  })

  it('offers query files rather than .sql for MongoDB', async () => {
    const { f } = await setup({ '/q/a.json': '{}' })
    dialog.open.mockResolvedValue('/q/a.json')
    await f.openSqlFile(session({ engine: 'mongodb' }))
    expect(dialog.open.mock.calls[0][0].filters).toEqual([{ name: 'Query', extensions: ['json', 'js', 'txt'] }])
  })
})

describe('saving', () => {
  it('writes a file tab back to its own file and marks it saved', async () => {
    const { f, tabs, queryDirty, live, written, toasts } = await setup({ '/q/a.sql': 'SELECT 1' })
    const tab = live((await f.openSqlPath('s1', 'app', '/q/a.sql'))!.id)
    tabs.updateQuery(tab.id, { sql: 'SELECT 2' })
    expect(queryDirty(tab)).toBe(true)
    expect(await f.saveTab(tab)).toBe(true)
    expect(written).toEqual({ '/q/a.sql': 'SELECT 2' })
    expect(queryDirty(tab)).toBe(false)
    expect(dialog.save).not.toHaveBeenCalled()
    expect(toasts()).toEqual(['success: Saved a.sql'])
  })

  it('a failed write keeps the edits unsaved and says why', async () => {
    const { f, tabs, queryDirty, live, toasts } = await setup(
      { '/q/a.sql': 'SELECT 1' },
      {
        file_write_text: () => {
          throw { kind: 'io', message: 'Permission denied', code: null, statement: null }
        },
      },
    )
    const tab = live((await f.openSqlPath('s1', 'app', '/q/a.sql'))!.id)
    tabs.updateQuery(tab.id, { sql: 'SELECT 2' })
    expect(await f.saveTab(tab)).toBe(false)
    expect(queryDirty(tab)).toBe(true)
    expect(toasts()).toEqual(['error: Could not save a.sql: Permission denied'])
  })

  it('edits made while the write is in flight stay unsaved', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    const { f, tabs, queryDirty, live } = await setup(
      { '/q/a.sql': 'SELECT 1' },
      { file_write_text: () => gate.then(() => null) },
    )
    const tab = live((await f.openSqlPath('s1', 'app', '/q/a.sql'))!.id)
    tabs.updateQuery(tab.id, { sql: 'SELECT 2' })
    const saving = f.saveTab(tab)
    tabs.updateQuery(tab.id, { sql: 'SELECT 22' })
    release()
    await saving
    expect(tab.savedSql).toBe('SELECT 2')
    expect(queryDirty(tab)).toBe(true)
  })

  it('a tab without a file asks for a path, named after the tab, and becomes that file', async () => {
    const { f, tabs, live, written, queryDirty } = await setup()
    const tab = live(tabs.newQuery('s1', 'app', 'SELECT 1').id)
    dialog.save.mockResolvedValue('/out/mine.sql')
    expect(await f.saveTab(tab)).toBe(true)
    expect(dialog.save).toHaveBeenCalledWith({ title: 'Save to file', defaultPath: 'Query 1.sql', filters: [{ name: 'SQL', extensions: ['sql'] }] })
    expect(written).toEqual({ '/out/mine.sql': 'SELECT 1' })
    expect(tab).toMatchObject({ file: '/out/mine.sql', title: 'mine.sql' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('writes a CRLF file back with CRLF line endings, and only marks it saved for the LF text', async () => {
    const { f, tabs, live, written, queryDirty } = await setup({ '/q/win.sql': 'a\r\nb\r\n' })
    const tab = live((await f.openSqlPath('s1', 'app', '/q/win.sql'))!.id)
    tabs.updateQuery(tab.id, { sql: 'a\nb\nc\n' })
    expect(await f.saveTab(tab)).toBe(true)
    expect(written).toEqual({ '/q/win.sql': 'a\r\nb\r\nc\r\n' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('typing a CRLF file back to what it was is clean again', async () => {
    const { f, tabs, live, queryDirty } = await setup({ '/q/win.sql': 'a\r\nb' })
    const tab = live((await f.openSqlPath('s1', 'app', '/q/win.sql'))!.id)
    tabs.updateQuery(tab.id, { sql: 'a\nbc' })
    expect(queryDirty(tab)).toBe(true)
    tabs.updateQuery(tab.id, { sql: 'a\nb' })
    expect(queryDirty(tab)).toBe(false)
  })

  it('a second save while the path dialog is open does not stack another dialog', async () => {
    const { f, tabs, live, written } = await setup()
    const tab = live(tabs.newQuery('s1', 'app', 'x').id)
    let pick: (path: string) => void = () => {}
    dialog.save.mockReturnValue(new Promise<string>((r) => (pick = r)))
    const first = f.saveTab(tab)
    expect(await f.saveTab(tab)).toBe(false)
    pick('/out/one.sql')
    expect(await first).toBe(true)
    expect(dialog.save).toHaveBeenCalledTimes(1)
    expect(Object.keys(written)).toEqual(['/out/one.sql'])
  })

  it('a MongoDB tab is offered .js, not .sql', async () => {
    const { f, tabs, live, written } = await setup()
    const { sessions } = await import('../stores/sessions.svelte')
    sessions.list = [session({ engine: 'mongodb' })]
    const tab = live(tabs.newQuery('s1', 'app', 'db.users.find()').id)
    dialog.save.mockResolvedValue('/out/find')
    await f.saveTabAs(tab)
    expect(dialog.save).toHaveBeenCalledWith({ title: 'Save to file', defaultPath: 'Query 1.js', filters: [{ name: 'Query', extensions: ['js', 'json', 'txt'] }] })
    expect(Object.keys(written)).toEqual(['/out/find.js'])
  })

  it('adds .sql when the chosen name has no extension', async () => {
    const { f, tabs, live, written } = await setup()
    const tab = live(tabs.newQuery('s1', 'app', 'x').id)
    dialog.save.mockResolvedValue('/out/mine')
    await f.saveTabAs(tab)
    expect(Object.keys(written)).toEqual(['/out/mine.sql'])
  })

  it('does nothing when the save dialog is cancelled', async () => {
    const { f, tabs, live, written } = await setup()
    const tab = live(tabs.newQuery('s1', 'app', 'x').id)
    dialog.save.mockResolvedValue(null)
    expect(await f.saveTabAs(tab)).toBe(false)
    expect(written).toEqual({})
    expect(tab.file).toBeUndefined()
  })

  it('"Save to file…" on a file tab moves it to the new file and offers the old name', async () => {
    const { f, live, written } = await setup({ '/q/a.sql': 'SELECT 1' })
    const tab = live((await f.openSqlPath('s1', 'app', '/q/a.sql'))!.id)
    dialog.save.mockResolvedValue('/q/copy.sql')
    await f.saveTabAs(tab)
    expect(dialog.save.mock.calls[0][0].defaultPath).toBe('a.sql')
    expect(written).toEqual({ '/q/copy.sql': 'SELECT 1' })
    expect(tab).toMatchObject({ file: '/q/copy.sql', title: 'copy.sql' })
  })
})

describe('SQL dump import', () => {
  it('is not offered for MongoDB or read-only connections', async () => {
    const { f } = await setup()
    expect(f.importBlockedReason(session())).toBeNull()
    expect(f.importBlockedReason(session({ engine: 'mongodb' }))).toMatch(/MongoDB/)
    expect(f.importBlockedReason(session({ readOnly: true }))).toMatch(/read only/)
  })

  it('picking a dump raises the import request for that session', async () => {
    const { f, editorRequests } = await setup()
    dialog.open.mockResolvedValue('/dumps/prod.sql')
    await f.pickSqlImport(session())
    expect(editorRequests.sqlImport).toEqual({ sessionId: 's1', path: '/dumps/prod.sql' })
    expect(dialog.open.mock.calls[0][0].title).toBe('Import SQL dump')
  })

  it('cancelling the picker raises nothing', async () => {
    const { f, editorRequests } = await setup()
    dialog.open.mockResolvedValue(null)
    await f.pickSqlImport(session())
    expect(editorRequests.sqlImport).toBeNull()
  })

  it('a blocked connection says why instead of opening the picker', async () => {
    const { f, editorRequests, toasts } = await setup()
    await f.pickSqlImport(session({ readOnly: true }))
    expect(dialog.open).not.toHaveBeenCalled()
    expect(editorRequests.sqlImport).toBeNull()
    expect(toasts()).toEqual(['error: This connection is read only.'])
  })
})
