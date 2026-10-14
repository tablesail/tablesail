import { test, expect } from './fixtures'
import { MOD, invokes, newQueryTab, openConnection, recordInvokes, setDialog, setSql, sqlText, tabNamed, tabTitles } from './helpers'
import type { Page } from '@playwright/test'

// Open / save .sql files in query tabs, and importing a dump. The mock backend keeps "files" in
// window.__TABLESAIL_MOCK_FILES__ and answers the native dialogs from window.__TABLESAIL_MOCK_DIALOG__.

const REPORT = '/home/me/sql/report.sql'
const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })
const toasts = (page: Page) => page.locator('.toasts')

async function seed(page: Page, files: Record<string, string>) {
  await page.evaluate((f) => {
    const w = window as unknown as { __TABLESAIL_MOCK_FILES__?: Record<string, string> }
    w.__TABLESAIL_MOCK_FILES__ = { ...(w.__TABLESAIL_MOCK_FILES__ ?? {}), ...f }
  }, files)
}

/** What the mock "disk" holds at `path` (null = no such file). */
const onDisk = (page: Page, path: string) =>
  page.evaluate((p) => (window as unknown as { __TABLESAIL_MOCK_FILES__?: Record<string, string> }).__TABLESAIL_MOCK_FILES__?.[p] ?? null, path)

async function runPaletteCommand(page: Page, text: string, option: string | RegExp = text) {
  await page.keyboard.press(`${MOD}+k`)
  await page.getByRole('combobox', { name: 'Search commands' }).fill(text)
  await page.getByRole('option', { name: option }).first().click()
}

/** Opens REPORT through ⌘O and waits for its tab. */
async function openReport(page: Page, content = 'SELECT * FROM customers;') {
  await seed(page, { [REPORT]: content })
  await setDialog(page, { open: REPORT })
  await page.keyboard.press(`${MOD}+o`)
  await expect(tabNamed(page, 'report.sql')).toBeVisible()
}

const dot = (page: Page) => page.locator('.strip .pending')

test.describe('open a .sql file', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('⌘O opens the picked file in a new tab named after it, with its text in the editor', async ({ page }) => {
    await openReport(page, 'SELECT 1;\nSELECT 2;')
    await expect(tabNamed(page, 'report.sql')).toHaveAttribute('aria-selected', 'true')
    await expect(tabNamed(page, 'report.sql')).toHaveAttribute('title', REPORT)
    expect(await sqlText(page)).toBe('SELECT 1;\nSELECT 2;')
    expect(await tabTitles(page)).toEqual(['report.sql'])
    await expect(dot(page)).toHaveCount(0)
    await expect(page.locator('.tab-body:not([hidden]) .toolbar .file')).toHaveText('report.sql')
  })

  test('asks the native dialog for .sql and .txt files', async ({ page }) => {
    await recordInvokes(page)
    await seed(page, { [REPORT]: 'SELECT 1;' })
    await setDialog(page, { open: REPORT })
    await page.keyboard.press(`${MOD}+o`)
    await expect(tabNamed(page, 'report.sql')).toBeVisible()
    const [call] = await invokes(page, 'plugin:dialog|open')
    expect(JSON.stringify(call.args)).toContain('"sql"')
    expect(JSON.stringify(call.args)).toContain('"txt"')
  })

  test('the command palette has "Open SQL file…" too', async ({ page }) => {
    await seed(page, { [REPORT]: 'SELECT 7;' })
    await setDialog(page, { open: REPORT })
    await runPaletteCommand(page, 'open sql', /Open SQL file/)
    await expect(tabNamed(page, 'report.sql')).toBeVisible()
    expect(await sqlText(page)).toBe('SELECT 7;')
  })

  test('so does the toolbar menu', async ({ page }) => {
    await seed(page, { [REPORT]: 'SELECT 8;' })
    await setDialog(page, { open: REPORT })
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Open SQL file/ }).click()
    await expect(tabNamed(page, 'report.sql')).toBeVisible()
  })

  test('cancelling the picker opens nothing', async ({ page }) => {
    await setDialog(page, { open: null })
    await page.keyboard.press(`${MOD}+o`)
    await page.waitForTimeout(300)
    expect(await tabTitles(page)).toEqual([])
    await expect(toasts(page).getByRole('status')).toHaveCount(0)
  })

  test('opening the same file again selects its tab instead of opening a second one', async ({ page }) => {
    await openReport(page)
    await newQueryTab(page)
    await page.keyboard.press(`${MOD}+o`)
    await expect(tabNamed(page, 'report.sql')).toHaveAttribute('aria-selected', 'true')
    expect(await tabTitles(page)).toEqual(['report.sql', 'Query 1'])
  })

  test('a file that cannot be read shows an error and opens no tab', async ({ page }) => {
    await setDialog(page, { open: '/home/me/sql/gone.sql' })
    await page.keyboard.press(`${MOD}+o`)
    await expect(toasts(page)).toContainText('Could not open gone.sql')
    expect(await tabTitles(page)).toEqual([])
  })

  test('a file opened in a Query tab does not use up the Query numbers', async ({ page }) => {
    await openReport(page)
    await newQueryTab(page)
    expect(await tabTitles(page)).toEqual(['report.sql', 'Query 1'])
  })
})

test.describe('save a .sql file', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('editing a file tab shows the unsaved dot; ⌘S writes the file and clears it', async ({ page }) => {
    await openReport(page)
    await setSql(page, 'SELECT 42;')
    await expect(dot(page)).toHaveCount(1)
    await expect(tabNamed(page, 'report.sql')).toHaveAttribute('title', `${REPORT} (unsaved changes)`)
    expect(await onDisk(page, REPORT)).toBe('SELECT * FROM customers;')

    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(() => onDisk(page, REPORT)).toBe('SELECT 42;')
    await expect(dot(page)).toHaveCount(0)
    await expect(toasts(page)).toContainText('Saved report.sql')
    await expect(tabNamed(page, 'report.sql')).toHaveAttribute('title', REPORT)
  })

  test('typing the text back to what is on disk clears the dot by itself', async ({ page }) => {
    await openReport(page, 'SELECT 1;')
    await setSql(page, 'SELECT 2;')
    await expect(dot(page)).toHaveCount(1)
    await setSql(page, 'SELECT 1;')
    await expect(dot(page)).toHaveCount(0)
  })

  test('⌘S on a query tab without a file asks for a path, writes the text there and the tab becomes that file', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT 3;')
    await recordInvokes(page)
    await setDialog(page, { save: '/home/me/out/first.sql' })
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(() => onDisk(page, '/home/me/out/first.sql')).toBe('SELECT 3;')
    await expect(tabNamed(page, 'first.sql')).toBeVisible()
    expect(await invokes(page, 'plugin:dialog|save')).toHaveLength(1)
    await expect(dot(page)).toHaveCount(0)
    // from now on ⌘S just writes the file
    await setSql(page, 'SELECT 4;')
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(() => onDisk(page, '/home/me/out/first.sql')).toBe('SELECT 4;')
    expect(await invokes(page, 'plugin:dialog|save')).toHaveLength(1)
  })

  test('cancelling the path dialog on ⌘S writes nothing and keeps the tab as it was', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT 5;')
    await recordInvokes(page)
    await setDialog(page, { save: null })
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(async () => (await invokes(page, 'plugin:dialog|save')).length).toBe(1)
    expect(await invokes(page, 'file_write_text')).toHaveLength(0)
    expect(await tabTitles(page)).toEqual(['Query 1'])
  })

  test('a CRLF file keeps its line endings when saved, and opens clean', async ({ page }) => {
    await openReport(page, 'SELECT 1;\r\nSELECT 2;\r\n')
    expect(await sqlText(page)).toBe('SELECT 1;\nSELECT 2;\n')
    await expect(dot(page)).toHaveCount(0)
    await setSql(page, 'SELECT 1;\nSELECT 22;\n')
    await expect(dot(page)).toHaveCount(1)
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(() => onDisk(page, REPORT)).toBe('SELECT 1;\r\nSELECT 22;\r\n')
    await expect(dot(page)).toHaveCount(0)
  })

  test('⌘⇧S asks for a path (named after the tab), writes the text there and the tab becomes that file', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT 1 + 1;')
    await recordInvokes(page)
    await setDialog(page, { save: '/home/me/out/mine.sql' })
    await page.keyboard.press(`${MOD}+Shift+s`)
    await expect.poll(() => onDisk(page, '/home/me/out/mine.sql')).toBe('SELECT 1 + 1;')
    await expect(tabNamed(page, 'mine.sql')).toBeVisible()
    await expect(tabNamed(page, 'mine.sql')).toHaveAttribute('title', '/home/me/out/mine.sql')
    const [call] = await invokes(page, 'plugin:dialog|save')
    expect(JSON.stringify(call.args)).toContain('Query 1.sql')

    // From now on ⌘S saves to that file.
    await setSql(page, 'SELECT 3;')
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(() => onDisk(page, '/home/me/out/mine.sql')).toBe('SELECT 3;')
  })

  test('cancelling the save dialog writes nothing and keeps the tab as it was', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT 1;')
    await setDialog(page, { save: null })
    await page.keyboard.press(`${MOD}+Shift+s`)
    await page.waitForTimeout(300)
    expect(await tabTitles(page)).toEqual(['Query 1'])
    expect(await onDisk(page, '/home/me/out/mine.sql')).toBeNull()
  })

  test('the palette and the toolbar menu can save to a file too', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT 5;')
    await setDialog(page, { save: '/home/me/out/five.sql' })
    await runPaletteCommand(page, 'save to file')
    await expect.poll(() => onDisk(page, '/home/me/out/five.sql')).toBe('SELECT 5;')

    await newQueryTab(page)
    await setSql(page, 'SELECT 6;')
    await setDialog(page, { save: '/home/me/out/six.sql' })
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Save to file/ }).click()
    await expect.poll(() => onDisk(page, '/home/me/out/six.sql')).toBe('SELECT 6;')
  })

  test('a failing write keeps the edits unsaved and tells the user', async ({ page }) => {
    await openReport(page)
    await setSql(page, 'SELECT 9;')
    await page.evaluate(() => {
      const t = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__
      const orig = t.invoke
      t.invoke = (cmd, args) =>
        cmd === 'file_write_text' ? Promise.reject({ kind: 'io', message: 'Permission denied', code: null, statement: null }) : orig(cmd, args)
    })
    await page.keyboard.press(`${MOD}+s`)
    await expect(toasts(page)).toContainText('Could not save report.sql: Permission denied')
    await expect(dot(page)).toHaveCount(1)
  })
})

test.describe('closing a file tab with unsaved edits', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openReport(page)
    await setSql(page, 'SELECT 99;')
    await expect(dot(page)).toHaveCount(1)
  })

  test('⌘W asks; Cancel keeps the tab and its edits', async ({ page }) => {
    await page.keyboard.press(`${MOD}+w`)
    const dlg = dialog(page, 'Unsaved changes')
    await expect(dlg).toContainText('report.sql')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['report.sql'])
    expect(await sqlText(page)).toBe('SELECT 99;')
  })

  test('Don’t save closes the tab and leaves the file alone', async ({ page }) => {
    await page.keyboard.press(`${MOD}+w`)
    await dialog(page, 'Unsaved changes').getByRole('button', { name: 'Don’t save' }).click()
    await expect(tabNamed(page, 'report.sql')).toHaveCount(0)
    expect(await onDisk(page, REPORT)).toBe('SELECT * FROM customers;')
  })

  test('Save writes the file and closes the tab', async ({ page }) => {
    await page.keyboard.press(`${MOD}+w`)
    await dialog(page, 'Unsaved changes').getByRole('button', { name: 'Save', exact: true }).click()
    await expect(tabNamed(page, 'report.sql')).toHaveCount(0)
    expect(await onDisk(page, REPORT)).toBe('SELECT 99;')
  })

  test('the close button on the tab asks too', async ({ page }) => {
    await page.getByRole('button', { name: 'Close tab report.sql' }).click()
    await expect(dialog(page, 'Unsaved changes')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog(page, 'Unsaved changes')).toHaveCount(0)
    await expect(tabNamed(page, 'report.sql')).toBeVisible()
  })

  test('a saved file tab and a plain query tab close without asking', async ({ page }) => {
    await page.keyboard.press(`${MOD}+s`)
    await expect(dot(page)).toHaveCount(0)
    await page.keyboard.press(`${MOD}+w`)
    await expect(tabNamed(page, 'report.sql')).toHaveCount(0)
    await newQueryTab(page)
    await setSql(page, 'SELECT 1')
    await page.keyboard.press(`${MOD}+w`)
    await expect(dialog(page, 'Unsaved changes')).toHaveCount(0)
    expect(await tabTitles(page)).toEqual([])
  })
})

test('a file tab with unsaved edits comes back unsaved after a restart', async ({ page }) => {
  await openConnection(page)
  await openReport(page)
  await setSql(page, 'SELECT 123;')
  await expect(dot(page)).toHaveCount(1)
  await page.waitForTimeout(600) // the open tabs are saved shortly after a change
  await page.reload()
  await openConnection(page)
  await expect(tabNamed(page, 'report.sql')).toBeVisible()
  await expect(dot(page)).toHaveCount(1)
  expect(await sqlText(page)).toBe('SELECT 123;')
})

test.describe('import a SQL dump', () => {
  const DUMP = '/home/me/dumps/shop.sql'
  const DUMP_SQL = 'CREATE TABLE widgets (id int);\nINSERT INTO widgets VALUES (1);\nINSERT INTO widgets VALUES (2);'

  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await seed(page, { [DUMP]: DUMP_SQL })
  })

  const importDialog = (page: Page) => dialog(page, 'Import SQL dump')

  test('palette → dialog with the target database and "Stop on first error" on → progress → done, sidebar refreshed', async ({ page }) => {
    await recordInvokes(page)
    await setDialog(page, { open: DUMP })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    const dlg = importDialog(page)
    await expect(dlg).toContainText('shop.sql')
    await expect(dlg.getByRole('checkbox', { name: 'Stop on first error' })).toBeChecked()
    await expect(dlg.getByRole('combobox', { name: 'Target database' })).toHaveValue('tablesail')

    const before = (await invokes(page, 'schema_snapshot')).length
    await dlg.getByRole('button', { name: 'Import', exact: true }).click()
    await expect(dlg).toContainText(/Executed .* statements/)
    await expect(dlg.getByRole('button', { name: 'Close' })).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Import', exact: true })).toBeDisabled()

    const [start] = await invokes(page, 'sql_import_start')
    expect(start.args).toMatchObject({ spec: { path: DUMP, database: 'tablesail', stopOnError: true } })
    // the schema is reloaded so new tables show up in the sidebar
    await expect.poll(async () => (await invokes(page, 'schema_snapshot')).length).toBeGreaterThan(before)
    expect(JSON.stringify((await invokes(page, 'schema_snapshot')).at(-1)?.args)).toContain('true')

    await dlg.getByRole('button', { name: 'Close' }).click()
    await expect(dlg).toHaveCount(0)
  })

  test('the statements show up in the query log', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await setDialog(page, { open: DUMP })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    await importDialog(page).getByRole('button', { name: 'Import', exact: true }).click()
    await expect(page.locator('.console .sql', { hasText: 'CREATE TABLE widgets' })).toBeVisible()
    await expect(page.locator('.console .sql', { hasText: 'INSERT INTO widgets VALUES (2)' })).toBeVisible()
  })

  test('"Stop on first error" can be turned off and is sent along', async ({ page }) => {
    await recordInvokes(page)
    await setDialog(page, { open: DUMP })
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Import SQL dump/ }).click()
    const dlg = importDialog(page)
    await dlg.getByRole('checkbox', { name: 'Stop on first error' }).uncheck()
    await expect(dlg).toContainText('Failing statements are skipped')
    await dlg.getByRole('button', { name: 'Import', exact: true }).click()
    await expect(dlg).toContainText(/Executed/)
    const [start] = await invokes(page, 'sql_import_start')
    expect(start.args).toMatchObject({ spec: { stopOnError: false } })
  })

  test('another database can be chosen', async ({ page }) => {
    await recordInvokes(page)
    await setDialog(page, { open: DUMP })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    const dlg = importDialog(page)
    // The database list loads after the dialog opens.
    await expect(dlg.getByRole('combobox', { name: 'Target database' }).locator('option').first()).toBeAttached()
    const options = await dlg.getByRole('combobox', { name: 'Target database' }).locator('option').allTextContents()
    const other = options.find((o) => o !== 'tablesail') ?? options[0]
    await dlg.getByRole('combobox', { name: 'Target database' }).selectOption(other)
    await dlg.getByRole('button', { name: 'Import', exact: true }).click()
    await expect(dlg).toContainText(/Executed/)
    const [start] = await invokes(page, 'sql_import_start')
    expect(start.args).toMatchObject({ spec: { database: other } })
  })

  test('an error from the backend is shown in the dialog and the import can be retried', async ({ page }) => {
    await seed(page, {})
    await setDialog(page, { open: '/home/me/dumps/missing.sql' })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    const dlg = importDialog(page)
    await dlg.getByRole('button', { name: 'Import', exact: true }).click()
    await expect(dlg.getByRole('alert')).toContainText('No such file')
    await expect(dlg.getByRole('button', { name: 'Import', exact: true })).toBeEnabled()
  })

  test('cancelling the file picker opens no dialog', async ({ page }) => {
    await setDialog(page, { open: null })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    await page.waitForTimeout(300)
    await expect(importDialog(page)).toHaveCount(0)
  })

  test('Cancel closes it without importing', async ({ page }) => {
    await recordInvokes(page)
    await setDialog(page, { open: DUMP })
    await runPaletteCommand(page, 'import sql', /Import SQL dump/)
    await importDialog(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(importDialog(page)).toHaveCount(0)
    expect(await invokes(page, 'sql_import_start')).toHaveLength(0)
  })
})

test.describe('dump import is not offered where it cannot work', () => {
  test('read-only connection: not in the palette, disabled in the menu', async ({ page }) => {
    await openConnection(page, 'Production')
    await page.keyboard.press(`${MOD}+k`)
    await page.getByRole('combobox', { name: 'Search commands' }).fill('import sql')
    await expect(page.getByText('No matching actions')).toBeVisible()
    await page.keyboard.press('Escape')
    await page.getByRole('button', { name: 'Menu' }).click()
    await expect(page.getByRole('menuitem', { name: /Import SQL dump/ })).toBeDisabled()
  })

  test('MongoDB: not in the palette, disabled in the menu', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await page.keyboard.press(`${MOD}+k`)
    await page.getByRole('combobox', { name: 'Search commands' }).fill('import sql')
    await expect(page.getByText('No matching actions')).toBeVisible()
    await page.keyboard.press('Escape')
    await page.getByRole('button', { name: 'Menu' }).click()
    await expect(page.getByRole('menuitem', { name: /Import SQL dump/ })).toBeDisabled()
  })
})
