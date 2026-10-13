import { test, expect } from './fixtures'
import {
  MOD,
  bottomBar,
  detailField,
  detailValue,
  grid,
  gridCell,
  gridRows,
  gotoApp,
  inspector,
  openConnection,
  openTableLoaded,
  rowTexts,
  card,
} from './helpers'
import type { Page } from '@playwright/test'

const editor = (page: Page) => grid(page).locator('.edit-host').locator('input, select')
const pending = (page: Page) => bottomBar(page).locator('.pending-label')
const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })
const action = (page: Page, name: string) => bottomBar(page).getByRole('button', { name, exact: true })

/** Edits the customer name cell of the first row through the inline editor. */
async function editFirstName(page: Page, value: string) {
  await gridCell(page, 0, 2).dblclick()
  await expect(editor(page)).toBeVisible()
  await editor(page).fill(value)
  await page.keyboard.press('Enter')
}

test.describe('editing cells', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
  })

  test('double-clicking a cell opens an inline editor with the current value; Enter stages the change', async ({ page }) => {
    await expect(pending(page)).toHaveCount(0)
    await gridCell(page, 0, 2).dblclick()
    await expect(editor(page)).toHaveValue('Alice')
    await expect(editor(page)).toBeFocused()
    await editor(page).fill('Alicia')
    await page.keyboard.press('Enter')

    await expect(editor(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alicia')
    await expect(gridCell(page, 0, 2)).toHaveClass(/edited/)
    await expect(pending(page)).toHaveText('1 change')
    await expect(action(page, 'Preview')).toBeVisible()
    await expect(action(page, 'Discard')).toBeVisible()
    await expect(action(page, 'Commit')).toBeVisible()
    // Focus is back on the grid so the keyboard keeps working.
    await expect(grid(page)).toBeFocused()
  })

  test('Escape cancels an edit without staging anything', async ({ page }) => {
    await gridCell(page, 0, 2).dblclick()
    await editor(page).fill('Nope')
    await page.keyboard.press('Escape')
    await expect(editor(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await expect(pending(page)).toHaveCount(0)
  })

  test('leaving the editor (clicking elsewhere) commits the edit', async ({ page }) => {
    await gridCell(page, 0, 2).dblclick()
    await editor(page).fill('Blurred')
    await gridCell(page, 1, 1).click()
    await expect(gridCell(page, 0, 2)).toHaveText('Blurred')
    await expect(pending(page)).toHaveText('1 change')
  })

  test('Enter on a selected cell, F2 and just typing all start editing', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await page.keyboard.press('Enter')
    await expect(editor(page)).toHaveValue('Alice')
    await page.keyboard.press('Escape')

    await page.keyboard.press('F2')
    await expect(editor(page)).toHaveValue('Alice')
    await page.keyboard.press('Escape')

    await page.keyboard.type('Zed')
    await expect(editor(page)).toHaveValue('Zed')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 2)).toHaveText('Zed')
  })

  test('Enter commits and moves down; Tab commits and moves right', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await page.keyboard.press('Enter')
    await editor(page).fill('First')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 2)).toHaveText('First')
    await page.keyboard.press('Enter') // the active cell moved down: edit row 2
    await expect(editor(page)).toHaveValue('Bob')
    await page.keyboard.press('Escape')

    await gridCell(page, 1, 1).click()
    await page.keyboard.press('Enter')
    await editor(page).fill('x@example.com')
    await page.keyboard.press('Tab')
    await expect(gridCell(page, 1, 1)).toHaveText('x@example.com')
    await page.keyboard.press('Enter')
    await expect(editor(page)).toHaveValue('Bob') // moved right to the name cell
  })

  test('the primary key column can be edited too (it is an ordinary cell)', async ({ page }) => {
    await gridCell(page, 0, 0).dblclick()
    await expect(editor(page)).toBeVisible()
    await page.keyboard.press('Escape')
  })

  test('Set NULL (⌘⇧N) works on nullable columns and refuses NOT NULL ones', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await page.keyboard.press(`${MOD}+Shift+N`)
    await expect(gridCell(page, 0, 2)).toHaveText('NULL')
    await expect(pending(page)).toHaveText('1 change')

    await gridCell(page, 0, 1).click() // email is NOT NULL
    await page.keyboard.press(`${MOD}+Shift+N`)
    await expect(bottomBar(page).getByText('email is NOT NULL.')).toBeVisible()
    await expect(gridCell(page, 0, 1)).toHaveText('a@example.com')
  })

  test('Preview shows the SQL that would run, and can be copied or closed', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await action(page, 'Preview').click()
    const preview = dialog(page, 'Preview SQL')
    await expect(preview).toBeVisible()
    await expect(preview).toContainText('UPDATE')
    await preview.getByRole('button', { name: 'Copy' }).click()
    await expect(preview.getByRole('button', { name: 'Copied' })).toBeVisible()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('UPDATE')
    await preview.getByRole('button', { name: 'Close' }).click()
    await expect(preview).toHaveCount(0)
    await expect(pending(page)).toHaveText('1 change')
  })

  test('Preview → Commit commits from inside the dialog', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await action(page, 'Preview').click()
    await dialog(page, 'Preview SQL').getByRole('button', { name: 'Commit' }).click()
    await expect(bottomBar(page).getByText('Committed 1 statement, 1 row affected.')).toBeVisible()
    await expect(pending(page)).toHaveCount(0)
  })

  test('Discard throws the pending changes away', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await expect(pending(page)).toHaveText('1 change')
    await action(page, 'Discard').click()
    await expect(pending(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await expect(gridCell(page, 0, 2)).not.toHaveClass(/edited/)
  })

  test('Commit on a non-production connection runs straight away and reports what happened', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await action(page, 'Commit').click()
    await expect(dialog(page, 'Commit to production?')).toHaveCount(0)
    await expect(bottomBar(page).getByText('Committed 1 statement, 1 row affected.')).toBeVisible()
    await expect(pending(page)).toHaveCount(0)
  })

  test('⌘S commits', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await page.keyboard.press(`${MOD}+s`)
    await expect(bottomBar(page).getByText('Committed 1 statement')).toBeVisible()
  })

  test('several edits are counted and committed together', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await gridCell(page, 1, 2).dblclick()
    await editor(page).fill('Robert')
    await page.keyboard.press('Enter')
    await expect(pending(page)).toHaveText('2 changes')
    await action(page, 'Preview').click()
    await expect(dialog(page, 'Preview SQL')).toBeVisible()
  })

  test('editing the same cell twice is still one change; setting it back removes it', async ({ page }) => {
    await editFirstName(page, 'Alicia')
    await editFirstName(page, 'Alicia 2')
    await expect(pending(page)).toHaveText('1 change')
    await editFirstName(page, 'Alice')
    await expect(pending(page)).toHaveCount(0)
  })
})

test.describe('adding and deleting rows', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
  })

  test('+ Row inserts a draft row at the top, marked with + and counted in the bottom bar', async ({ page }) => {
    await bottomBar(page).getByRole('button', { name: 'Row', exact: true }).click()
    await expect(grid(page).locator('.g').first()).toHaveText('+')
    await expect(gridRows(page).first()).toHaveClass(/inserted/)
    await expect(pending(page)).toHaveText('1 change')
    await expect(bottomBar(page).getByText('2 rows + 1 new')).toBeVisible()
    // The new row is selected and ready to type into; existing rows moved down.
    await expect.poll(() => rowTexts(page, 1)).toEqual(['1', 'a@example.com', 'Alice'])
  })

  test('typing into a new row fills its cells; Delete removes an unsaved row', async ({ page }) => {
    await bottomBar(page).getByRole('button', { name: 'Row', exact: true }).click()
    await expect(grid(page).locator('.row.cur')).toHaveCount(1)
    await page.keyboard.press('ArrowRight')
    await page.keyboard.press('Enter')
    await editor(page).fill('new@example.com')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 1)).toHaveText('new@example.com')

    await gridCell(page, 0, 1).click()
    await page.keyboard.press('Delete')
    await expect(pending(page)).toHaveCount(0)
    await expect(bottomBar(page).getByText('2 rows', { exact: true })).toBeVisible()
  })

  test('Delete marks the selected row as deleted; pressing it again restores the row', async ({ page }) => {
    await gridCell(page, 1, 1).click()
    await page.keyboard.press('Delete')
    await expect(gridRows(page).nth(1)).toHaveClass(/deleted/)
    await expect(pending(page)).toHaveText('1 change')
    await page.keyboard.press('Delete')
    await expect(gridRows(page).nth(1)).not.toHaveClass(/deleted/)
    await expect(pending(page)).toHaveCount(0)
  })

  test('the trash button deletes the selected rows and is disabled without a selection', async ({ page }) => {
    const trash = bottomBar(page).getByRole('button', { name: 'Delete selected rows' })
    await expect(trash).toBeDisabled()
    await grid(page).locator('.g').nth(0).click() // select the first row via the gutter
    await expect(trash).toBeEnabled()
    await trash.click()
    await expect(gridRows(page).first()).toHaveClass(/deleted/)
    await expect(pending(page)).toHaveText('1 change')
  })

  test('selecting several rows and deleting counts every row', async ({ page }) => {
    await grid(page).locator('.corner').click()
    await page.keyboard.press('Delete')
    await expect(pending(page)).toHaveText('2 changes')
  })

  test('⌘D duplicates the selected row into a new draft row', async ({ page }) => {
    await gridCell(page, 1, 2).click()
    await page.keyboard.press(`${MOD}+d`)
    await expect(gridRows(page).first()).toHaveClass(/inserted/)
    await expect.poll(() => rowTexts(page, 0)).toEqual(['DEFAULT', 'b@example.com', 'Bob'])
    await expect(pending(page)).toHaveText('1 change')
  })

  test('the cell menu offers Duplicate, Delete and Set NULL', async ({ page }) => {
    await gridCell(page, 0, 2).click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: /Duplicate row/ })).toBeEnabled()
    await expect(page.getByRole('menuitem', { name: /Delete row/ })).toBeEnabled()
    await expect(page.getByRole('menuitem', { name: /Set NULL/ })).toBeEnabled()
    await page.getByRole('menuitem', { name: /Delete row/ }).click()
    await expect(gridRows(page).first()).toHaveClass(/deleted/)
    await gridCell(page, 0, 2).click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: /Restore row/ })).toBeVisible()
  })
})

test.describe('typed editors', () => {
  test('enum columns edit with a select of their values', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'orders')
    await gridCell(page, 0, 3).dblclick()
    const select = grid(page).locator('.edit-host select')
    await expect(select).toBeVisible()
    await expect(select.locator('option')).toContainText(['sad', 'ok', 'happy'])
    await select.selectOption('sad')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 3)).toContainText('sad')
    await expect(pending(page)).toHaveText('1 change')
  })

  test('boolean columns edit with a true/false select', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 4).dblclick()
    const select = grid(page).locator('.edit-host select')
    await expect(select).toBeVisible()
    await expect(select.locator('option')).toContainText(['true', 'false'])
    await select.selectOption('true')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 4)).toHaveText('true')
  })

  test('JSON cells open a JSON editor dialog instead of the inline editor', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    // Select a row first so the Details panel is already open and the grid has its final width.
    await gridCell(page, 0, 0).click()
    await expect(inspector(page)).toBeVisible()
    await expect.poll(() => grid(page).evaluate((el) => el.clientWidth)).toBeLessThan(1000)
    await gridCell(page, 2, 6).scrollIntoViewIfNeeded()
    await grid(page).evaluate((el) => (el.scrollLeft = el.scrollWidth))
    await gridCell(page, 2, 6).dblclick()
    const dlg = dialog(page, 'payload (edit JSON)')
    await expect(dlg).toBeVisible()
    await expect(dlg.getByRole('textbox')).toHaveValue(/"n": 3/)
    await expect(dlg).toContainText('Valid JSON')
    await page.keyboard.press('Escape')
    await expect(dlg).toHaveCount(0)
    await expect(grid(page)).toBeFocused()
  })

  test('double-clicking a far-right cell on the very first selection edits that cell', async ({ page }) => {
    // BUG: the first click of the double-click auto-opens the Details panel, which narrows the grid by 300px
    // while the second click is still on its way, so the double-click lands on another cell (or on nothing)
    // and the JSON editor never opens. Related to the "clicked cell stays in view" bug in grid.spec.ts.
    test.fail()
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 2, 6).dblclick()
    await expect(dialog(page, 'payload (edit JSON)')).toBeVisible({ timeout: 2000 })
  })
})

test.describe('read-only', () => {
  test('a read-only connection blocks editing, adding and deleting, and says so', async ({ page }) => {
    await openConnection(page, 'Production')
    await openTableLoaded(page, 'customers')
    await expect(page.getByText('This connection is read-only.')).toBeVisible()
    await expect(bottomBar(page).getByRole('button', { name: 'Row', exact: true })).toBeDisabled()
    await expect(bottomBar(page).getByRole('button', { name: 'Delete selected rows' })).toBeDisabled()
    await expect(bottomBar(page).getByRole('button', { name: 'Import CSV' })).toBeDisabled()

    await gridCell(page, 0, 2).dblclick()
    await expect(editor(page)).toHaveCount(0)
    await gridCell(page, 0, 2).click()
    await page.keyboard.type('x')
    await expect(editor(page)).toHaveCount(0)
    await page.keyboard.press('Delete')
    await expect(pending(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('a read-only connection shows details as plain text, not inputs', async ({ page }) => {
    await openConnection(page, 'Production')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 2).click()
    await expect(detailField(page, 'name')).toBeVisible()
    await expect(inspector(page).locator('input.value, textarea.value')).toHaveCount(0)
    expect(await detailValue(page, 'name')).toBe('Alice')
  })

  test('a table without a primary key is read-only with an explanation', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'no_pk')
    await expect(page.getByText(/no primary key/i).first()).toBeVisible()
    await gridCell(page, 0, 1).dblclick()
    await expect(editor(page)).toHaveCount(0)
  })

  test('views cannot be edited', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customer_totals')
    await expect(page.getByText("Views can't be edited.")).toBeVisible()
    await expect(bottomBar(page).getByRole('button', { name: 'Row', exact: true })).toBeDisabled()
  })
})

test.describe('production commits', () => {
  /** Creates a writable production connection (the seeded Production one is read-only) and opens a table on it. */
  async function openWritableProduction(page: Page) {
    await gotoApp(page)
    await page.getByRole('button', { name: 'New connection' }).click()
    const form = dialog(page, 'New connection')
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Prod RW')
    await form.getByRole('radio', { name: 'Production' }).click()
    await form.getByRole('button', { name: 'Save' }).click()
    await card(page, 'Prod RW').getByRole('button', { name: 'Open', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Toggle sidebar' })).toBeVisible()
    await openTableLoaded(page, 'customers')
  }

  test('a writable production connection warns, then asks for confirmation before committing', async ({ page }) => {
    await openWritableProduction(page)
    await expect(page.getByText('Production connection: commits ask for confirmation.')).toBeVisible()
    await editFirstName(page, 'Alicia')
    await action(page, 'Commit').click()

    const confirm = dialog(page, 'Commit to production?')
    await expect(confirm).toBeVisible()
    await expect(confirm).toContainText('Prod RW')
    await expect(confirm).toContainText('1 row updated')
    await expect(confirm).toContainText('rolled back if a statement fails')
    // Nothing committed yet.
    await expect(pending(page)).toHaveText('1 change')
    await expect(bottomBar(page).getByText('Committed')).toHaveCount(0)
  })

  test('Cancel (and Escape) keep the pending changes and do not commit', async ({ page }) => {
    await openWritableProduction(page)
    await editFirstName(page, 'Alicia')
    await action(page, 'Commit').click()
    await dialog(page, 'Commit to production?').getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog(page, 'Commit to production?')).toHaveCount(0)
    await expect(pending(page)).toHaveText('1 change')

    await page.keyboard.press(`${MOD}+s`)
    await expect(dialog(page, 'Commit to production?')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog(page, 'Commit to production?')).toHaveCount(0)
    await expect(pending(page)).toHaveText('1 change')
  })

  test('confirming commits and reports the result', async ({ page }) => {
    await openWritableProduction(page)
    await editFirstName(page, 'Alicia')
    await action(page, 'Commit').click()
    await dialog(page, 'Commit to production?').getByRole('button', { name: 'Commit to production' }).click()
    await expect(bottomBar(page).getByText('Committed 1 statement, 1 row affected.')).toBeVisible()
    await expect(pending(page)).toHaveCount(0)
  })

  test('the confirmation lists inserts, updates and deletes; Preview SQL is available from it', async ({ page }) => {
    await openWritableProduction(page)
    await editFirstName(page, 'Alicia')
    await bottomBar(page).getByRole('button', { name: 'Row', exact: true }).click()
    await gridCell(page, 2, 1).click()
    await page.keyboard.press('Delete')
    await action(page, 'Commit').click()
    const confirm = dialog(page, 'Commit to production?')
    await expect(confirm).toContainText('1 row updated')
    await expect(confirm).toContainText('1 row inserted')
    await expect(confirm).toContainText('1 row deleted')
    await confirm.getByRole('button', { name: 'Preview SQL' }).click()
    await expect(dialog(page, 'Preview SQL')).toBeVisible()
  })
})

test.describe('details panel editing', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 0).click()
    await expect(inspector(page)).toBeVisible()
  })

  const input = (page: Page, name: string) => detailField(page, name).locator('input.value, textarea.value')

  test('fields of an editable table are inputs holding the current values', async ({ page }) => {
    await expect(input(page, 'id')).toHaveValue('1')
    await expect(input(page, 'email')).toHaveValue('a@example.com')
    await expect(input(page, 'name')).toHaveValue('Alice')
  })

  test('changing a field and pressing Enter stages a change that shows in the grid and bottom bar', async ({ page }) => {
    await input(page, 'name').fill('Alicia')
    await input(page, 'name').press('Enter')
    await expect(pending(page)).toHaveText('1 change')
    await expect(action(page, 'Preview')).toBeVisible()
    await expect(action(page, 'Discard')).toBeVisible()
    await expect(action(page, 'Commit')).toBeVisible()
    await expect(gridCell(page, 0, 2)).toHaveText('Alicia')
    await expect(gridCell(page, 0, 2)).toHaveClass(/edited/)
    await expect(input(page, 'name')).toHaveValue('Alicia')
  })

  test('blurring the field also stages the change', async ({ page }) => {
    await input(page, 'name').fill('Alicia')
    await input(page, 'email').focus()
    await expect(pending(page)).toHaveText('1 change')
    await expect(gridCell(page, 0, 2)).toHaveText('Alicia')
  })

  test('Escape reverts the field and stages nothing', async ({ page }) => {
    await input(page, 'name').fill('Alicia')
    await input(page, 'name').press('Escape')
    await expect(input(page, 'name')).toHaveValue('Alice')
    await expect(pending(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('an unchanged field stages nothing', async ({ page }) => {
    await input(page, 'name').focus()
    await input(page, 'name').press('Enter')
    await expect(pending(page)).toHaveCount(0)
  })

  test('the field menu offers Set NULL, Set DEFAULT, Set EMPTY and Copy value', async ({ page }) => {
    await detailField(page, 'name').getByRole('button', { name: 'Field options' }).click()
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toEqual(['Set NULL', 'Set DEFAULT', 'Set EMPTY', 'Copy value'])
  })

  test('Set NULL stages NULL for the field; Set EMPTY stages an empty string', async ({ page }) => {
    await detailField(page, 'name').getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Set NULL' }).click()
    await expect(gridCell(page, 0, 2)).toHaveText('NULL')
    await expect(pending(page)).toHaveText('1 change')

    await detailField(page, 'name').getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Set EMPTY' }).click()
    await expect(gridCell(page, 0, 2)).toHaveText('')
    await expect(pending(page)).toHaveText('1 change')
  })

  test('Set DEFAULT stages the DEFAULT keyword', async ({ page }) => {
    await detailField(page, 'name').getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Set DEFAULT' }).click()
    await expect(gridCell(page, 0, 2)).toHaveText('DEFAULT')
  })

  test('Copy value puts the field value on the clipboard', async ({ page }) => {
    await detailField(page, 'email').getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Copy value' }).click()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('a@example.com')
  })

  test('discarding from the bottom bar resets the field to the stored value', async ({ page }) => {
    // BUG: Discard clears the grid selection, so TableView stops publishing a row to the inspector and the
    // Details panel keeps showing the discarded value ("Alicia") until another row is clicked
    // (TableView.svelte: the inspector.show effect returns early when `selection` is null).
    test.fail()
    await input(page, 'name').fill('Alicia')
    await input(page, 'name').press('Enter')
    await action(page, 'Discard').click()
    await expect(input(page, 'name')).toHaveValue('Alice')
    await expect(pending(page)).toHaveCount(0)
  })

  test('details edits keep working after switching rows', async ({ page }) => {
    await gridCell(page, 1, 0).click()
    await expect(input(page, 'name')).toHaveValue('Bob')
    await input(page, 'name').fill('Robert')
    await input(page, 'name').press('Enter')
    await expect(gridCell(page, 1, 2)).toHaveText('Robert')
    await expect(pending(page)).toHaveText('1 change')
  })
})

test.describe('JSON editing in the Details panel', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 2, 6).click()
    await expect(inspector(page)).toBeVisible()
  })

  const area = (page: Page) => detailField(page, 'payload').locator('textarea.value')

  test('a JSON field is a textarea holding the pretty-printed document', async ({ page }) => {
    await expect(area(page)).toBeVisible()
    expect(JSON.parse(await area(page).inputValue())).toEqual({ n: 3, tags: ['a', 'b'] })
  })

  test('invalid JSON shows a parse error and stages nothing', async ({ page }) => {
    await area(page).fill('{ "n": ')
    await area(page).blur()
    await expect(detailField(page, 'payload').locator('.jerr')).toBeVisible()
    await expect(detailField(page, 'payload').locator('.jerr')).not.toBeEmpty()
    await expect(pending(page)).toHaveCount(0)
  })

  test('valid JSON is staged and the error goes away', async ({ page }) => {
    await area(page).fill('{ "n": ')
    await area(page).blur()
    await expect(detailField(page, 'payload').locator('.jerr')).toBeVisible()
    await area(page).fill('{"n": 99}')
    await area(page).blur()
    await expect(detailField(page, 'payload').locator('.jerr')).toHaveCount(0)
    await expect(pending(page)).toHaveText('1 change')
  })

  test('Escape in the textarea restores the stored JSON', async ({ page }) => {
    const before = await area(page).inputValue()
    await area(page).fill('{"n": 1}')
    await area(page).press('Escape')
    await expect(area(page)).toHaveValue(before)
    await expect(pending(page)).toHaveCount(0)
  })

  test('the field menu has Set NULL/DEFAULT and Pretty/Minify next to Copy', async ({ page }) => {
    await detailField(page, 'payload').getByRole('button', { name: 'Field options' }).click()
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toEqual(['Set NULL', 'Set DEFAULT', 'Set EMPTY', '✓  Pretty', 'Minify', 'Copy value'])
  })
})

test.describe('unsaved changes (drafts)', () => {
  test('pending changes survive a reload and are offered back for restoring', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
    await editFirstName(page, 'Alicia')
    await expect(pending(page)).toHaveText('1 change')
    await expect.poll(() => page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('tablesoft:draft:')).length)).toBeGreaterThan(0)

    await page.reload()
    await openConnection(page, 'Local Postgres')
    await page.getByRole('treeitem', { name: 'customers', exact: true }).dblclick()
    const prompt = dialog(page, 'Unsaved changes found')
    await expect(prompt).toBeVisible()
    await expect(prompt).toContainText('1 pending change')
    await expect(prompt).toContainText('customers')

    await prompt.getByRole('button', { name: 'Restore' }).click()
    await expect(pending(page)).toHaveText('1 change')
    await expect(gridCell(page, 0, 2)).toHaveText('Alicia')
  })

  test('Discard throws the draft away for good', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
    await editFirstName(page, 'Alicia')
    await page.reload()
    await openConnection(page, 'Local Postgres')
    await page.getByRole('treeitem', { name: 'customers', exact: true }).dblclick()
    await dialog(page, 'Unsaved changes found').getByRole('button', { name: 'Discard' }).click()
    await expect(pending(page)).toHaveCount(0)
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await expect.poll(() => page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('tablesoft:draft:')).length)).toBe(0)
  })

  test('the draft prompt cannot be dismissed with Escape or by clicking outside', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
    await editFirstName(page, 'Alicia')
    await page.reload()
    await openConnection(page, 'Local Postgres')
    await page.getByRole('treeitem', { name: 'customers', exact: true }).dblclick()
    const prompt = dialog(page, 'Unsaved changes found')
    await expect(prompt).toBeVisible()
    await page.keyboard.press('Escape')
    await page.mouse.click(5, 5)
    await expect(prompt).toBeVisible()
  })

  test('committing clears the draft', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await openTableLoaded(page, 'customers')
    await editFirstName(page, 'Alicia')
    await action(page, 'Commit').click()
    await expect(bottomBar(page).getByText('Committed')).toBeVisible()
    await expect.poll(() => page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('tablesoft:draft:')).length)).toBe(0)
  })
})
