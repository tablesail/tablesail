import { test, expect } from './fixtures'
import { bottomBar, invokes, openConnection, openTableLoaded, recordInvokes, setDialog, tableRow } from './helpers'
import type { Page } from '@playwright/test'

const dialog = (page: Page, name: string | RegExp) => page.getByRole('dialog', { name })
const exportButton = (page: Page) => bottomBar(page).getByRole('button', { name: 'Export' })
const importButton = (page: Page) => bottomBar(page).getByRole('button', { name: 'Import CSV' })

test.describe('export dialog', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await recordInvokes(page)
  })

  const dlg = (page: Page) => dialog(page, 'Export')

  test('opens from the bottom bar and describes what is exported', async ({ page }) => {
    await exportButton(page).click()
    await expect(dlg(page)).toBeVisible()
    await expect(dlg(page)).toContainText('Table public.customers')
    await expect(dlg(page).getByLabel('Format')).toHaveValue('csv')
    await expect(dlg(page).getByRole('button', { name: 'Export', exact: true })).toBeEnabled()
    await expect(dlg(page).getByRole('button', { name: 'Cancel' })).toBeVisible()
  })

  test('also opens from the table context menu in the sidebar', async ({ page }) => {
    await tableRow(page, 'orders').click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Export…' }).click()
    await expect(dlg(page)).toContainText('Table public.orders')
  })

  test('each format shows its own options', async ({ page }) => {
    await exportButton(page).click()
    const format = dlg(page).getByLabel('Format')
    await expect(format.locator('option')).toHaveText(['CSV', 'JSON', 'JSON Lines', 'SQL INSERT'])

    // CSV
    await expect(dlg(page).getByLabel('Delimiter')).toHaveValue(',')
    await expect(dlg(page).getByLabel('Delimiter').locator('option')).toHaveText(['Comma (,)', 'Semicolon (;)', 'Tab', 'Pipe (|)'])
    await expect(dlg(page).getByLabel('Header row')).toBeChecked()
    await expect(dlg(page).getByPlaceholder('(empty)')).toBeVisible() // NULL as

    await format.selectOption('json')
    await expect(dlg(page).getByLabel('Pretty-print')).toBeChecked()
    await expect(dlg(page).getByLabel('Delimiter')).toHaveCount(0)

    await format.selectOption('jsonLines')
    await expect(dlg(page).getByLabel('Pretty-print')).toHaveCount(0)
    await expect(dlg(page).getByLabel('Header row')).toHaveCount(0)

    await format.selectOption('sql')
    await expect(dlg(page).getByLabel('Table name')).toHaveValue('customers')
    await expect(dlg(page).getByLabel('Rows per INSERT')).toHaveValue('100')
  })

  test('the file box shows a default name with the right extension for each format', async ({ page }) => {
    await exportButton(page).click()
    const file = dlg(page).locator('.file input')
    await expect(file).toHaveAttribute('placeholder', 'customers.csv')
    await dlg(page).getByLabel('Format').selectOption('json')
    await expect(file).toHaveAttribute('placeholder', 'customers.json')
    await dlg(page).getByLabel('Format').selectOption('jsonLines')
    await expect(file).toHaveAttribute('placeholder', 'customers.jsonl')
    await dlg(page).getByLabel('Format').selectOption('sql')
    await expect(file).toHaveAttribute('placeholder', 'customers.sql')
  })

  test('Browse… fills the path from the save dialog', async ({ page }) => {
    await setDialog(page, { save: '/tmp/people.csv' })
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Browse…' }).click()
    await expect(dlg(page).locator('.file input')).toHaveValue('/tmp/people.csv')
  })

  test('Export asks where to save, runs the job and reports the result', async ({ page }) => {
    await setDialog(page, { save: '/tmp/customers.csv' })
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page).getByText(/^Exported 1,000 rows/)).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Close' })).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Export again' })).toBeVisible()

    const [call] = await invokes(page, 'export_start')
    expect(call.args.path).toBe('/tmp/customers.csv')
    expect(call.args.format).toEqual({ kind: 'csv', delimiter: ',', header: true, nullAs: '' })
    expect((call.args.source as { kind: string; request: { table: { name: string } } }).kind).toBe('table')
    expect((call.args.source as { request: { table: { name: string } } }).request.table.name).toBe('customers')
  })

  test('the chosen options are what the backend receives', async ({ page }) => {
    await setDialog(page, { save: '/tmp/out.csv' })
    await exportButton(page).click()
    await dlg(page).getByLabel('Delimiter').selectOption(';')
    await dlg(page).getByLabel('Header row').uncheck()
    await dlg(page).getByPlaceholder('(empty)').fill('NULL')
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page).getByText(/^Exported/)).toBeVisible()
    const [call] = await invokes(page, 'export_start')
    expect(call.args.format).toEqual({ kind: 'csv', delimiter: ';', header: false, nullAs: 'NULL' })
  })

  test('SQL INSERT export sends the table name and batch size', async ({ page }) => {
    await setDialog(page, { save: '/tmp/out.sql' })
    await exportButton(page).click()
    await dlg(page).getByLabel('Format').selectOption('sql')
    await dlg(page).getByLabel('Table name').fill('backup_customers')
    await dlg(page).getByLabel('Rows per INSERT').fill('25')
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page).getByText(/^Exported/)).toBeVisible()
    const [call] = await invokes(page, 'export_start')
    expect(call.args.format).toEqual({ kind: 'sql', table: 'backup_customers', rowsPerInsert: 25 })
    expect(call.args.path).toBe('/tmp/out.sql')
  })

  test('JSON export honours pretty-print', async ({ page }) => {
    await setDialog(page, { save: '/tmp/out.json' })
    await exportButton(page).click()
    await dlg(page).getByLabel('Format').selectOption('json')
    await dlg(page).getByLabel('Pretty-print').uncheck()
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page).getByText(/^Exported/)).toBeVisible()
    const [call] = await invokes(page, 'export_start')
    expect(call.args.format).toEqual({ kind: 'json', pretty: false })
  })

  test('a typed path is used without opening the save dialog', async ({ page }) => {
    await exportButton(page).click()
    await dlg(page).locator('.file input').fill('/tmp/typed.csv')
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page).getByText(/^Exported/)).toBeVisible()
    expect(await invokes(page, 'export_start')).toHaveLength(1)
    expect((await invokes(page, 'export_start'))[0].args.path).toBe('/tmp/typed.csv')
  })

  test('cancelling the save dialog starts nothing', async ({ page }) => {
    await setDialog(page, { save: null })
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await expect(dlg(page)).toBeVisible()
    expect(await invokes(page, 'export_start')).toHaveLength(0)
    await expect(dlg(page).locator('.job')).toHaveCount(0)
  })

  test('Export again re-runs the job', async ({ page }) => {
    await setDialog(page, { save: '/tmp/a.csv' })
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Export', exact: true }).click()
    await dlg(page).getByRole('button', { name: 'Export again' }).click()
    await expect.poll(async () => (await invokes(page, 'export_start')).length).toBe(2)
  })

  test('changing the format after a path was chosen clears the path', async ({ page }) => {
    await setDialog(page, { save: '/tmp/a.csv' })
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Browse…' }).click()
    await expect(dlg(page).locator('.file input')).toHaveValue('/tmp/a.csv')
    await dlg(page).getByLabel('Format').selectOption('json')
    await expect(dlg(page).locator('.file input')).toHaveValue('')
  })

  test('Cancel, Escape and the backdrop close it', async ({ page }) => {
    await exportButton(page).click()
    await dlg(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg(page)).toHaveCount(0)
    await exportButton(page).click()
    await page.keyboard.press('Escape')
    await expect(dlg(page)).toHaveCount(0)
    await exportButton(page).click()
    await page.mouse.click(5, 5)
    await expect(dlg(page)).toHaveCount(0)
  })

  test('the export button is available on every kind of table, including read-only connections', async ({ page }) => {
    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await page.getByRole('option', { name: /^Production/ }).getByRole('button', { name: 'Open', exact: true }).click()
    await openTableLoaded(page, 'customers')
    await expect(exportButton(page)).toBeEnabled()
  })
})

test.describe('import dialog', () => {
  const dlg = (page: Page) => dialog(page, /^Import CSV into /)
  const mapping = (page: Page) => dlg(page).locator('.map select')

  test('is enabled for tables on writable connections and opens a dialog titled with the table', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await expect(importButton(page)).toBeEnabled()
    await importButton(page).click()
    await expect(dialog(page, 'Import CSV into customers')).toBeVisible()
    await expect(dlg(page).getByPlaceholder('No file chosen')).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Choose CSV…' })).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Import', exact: true })).toBeDisabled()
  })

  test('is disabled on read-only connections, views and tables that cannot be edited', async ({ page }) => {
    await openConnection(page, 'Production')
    await openTableLoaded(page, 'customers')
    await expect(importButton(page)).toBeDisabled()

    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await page.getByRole('option', { name: /^Local Postgres/ }).getByRole('button', { name: 'Open', exact: true }).click()
    await openTableLoaded(page, 'customer_totals')
    await expect(importButton(page)).toBeDisabled()
    await tableRow(page, 'customer_totals').click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: 'Import CSV…' })).toBeDisabled()
    await page.keyboard.press('Escape')

    await openTableLoaded(page, 'no_pk')
    await expect(importButton(page)).toBeDisabled()
  })

  test('also opens from the sidebar context menu', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'orders').click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Import CSV…' }).click()
    await expect(dialog(page, 'Import CSV into orders')).toBeVisible()
  })

  test('choosing a file previews it and auto-matches the columns by name', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()

    await expect(dlg(page).getByPlaceholder('No file chosen')).toHaveValue('people.csv')
    await expect(dlg(page).locator('.map th .csvname')).toHaveText(['id', 'name'])
    await expect(mapping(page)).toHaveCount(2)
    await expect(mapping(page).nth(0)).toHaveValue('id')
    await expect(mapping(page).nth(1)).toHaveValue('name')
    await expect(dlg(page).getByText('Previewing 2 rows · 2 of 2 columns mapped')).toBeVisible()
    await expect(dlg(page).locator('.map tbody tr')).toHaveCount(2)
    await expect(dlg(page).getByRole('button', { name: 'Import', exact: true })).toBeEnabled()
    // Generated/auto columns would be skipped; here every CSV column maps to a real one.
    await expect(mapping(page).nth(1).locator('option')).toHaveText(['(skip)', 'id', 'email', 'name'])
  })

  test('changing the mapping updates the summary; with nothing mapped Import is disabled', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(mapping(page)).toHaveCount(2)

    await mapping(page).nth(0).selectOption('')
    await expect(dlg(page).getByText('Previewing 2 rows · 1 of 2 columns mapped')).toBeVisible()
    await mapping(page).nth(1).selectOption('')
    await expect(dlg(page).getByText('Previewing 2 rows · 0 of 2 columns mapped')).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Import', exact: true })).toBeDisabled()

    await dlg(page).getByRole('button', { name: 'Auto-match columns' }).click()
    await expect(dlg(page).getByText('Previewing 2 rows · 2 of 2 columns mapped')).toBeVisible()
  })

  test('unchecking "First row is a header" treats it as data and names columns generically', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(mapping(page)).toHaveCount(2)
    await dlg(page).getByLabel('First row is a header').uncheck()
    await expect(dlg(page).locator('.map th .csvname')).toHaveText(['Column 1', 'Column 2'])
    await expect(dlg(page).locator('.map tbody tr')).toHaveCount(3)
    // Without headers columns map by position.
    await expect(mapping(page).nth(0)).toHaveValue('id')
    await expect(mapping(page).nth(1)).toHaveValue('email')
  })

  test('Import runs the job with the mapping and reports the result; then it cannot be run twice by accident', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await recordInvokes(page)
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(mapping(page)).toHaveCount(2)
    await mapping(page).nth(1).selectOption('email')
    await dlg(page).getByLabel('NULL text').fill('\\N')
    await dlg(page).getByRole('button', { name: 'Import', exact: true }).click()

    await expect(dlg(page).getByText(/^Imported 1,000 rows/)).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Close' })).toBeVisible()
    await expect(dlg(page).getByRole('button', { name: 'Import', exact: true })).toBeDisabled()

    const [call] = await invokes(page, 'import_start')
    const spec = call.args.import as { path: string; table: { name: string }; delimiter: string; hasHeader: boolean; mapping: (string | null)[]; nullAs: string | null }
    expect(spec.path).toBe('/tmp/people.csv')
    expect(spec.table.name).toBe('customers')
    expect(spec.hasHeader).toBe(true)
    expect(spec.mapping).toEqual(['id', 'email'])
    expect(spec.nullAs).toBe('\\N')
    expect(spec.delimiter).toBe(',')
  })

  test('changing the delimiter reloads the preview', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await recordInvokes(page)
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(mapping(page)).toHaveCount(2)
    await dlg(page).getByLabel('Delimiter').selectOption(';')
    await expect.poll(async () => (await invokes(page, 'csv_preview')).map((c) => c.args.delimiter)).toEqual([null, ';'])
  })

  test('cancelling the file dialog leaves the dialog empty', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await setDialog(page, { open: null })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(dlg(page).getByPlaceholder('No file chosen')).toHaveValue('')
    await expect(dlg(page).locator('.map')).toHaveCount(0)
  })

  test('Cancel, Escape and the backdrop close it', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg(page)).toHaveCount(0)
    await importButton(page).click()
    await page.keyboard.press('Escape')
    await expect(dlg(page)).toHaveCount(0)
  })

  test('the column selects in the mapping table have an accessible name', async ({ page }) => {
    // BUG: each mapping <select> in ImportDialog sits in a table header next to a .csvname div with no
    // label/aria-label, so a screen reader announces N identical unnamed comboboxes.
    test.fail()
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await setDialog(page, { open: '/tmp/people.csv' })
    await importButton(page).click()
    await dlg(page).getByRole('button', { name: 'Choose CSV…' }).click()
    await expect(mapping(page)).toHaveCount(2)
    await expect(dlg(page).getByRole('combobox', { name: /id/ }).first()).toBeVisible({ timeout: 1500 })
  })
})
