import { test, expect } from './fixtures'
import {
  MOD,
  grid,
  gridCell,
  inspector,
  newQueryTab,
  openConnection,
  openTableLoaded,
  rowTexts,
  runStatement,
  setSql,
  sidebar,
  sqlEditor,
  sqlText,
  tabNamed,
  tabTitles,
} from './helpers'
import type { Page } from '@playwright/test'

const runButton = (page: Page) => page.getByRole('button', { name: /^Run query/ })
const runAll = (page: Page) => page.getByRole('button', { name: /Run all/ })
const stopButton = (page: Page) => page.getByRole('button', { name: /Stop/ })
const resultTab = (page: Page, name: string | RegExp) => page.getByRole('tab', { name })
const status = (page: Page) => page.locator('.tab-body:not([hidden]) .toolbar .status')
const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })

test.describe('editor basics', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  test('a new query tab shows the engine and environment, an empty editor and a hint', async ({ page }) => {
    await expect(page.getByText('SQL Editor', { exact: true })).toBeVisible()
    await expect(page.getByText('PostgreSQL · local', { exact: true })).toBeVisible()
    await expect(page.locator('.tab-body:not([hidden]) .cm-placeholder')).toContainText('Write SQL here')
    await expect(page.getByText('Run a query to see results here.')).toBeVisible()
    await expect(resultTab(page, 'Messages')).toBeVisible()
  })

  test('typing in the editor works and keeps its text', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT 1')
    expect(await sqlText(page)).toBe('SELECT 1')
  })

  test('typing inserts matching brackets and quotes like an editor should', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type("SELECT count(*) FROM t WHERE a = 'x'")
    expect(await sqlText(page)).toBe("SELECT count(*) FROM t WHERE a = 'x'")
  })

  test('the editor text survives switching to another tab and back', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await openTableLoaded(page, 'orders')
    await tabNamed(page, 'Query 1').click()
    expect(await sqlText(page)).toBe('SELECT * FROM customers')
  })

  test('each query tab has its own text', async ({ page }) => {
    await setSql(page, 'SELECT 1')
    await newQueryTab(page)
    expect(await sqlText(page)).toBe('')
    await setSql(page, 'SELECT 2')
    await tabNamed(page, 'Query 1').click()
    expect(await sqlText(page)).toBe('SELECT 1')
  })

  test('the database picker lists the databases of the connection', async ({ page }) => {
    const picker = page.locator('.tab-body:not([hidden]) .toolbar select')
    await expect(picker.locator('option')).toHaveText(['tablesoft', 'tablesoft_other', 'postgres'])
    await expect(picker).toHaveValue('tablesoft')
    await picker.selectOption('postgres')
    await expect(picker).toHaveValue('postgres')
  })

  test('the split between editor and results can be dragged', async ({ page }) => {
    const editorPane = page.locator('.tab-body:not([hidden]) .pane.editor')
    const before = (await editorPane.boundingBox())!.height
    const split = page.locator('.tab-body:not([hidden]) .split')
    const b = (await split.boundingBox())!
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2)
    await page.mouse.down()
    await page.mouse.move(b.x + b.width / 2, b.y + 150, { steps: 6 })
    await page.mouse.up()
    await expect.poll(async () => (await editorPane.boundingBox())!.height).toBeGreaterThan(before + 100)
  })
})

test.describe('running queries', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  test('⌘↩ runs the statement and shows a result grid with the rows', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(resultTab(page, 'Result 1')).toHaveAttribute('aria-selected', 'true')
    await expect(grid(page)).toBeVisible()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    expect(await rowTexts(page, 1)).toEqual(['2', 'b@example.com', 'Bob'])
    await expect(page.getByText('2 rows', { exact: true })).toBeVisible()
    await expect(status(page)).toHaveText(/^1 statement finished in /)
  })

  test('the Run query button does the same as the shortcut', async ({ page }) => {
    await setSql(page, 'SELECT * FROM orders')
    await runButton(page).click()
    await expect(gridCell(page, 0, 3)).toHaveText(/happy/)
    await expect(page.getByText('3 rows', { exact: true })).toBeVisible()
  })

  test('a running query shows progress and the Run button turns into Stop', async ({ page }) => {
    await setSql(page, 'SELECT pg_sleep(100)')
    await runStatement(page)
    await expect(stopButton(page)).toBeVisible()
    await expect(runButton(page)).toHaveCount(0)
    await expect(status(page)).toHaveText(/^Running…/)
    await expect(runAll(page)).toBeDisabled()
  })

  test('Stop cancels the running query and restores the Run button', async ({ page }) => {
    await setSql(page, 'SELECT pg_sleep(100)')
    await runStatement(page)
    await stopButton(page).click()
    await expect(runButton(page)).toBeVisible()
    await expect(stopButton(page)).toHaveCount(0)
    await expect(status(page)).toHaveText(/^1 statement cancelled in /)
    await expect(resultTab(page, 'Messages')).toHaveAttribute('aria-selected', 'true')
    await expect(page.locator('.state-cancelled')).toBeVisible()
    // The tab is usable again.
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('running again while one is in flight does not start a second run', async ({ page }) => {
    await setSql(page, 'SELECT pg_sleep(100)')
    await runStatement(page)
    await expect(stopButton(page)).toBeVisible()
    await runStatement(page)
    await runStatement(page)
    await expect(stopButton(page)).toHaveCount(1)
    await stopButton(page).click()
    await expect(runButton(page)).toBeVisible()
  })

  test('an error is shown in Messages with the server message and code, and the status says failed', async ({ page }) => {
    await setSql(page, 'SELECT * FROM nonexistent_table')
    await runStatement(page)
    await expect(resultTab(page, 'Messages')).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByText('relation "nonexistent_table" does not exist')).toBeVisible()
    await expect(page.getByText('(42P01)')).toBeVisible()
    await expect(page.locator('.state-error')).toBeVisible()
    await expect(status(page)).toHaveText(/^1 statement failed in /)
    await expect(resultTab(page, 'Messages').locator('.badge')).toHaveText('!')
  })

  test('an error does not leave the tab stuck: fix the query and run again', async ({ page }) => {
    await setSql(page, 'SELECT * FROM nonexistent_table')
    await runStatement(page)
    await expect(status(page)).toHaveText(/failed/)
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(resultTab(page, 'Result 1')).toHaveAttribute('aria-selected', 'true')
    await expect(status(page)).toHaveText(/^1 statement finished/)
    await expect(resultTab(page, 'Messages').locator('.badge')).toHaveCount(0)
  })

  test('⇧⌘↩ and Run all execute every statement and give one result tab each', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers;\nSELECT * FROM orders;\nSELECT * FROM events')
    await page.keyboard.press(`${MOD}+Shift+Enter`)
    await expect(resultTab(page, /^Result \d$/)).toHaveText(['Result 1', 'Result 2', 'Result 3'])
    await expect(status(page)).toHaveText(/^3 statements finished/)
    await expect(resultTab(page, 'Result 1')).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByText('2 rows', { exact: true })).toBeVisible()

    await resultTab(page, 'Result 2').click()
    await expect(resultTab(page, 'Result 2')).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByText('3 rows', { exact: true })).toBeVisible()
    await expect(gridCell(page, 0, 3)).toHaveText(/happy/)

    await resultTab(page, 'Messages').click()
    await expect(page.locator('.tab-body:not([hidden]) .msg')).toHaveCount(3)
    await expect(page.getByText('Statement 2', { exact: true })).toBeVisible()
  })

  test('Run all button works like the shortcut', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers; SELECT * FROM orders')
    await runAll(page).click()
    await expect(resultTab(page, /^Result \d$/)).toHaveCount(2)
  })

  test('⌘↩ runs only the statement under the cursor', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers;\nSELECT * FROM orders;')
    await page.keyboard.press(`${MOD}+End`) // cursor in the last statement
    await runStatement(page)
    await expect(status(page)).toHaveText(/^1 statement finished/)
    await expect(page.getByText('3 rows', { exact: true })).toBeVisible()
    await page.keyboard.press(`${MOD}+Home`)
    await runStatement(page)
    await expect(page.getByText('2 rows', { exact: true })).toBeVisible()
  })

  test('a selection runs instead of the whole statement', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers;\nSELECT * FROM orders;')
    await page.keyboard.press(`${MOD}+Home`)
    await page.keyboard.press('Shift+End')
    await runStatement(page)
    await expect(page.getByText('2 rows', { exact: true })).toBeVisible()
  })

  test('statements that return no rows report affected rows in Messages', async ({ page }) => {
    await setSql(page, "UPDATE customers SET name = 'x' WHERE id = 1")
    await runStatement(page)
    await expect(resultTab(page, 'Messages')).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByText('3 rows affected')).toBeVisible()
    await expect(page.getByText('Done', { exact: true })).toBeVisible()
  })

  test('selecting a result row shows it in the Details panel (read-only)', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await gridCell(page, 1, 2).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).locator('.field').filter({ hasText: 'email' })).toContainText('b@example.com')
    await expect(inspector(page).locator('input.value, textarea.value')).toHaveCount(0)
  })

  test('Export… on a result opens the export dialog for the query results', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await page.getByRole('button', { name: 'Export…' }).click()
    await expect(dialog(page, 'Export')).toBeVisible()
    await expect(dialog(page, 'Export')).toContainText('Query results')
  })

  test('a result with many rows scrolls and keeps the count', async ({ page }) => {
    await setSql(page, 'SELECT * FROM big_table')
    await runStatement(page)
    await expect(page.getByText('5,000 rows', { exact: true })).toBeVisible()
    await gridCell(page, 0, 0).click()
    await page.keyboard.press(`${MOD}+ArrowDown`)
    await expect(grid(page).locator('.cell', { hasText: /^5000$/ })).toBeVisible()
  })
})

test.describe('format', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  test('Format uppercases keywords and spreads the statement over lines', async ({ page }) => {
    await setSql(page, 'select id,name from customers where id=1 order by name')
    await page.getByRole('button', { name: 'Format' }).click()
    await expect.poll(() => sqlText(page)).toContain('SELECT')
    const text = await sqlText(page)
    expect(text).toContain('FROM')
    expect(text).toContain('WHERE')
    expect(text).toContain('ORDER BY')
    expect(text.split('\n').length).toBeGreaterThan(3)
  })

  test('Format only touches the selection when there is one', async ({ page }) => {
    await setSql(page, 'select 1;\nselect 2')
    await page.keyboard.press(`${MOD}+Home`)
    await page.keyboard.press('Shift+End')
    await page.getByRole('button', { name: 'Format' }).click()
    await expect.poll(() => sqlText(page)).toMatch(/^SELECT/)
    expect(await sqlText(page)).toContain('select 2')
  })

  test('Format on an empty editor does nothing and does not crash', async ({ page }) => {
    await page.getByRole('button', { name: 'Format' }).click()
    expect(await sqlText(page)).toBe('')
  })

  test('Format is not offered for MongoDB', async ({ page }) => {
    await page.getByRole('button', { name: /Connections/ }).first().click()
    await page.getByRole('option', { name: /^Mongo\b/ }).getByRole('button', { name: 'Open', exact: true }).click()
    await expect(page.getByRole('button', { name: /MongoDB 8\.2\.1/ })).toBeVisible()
    await page.getByRole('button', { name: 'New SQL query' }).click()
    await expect(sqlEditor(page)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Format' })).toHaveCount(0)
  })
})

test.describe('saved queries', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  test('Save asks for a name, adds the query to the Queries tab and renames the editor tab', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await page.getByRole('button', { name: 'Save' }).click()
    const dlg = dialog(page, 'Save query')
    await expect(dlg).toBeVisible()
    await dlg.getByPlaceholder('Revenue by month').fill('All customers')
    await dlg.getByRole('button', { name: 'Save' }).click()

    await expect(dlg).toHaveCount(0)
    await expect(page.getByText('Saved “All customers”')).toBeVisible()
    await expect(tabNamed(page, 'All customers')).toBeVisible()

    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    const item = sidebar(page).getByRole('button', { name: /All customers/ })
    await expect(item).toBeVisible()
    await expect(item).toContainText('SELECT * FROM customers')
  })

  test('the name box has focus when the Save dialog opens', async ({ page }) => {
    // BUG: Svelte's `autofocus` attribute only focuses when document.activeElement is <body>; here the Save
    // toolbar button still has focus, so the input stays unfocused and typing a name goes nowhere
    // (QueryTab.svelte save dialog; the same applies to every Modal opened from a focused control).
    await setSql(page, 'SELECT 1')
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'Save query').getByPlaceholder('Revenue by month')).toBeFocused({ timeout: 1500 })
  })

  test('Enter in the name box saves', async ({ page }) => {
    await setSql(page, 'SELECT 1')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByPlaceholder('Revenue by month').fill('One')
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Save query')).toHaveCount(0)
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await expect(sidebar(page).getByRole('button', { name: /One/ })).toBeVisible()
  })

  test('Cancel and Escape discard the dialog without saving', async ({ page }) => {
    await setSql(page, 'SELECT 1')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByRole('button', { name: 'Cancel' }).click()
    await page.getByRole('button', { name: 'Save' }).click()
    await page.keyboard.press('Escape')
    await expect(dialog(page, 'Save query')).toHaveCount(0)
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await expect(sidebar(page).getByText('Save a query from the SQL editor to keep it here.')).toBeVisible()
  })

  test('saving an empty editor does nothing', async ({ page }) => {
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'Save query')).toHaveCount(0)
  })

  test('a blank name falls back to a timestamped one', async ({ page }) => {
    await setSql(page, 'SELECT 1')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByRole('button', { name: 'Save' }).click()
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await expect(sidebar(page).getByRole('button').filter({ hasText: /^Query / }).first()).toBeVisible()
  })

  test('opening a saved query opens a new tab with its SQL; the saved query stays', async ({ page }) => {
    await setSql(page, 'SELECT * FROM orders')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByPlaceholder('Revenue by month').fill('Orders')
    await page.keyboard.press('Enter')
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await sidebar(page).getByRole('button', { name: /Orders/ }).click()
    expect(await tabTitles(page)).toHaveLength(2)
    expect(await sqlText(page)).toBe('SELECT * FROM orders')
  })

  test('the saved-query menu offers Open, Copy SQL and Delete', async ({ page }) => {
    await setSql(page, 'SELECT * FROM orders')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByPlaceholder('Revenue by month').fill('Orders')
    await page.keyboard.press('Enter')
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await sidebar(page).getByRole('button', { name: /Orders/ }).click({ button: 'right' })
    await expect(page.getByRole('menuitem')).toHaveText(['Open', 'Copy SQL', 'Delete'])
    await page.getByRole('menuitem', { name: 'Copy SQL' }).click()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('SELECT * FROM orders')

    await sidebar(page).getByRole('button', { name: /Orders/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    await expect(sidebar(page).getByText('Save a query from the SQL editor to keep it here.')).toBeVisible()
  })

  test('saved queries are searchable and survive a reload', async ({ page }) => {
    for (const [name, sql] of [
      ['Customers', 'SELECT * FROM customers'],
      ['Orders', 'SELECT * FROM orders'],
    ]) {
      await setSql(page, sql)
      await page.getByRole('button', { name: 'Save' }).click()
      await dialog(page, 'Save query').getByPlaceholder('Revenue by month').fill(name)
      await page.keyboard.press('Enter')
      await newQueryTab(page)
    }
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await sidebar(page).getByPlaceholder('Search queries…').fill('ord')
    await expect(sidebar(page).getByRole('button', { name: /Orders/ })).toBeVisible()
    await expect(sidebar(page).getByRole('button', { name: /Customers/ })).toHaveCount(0)

    await page.reload()
    await openConnection(page)
    await sidebar(page).getByRole('tab', { name: 'Queries' }).click()
    await expect(sidebar(page).getByRole('button', { name: /Customers/ })).toBeVisible()
  })
})

test.describe('history', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  test('every run is recorded with the connection and timing; clicking one reopens it', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await sidebar(page).getByRole('tab', { name: 'History' }).click()
    const item = sidebar(page).getByRole('button', { name: /SELECT \* FROM customers/ })
    await expect(item).toBeVisible()
    await expect(item).toContainText('Local Postgres')
    await expect(item).toContainText(/just now|\dm ago/)
    await expect(item).toContainText(/\d+ ms/)
    await expect(sidebar(page).getByText('1 queries')).toBeVisible()

    await item.click()
    expect(await tabTitles(page)).toHaveLength(2)
    expect(await sqlText(page)).toBe('SELECT * FROM customers')
  })

  test('failed runs are flagged and newest comes first', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await setSql(page, 'SELECT * FROM nonexistent_table')
    await runStatement(page)
    await expect(status(page)).toHaveText(/failed/)
    await sidebar(page).getByRole('tab', { name: 'History' }).click()
    const items = sidebar(page).locator('.item')
    await expect(items).toHaveCount(2)
    await expect(items.nth(0)).toContainText('nonexistent_table')
    await expect(items.nth(0)).toContainText('failed')
    await expect(items.nth(1)).not.toContainText('failed')
  })

  test('history is searchable and can be cleared', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await setSql(page, 'SELECT * FROM orders')
    await runStatement(page)
    await expect(page.getByText('3 rows', { exact: true })).toBeVisible()
    await sidebar(page).getByRole('tab', { name: 'History' }).click()
    await expect(sidebar(page).locator('.item')).toHaveCount(2)
    await sidebar(page).getByPlaceholder('Search history…').fill('orders')
    await expect(sidebar(page).locator('.item')).toHaveCount(1)
    await sidebar(page).getByPlaceholder('Search history…').fill('zzz')
    await expect(sidebar(page).getByText('No matching queries.')).toBeVisible()
    await sidebar(page).getByPlaceholder('Search history…').fill('')
    await sidebar(page).getByRole('button', { name: 'Clear', exact: true }).click()
    await expect(sidebar(page).getByText('Queries you run appear here.')).toBeVisible()
  })

  test('history survives a reload', async ({ page }) => {
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await page.reload()
    await openConnection(page)
    await sidebar(page).getByRole('tab', { name: 'History' }).click()
    await expect(sidebar(page).getByRole('button', { name: /SELECT \* FROM customers/ })).toBeVisible()
  })

  test('table browsing does not pollute the history (only editor runs do)', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await sidebar(page).getByRole('tab', { name: 'History' }).click()
    await expect(sidebar(page).getByText('Queries you run appear here.')).toBeVisible()
  })
})

test.describe('autocomplete', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
  })

  const popup = (page: Page) => page.locator('.cm-tooltip-autocomplete')

  test('typing a table prefix after FROM offers matching tables', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT * FROM cust')
    await expect(popup(page)).toBeVisible()
    await expect(popup(page).getByRole('option', { name: /customers/ })).toBeVisible()
    await expect(popup(page).getByRole('option', { name: /customer_totals/ })).toBeVisible()
  })

  test('Enter accepts the highlighted suggestion', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT * FROM orde')
    await expect(popup(page).getByRole('option', { name: /orders/ })).toBeVisible()
    await page.waitForTimeout(150) // CodeMirror ignores Enter/arrows for its 75ms interactionDelay
    await page.keyboard.press('Enter')
    await expect(popup(page)).toHaveCount(0)
    expect(await sqlText(page)).toMatch(/FROM orders$/)
  })

  test('Escape closes the popup without changing the text', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT * FROM cust')
    await expect(popup(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(popup(page)).toHaveCount(0)
    expect(await sqlText(page)).toBe('SELECT * FROM cust')
  })

  test('arrow keys move through the suggestions', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT * FROM cust')
    await expect(popup(page).getByRole('option').first()).toHaveAttribute('aria-selected', 'true')
    // CodeMirror ignores navigation keys for its interactionDelay (75ms) after the popup opens.
    await page.waitForTimeout(150)
    await page.keyboard.press('ArrowDown')
    await expect(popup(page).getByRole('option').nth(1)).toHaveAttribute('aria-selected', 'true')
  })

  test('keywords are suggested too, and columns after a table qualifier', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SEL')
    await expect(popup(page).getByRole('option', { name: 'SELECT', exact: true })).toBeVisible()
    await page.keyboard.press('Escape')
    await page.keyboard.press(`${MOD}+a`)
    await page.keyboard.type('SELECT customers.')
    await expect(popup(page).getByRole('option', { name: /email/ })).toBeVisible()
  })

  test('suggestions do not appear for text that matches nothing', async ({ page }) => {
    await sqlEditor(page).click()
    await page.keyboard.type('SELECT * FROM qqqqzzzz')
    await expect(popup(page)).toHaveCount(0)
  })
})

test.describe('production safety', () => {
  test('a destructive statement on a production connection asks before running', async ({ page }) => {
    await openConnection(page, 'Production')
    await newQueryTab(page)
    await setSql(page, 'DROP TABLE customers')
    await runStatement(page)
    const dlg = dialog(page, 'Run on production?')
    await expect(dlg).toBeVisible()
    await expect(dlg).toContainText('Production')
    await expect(dlg).toContainText('DROP TABLE customers')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    await expect(status(page)).toHaveText('')

    await sqlEditor(page).click() // focus is not restored to the editor after the dialog closes (see below)
    await runStatement(page)
    await dialog(page, 'Run on production?').getByRole('button', { name: 'Run on production' }).click()
    await expect(status(page)).toHaveText(/^1 statement finished/)
  })

  test('closing the production prompt gives the focus back to the editor', async ({ page }) => {
    // BUG: Modal.svelte neither traps nor restores focus. After Cancel the focus is on <body>, so the next
    // ⌘↩ is swallowed and the user has to click into the editor again.
    await openConnection(page, 'Production')
    await newQueryTab(page)
    await setSql(page, 'DROP TABLE customers')
    await runStatement(page)
    await dialog(page, 'Run on production?').getByRole('button', { name: 'Cancel' }).click()
    await expect(sqlEditor(page)).toBeFocused({ timeout: 1500 })
  })

  test('plain reads on production run without a prompt', async ({ page }) => {
    await openConnection(page, 'Production')
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(dialog(page, 'Run on production?')).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('the toolbar shows "Read only" for a read-only connection', async ({ page }) => {
    await openConnection(page, 'Production')
    await newQueryTab(page)
    await expect(page.getByText('PostgreSQL · Read only', { exact: true })).toBeVisible()
  })

  test('a local connection never prompts, even for DROP', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await newQueryTab(page)
    await setSql(page, 'DROP TABLE customers')
    await runStatement(page)
    await expect(dialog(page, 'Run on production?')).toHaveCount(0)
    await expect(status(page)).toHaveText(/^1 statement finished/)
  })
})
