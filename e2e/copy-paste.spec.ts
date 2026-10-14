import { test, expect } from './fixtures'
import {
  MOD,
  bottomBar,
  card,
  grid,
  gridCell,
  gridRows,
  gotoApp,
  invokes,
  newQueryTab,
  openConnection,
  openTable,
  openTableLoaded,
  recordInvokes,
  rowTexts,
  runStatement,
  setSql,
} from './helpers'
import type { Page } from '@playwright/test'

const toasts = (page: Page) => page.locator('.toasts')
const pending = (page: Page) => bottomBar(page).locator('.pending-label')
const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText())
const setClipboard = (page: Page, text: string) => page.evaluate((t) => navigator.clipboard.writeText(t), text)
const menuItem = (page: Page, name: string | RegExp) => page.getByRole('menuitem', { name })

/** Right-click a cell, open "Copy as" and pick a format. */
async function copyAs(page: Page, row: number, col: number, format: string | RegExp) {
  await gridCell(page, row, col).click({ button: 'right' })
  await menuItem(page, /^Copy as/).click()
  await menuItem(page, format).click()
}

/** Selects the rectangle from (r0, c0) to (r1, c1) with the keyboard. */
async function select(page: Page, r0: number, c0: number, r1: number, c1: number) {
  await gridCell(page, r0, c0).click()
  for (let i = r0; i < r1; i++) await page.keyboard.press('Shift+ArrowDown')
  for (let i = c0; i < c1; i++) await page.keyboard.press('Shift+ArrowRight')
}

/** Puts text on the clipboard, then pastes it with the shortcut while the grid has the focus. */
async function paste(page: Page, text: string) {
  await setClipboard(page, text)
  await expect(grid(page)).toBeFocused()
  await page.keyboard.press(`${MOD}+v`)
}

test.describe('copy as', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('the cell menu has a "Copy as" flyout with every format', async ({ page }) => {
    await gridCell(page, 0, 1).click({ button: 'right' })
    const parent = menuItem(page, /^Copy as/)
    await expect(parent).toHaveAttribute('aria-haspopup', 'menu')
    await expect(parent).toHaveAttribute('aria-expanded', 'false')
    await parent.click()
    await expect(parent).toHaveAttribute('aria-expanded', 'true')
    const sub = page.getByRole('menu', { name: 'Copy as' })
    await expect(sub).toBeVisible()
    expect((await sub.getByRole('menuitem').allTextContents()).map((t) => t.trim())).toEqual([
      'INSERT statements',
      'JSON',
      'Markdown table',
      'Tab-separated',
    ])
  })

  test('the cell menu opens from the keyboard (Menu key, Shift+F10) and copies without the mouse', async ({ page }) => {
    await gridCell(page, 1, 2).click()
    await page.keyboard.press('ContextMenu')
    await expect(page.getByRole('menu')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
    await expect(grid(page)).toBeFocused()
    await page.keyboard.press('Shift+F10')
    const copyAsItem = menuItem(page, /^Copy as/)
    await copyAsItem.focus()
    await page.keyboard.press('ArrowRight')
    await page.keyboard.press('ArrowDown') // JSON
    await page.keyboard.press('Enter')
    await expect(page.getByRole('menu')).toHaveCount(0)
    expect(JSON.parse(await clipboard(page))).toEqual([{ id: 2, email: 'b@example.com', name: 'Bob' }])
  })

  test('hovering "Copy as" opens the flyout, hovering another item closes it', async ({ page }) => {
    await gridCell(page, 0, 1).click({ button: 'right' })
    await menuItem(page, /^Copy as/).hover()
    await expect(page.getByRole('menu', { name: 'Copy as' })).toBeVisible()
    await menuItem(page, /^Filter by this value/).hover()
    await expect(page.getByRole('menu', { name: 'Copy as' })).toHaveCount(0)
  })

  test('the flyout is keyboard driven: ArrowRight opens, ArrowLeft and Escape close just the flyout', async ({ page }) => {
    await gridCell(page, 0, 1).click({ button: 'right' })
    await menuItem(page, /^Copy as/).focus()
    await page.keyboard.press('ArrowRight')
    const sub = page.getByRole('menu', { name: 'Copy as' })
    await expect(sub.getByRole('menuitem', { name: 'INSERT statements' })).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await expect(sub.getByRole('menuitem', { name: 'JSON' })).toBeFocused()
    await page.keyboard.press('ArrowLeft')
    await expect(sub).toHaveCount(0)
    await expect(menuItem(page, /^Copy as/)).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(sub.getByRole('menuitem').first()).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(sub).toHaveCount(0)
    await expect(page.getByRole('menu')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
    await expect(grid(page)).toBeFocused()
  })

  test('INSERT statements: one per selected row, selected columns only', async ({ page }) => {
    await select(page, 0, 1, 1, 2)
    await copyAs(page, 0, 1, 'INSERT statements')
    expect(await clipboard(page)).toBe(
      [
        `INSERT INTO "public"."customers" ("email", "name") VALUES ('a@example.com', 'Alice');`,
        `INSERT INTO "public"."customers" ("email", "name") VALUES ('b@example.com', 'Bob');`,
      ].join('\n'),
    )
    await expect(toasts(page)).toContainText('Copied 2 rows as INSERT statements.')
  })

  test('a single selected cell copies its whole row', async ({ page }) => {
    await gridCell(page, 1, 2).click()
    await copyAs(page, 1, 2, 'JSON')
    expect(JSON.parse(await clipboard(page))).toEqual([{ id: 2, email: 'b@example.com', name: 'Bob' }])
    await expect(toasts(page)).toContainText('Copied 1 row as JSON.')
  })

  test('JSON keeps real types and only the selected columns', async ({ page }) => {
    await select(page, 0, 0, 1, 1)
    await copyAs(page, 0, 0, 'JSON')
    expect(JSON.parse(await clipboard(page))).toEqual([
      { id: 1, email: 'a@example.com' },
      { id: 2, email: 'b@example.com' },
    ])
  })

  test('Markdown table escapes pipes and line breaks of edited values', async ({ page }) => {
    await gridCell(page, 0, 2).dblclick()
    await grid(page).locator('.edit-host input').fill('a|b')
    await page.keyboard.press('Enter')
    await select(page, 0, 1, 1, 2)
    await copyAs(page, 0, 1, 'Markdown table')
    expect(await clipboard(page)).toBe(['| email | name |', '| --- | --- |', '| a@example.com | a\\|b |', '| b@example.com | Bob |'].join('\n'))
  })

  test('Tab-separated in the flyout matches Copy', async ({ page }) => {
    await select(page, 0, 0, 1, 2)
    await copyAs(page, 0, 0, 'Tab-separated')
    expect(await clipboard(page)).toBe('1\ta@example.com\tAlice\n2\tb@example.com\tBob')
  })

  test('a menu entry copies only what is selected, even when the click was outside it', async ({ page }) => {
    await select(page, 0, 0, 0, 0)
    await copyAs(page, 1, 1, 'JSON') // right-clicking another cell moves the selection there
    expect(JSON.parse(await clipboard(page))).toEqual([{ id: 2, email: 'b@example.com', name: 'Bob' }])
  })

  test('⌘C writes NULL for a NULL cell', async ({ page }) => {
    // big_table.payload is NULL in its first row
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 6).click()
    await page.keyboard.press(`${MOD}+c`)
    await expect.poll(() => clipboard(page)).toBe('NULL')
  })

  test('big_table: INSERT statements get number, boolean and NULL literals right', async ({ page }) => {
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 0).click()
    await copyAs(page, 0, 0, 'INSERT statements')
    const text = await clipboard(page)
    expect(text).toMatch(/^INSERT INTO "public"\."big_table" \("id", "uuid", "name", "amount", "is_active", "created_at", "payload"\) VALUES \(1, '[0-9a-f-]+', 'row 1', 0\.01, FALSE, '2020-01-01 00:01:00\.000\+00', NULL\);$/)
  })

  test('a huge selection loads at most 10,000 rows and says how many were over the limit', async ({ page }) => {
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 0).click()
    await page.keyboard.press(`${MOD}+a`)
    await copyAs(page, 0, 1, 'JSON')
    await expect(toasts(page)).toContainText('Copied 10000 rows as JSON. 990000 rows over the copy limit, left out.', { timeout: 20_000 })
    const rows = JSON.parse(await clipboard(page)) as { id: number }[]
    expect(rows).toHaveLength(10_000)
    expect(rows[0].id).toBe(1)
    expect(rows[9_999].id).toBe(10_000)
  })
})

test.describe('copy as: other engines', () => {
  test('MySQL quotes with backticks and qualifies the table with its database', async ({ page }) => {
    await openConnection(page, 'Local MySQL')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 0).click()
    await copyAs(page, 0, 0, 'INSERT statements')
    expect(await clipboard(page)).toBe("INSERT INTO `tablesail`.`customers` (`id`, `email`, `name`) VALUES (1, 'a@example.com', 'Alice');")
  })

  test('SQLite has no schema qualifier', async ({ page }) => {
    await openConnection(page, 'nasty.db')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 0).click()
    await copyAs(page, 0, 0, 'INSERT statements')
    expect(await clipboard(page)).toBe(`INSERT INTO "customers" ("id", "email", "name") VALUES (1, 'a@example.com', 'Alice');`)
  })

  test('MongoDB offers JSON and Tab-separated only', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 1).click({ button: 'right' })
    await menuItem(page, /^Copy as/).click()
    const sub = page.getByRole('menu', { name: 'Copy as' })
    expect((await sub.getByRole('menuitem').allTextContents()).map((t) => t.trim())).toEqual(['JSON', 'Tab-separated'])
    await sub.getByRole('menuitem', { name: 'JSON' }).click()
    expect(JSON.parse(await clipboard(page))).toEqual([{ id: 1, email: 'a@example.com', name: 'Alice' }])
  })
})

test.describe('copy as: query results', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    await expect(gridCell(page, 0, 0)).toBeVisible()
  })

  test('the result grid has the same copy menu, with a placeholder table name for INSERT', async ({ page }) => {
    await gridCell(page, 0, 1).click({ button: 'right' })
    await expect(menuItem(page, /^Copy as/)).toBeVisible()
    expect((await page.getByRole('menuitem').allTextContents()).map((t) => t.trim().replace(/\s+.*$/, ''))).toEqual(['Copy', 'Copy', 'Copy'])
    await menuItem(page, /^Copy as/).click()
    await page.getByRole('menuitem', { name: 'INSERT statements' }).click()
    await expect.poll(() => clipboard(page)).toBe(`INSERT INTO table_name ("id", "email", "name") VALUES (1, 'a@example.com', 'Alice');`)
  })

  test('JSON and Markdown work on results too', async ({ page }) => {
    await select(page, 0, 0, 1, 1)
    await gridCell(page, 0, 0).click({ button: 'right' })
    await menuItem(page, /^Copy as/).click()
    await page.getByRole('menuitem', { name: 'Markdown table' }).click()
    expect(await clipboard(page)).toBe(['| id | email |', '| ---: | --- |', '| 1 | a@example.com |', '| 2 | b@example.com |'].join('\n'))
  })

  test('pasting into results explains that they are read-only', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, 'x')
    await expect(toasts(page)).toContainText('Query results are read-only')
  })
})

test.describe('paste into the grid', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('pastes a block of cells as pending edits starting at the active cell', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, 'x@example.com\tXavier\ny@example.com\tYvonne')
    await expect(toasts(page)).toContainText('Pasted 4 cells.')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'x@example.com', 'Xavier'])
    await expect.poll(() => rowTexts(page, 1)).toEqual(['2', 'y@example.com', 'Yvonne'])
    await expect(gridCell(page, 0, 1)).toHaveClass(/edited/)
    await expect(gridCell(page, 1, 2)).toHaveClass(/edited/)
    await expect(gridCell(page, 0, 0)).not.toHaveClass(/edited/)
    await expect(pending(page)).toHaveText('2 changes')
    // the pasted block ends up selected, and the grid keeps the focus
    for (const [r, c, on] of [[0, 0, false], [0, 1, true], [0, 2, true], [1, 1, true], [1, 2, true], [1, 0, false]] as const) {
      if (on) await expect(gridCell(page, r, c)).toHaveAttribute('aria-selected', 'true')
      else await expect(gridCell(page, r, c)).not.toHaveAttribute('aria-selected', 'true')
    }
    await expect(grid(page)).toBeFocused()
  })

  test('commits through the usual flow as updates keyed by the primary key', async ({ page }) => {
    await recordInvokes(page)
    await gridCell(page, 1, 2).click()
    await paste(page, 'Robert')
    await expect(pending(page)).toHaveText('1 change')
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(async () => (await invokes(page, 'changes_apply')).length).toBe(1)
    const [call] = await invokes(page, 'changes_apply')
    expect(call.args.changes).toEqual({
      table: { database: 'tablesail', schema: 'public', name: 'customers' },
      changes: [
        {
          kind: 'update',
          key: [{ column: 'id', value: 2 }],
          original: [{ column: 'name', value: 'Bob' }],
          set: [{ column: 'name', value: { kind: 'text', value: 'Robert' } }],
        },
      ],
    })
  })

  test('rows that run past the end of the table become new pending rows', async ({ page }) => {
    await recordInvokes(page)
    await gridCell(page, 1, 1).click()
    await paste(page, 'b2@example.com\tBobby\nc@example.com\tCarol\nd@example.com\tDan')
    await expect(toasts(page)).toContainText('Pasted 6 cells (2 new rows).')
    await expect(bottomBar(page)).toContainText('+ 2 new')
    await expect(pending(page)).toHaveText('3 changes')
    // new rows sit on top, newest first; the existing row was edited in place
    await expect(gridRows(page)).toHaveCount(4)
    await expect.poll(() => rowTexts(page, 0)).toEqual(['DEFAULT', 'd@example.com', 'Dan'])
    await expect.poll(() => rowTexts(page, 1)).toEqual(['DEFAULT', 'c@example.com', 'Carol'])
    await expect(gridCell(page, 3, 1)).toHaveText('b2@example.com')
    await bottomBar(page).getByRole('button', { name: 'Commit', exact: true }).click()
    await expect.poll(async () => (await invokes(page, 'changes_apply')).length).toBe(1)
    const [call] = await invokes(page, 'changes_apply')
    const changes = (call.args.changes as { changes: { kind: string; values?: unknown }[] }).changes
    expect(changes.filter((c) => c.kind === 'insert').map((c) => c.values)).toEqual([
      [
        { column: 'email', value: { kind: 'text', value: 'c@example.com' } },
        { column: 'name', value: { kind: 'text', value: 'Carol' } },
      ],
      [
        { column: 'email', value: { kind: 'text', value: 'd@example.com' } },
        { column: 'name', value: { kind: 'text', value: 'Dan' } },
      ],
    ])
    expect(changes.filter((c) => c.kind === 'update')).toHaveLength(1)
  })

  test('the text NULL becomes NULL', async ({ page }) => {
    await recordInvokes(page)
    await gridCell(page, 0, 2).click()
    await paste(page, 'NULL')
    await expect(gridCell(page, 0, 2)).toHaveText('NULL')
    await expect(gridCell(page, 0, 2)).toHaveClass(/edited/)
    await page.keyboard.press(`${MOD}+s`)
    await expect.poll(async () => (await invokes(page, 'changes_apply')).length).toBe(1)
    const [call] = await invokes(page, 'changes_apply')
    expect(JSON.stringify(call.args.changes)).toContain('"set":[{"column":"name","value":{"kind":"null"}}]')
  })

  test('copy then paste round-trips NULL, empty strings and awkward text', async ({ page }) => {
    // build the awkward rows with a first paste, then copy them and paste them one row down
    await gridCell(page, 0, 1).click()
    await paste(page, '"tab\there"\tNULL\n"multi\nline ""quoted"""\t')
    await expect(pending(page)).toHaveText('2 changes')
    await select(page, 0, 1, 1, 2)
    await page.keyboard.press(`${MOD}+c`)
    await expect.poll(() => clipboard(page)).toBe('"tab\there"\tNULL\n"multi\nline ""quoted"""\t')
    // a quoted "NULL" is the text NULL, an unquoted one is NULL
    await gridCell(page, 0, 1).click()
    await paste(page, '"NULL"\tNULL')
    await expect(gridCell(page, 0, 1)).toHaveText('NULL')
    await expect(gridCell(page, 0, 1)).not.toHaveClass(/null/)
    await expect(gridCell(page, 0, 2)).toHaveClass(/null/)
  })

  test('falls back to CSV when the text has no tabs', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, 'csv@example.com,"Doe, John"')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'csv@example.com', 'Doe, John'])
  })

  test('a comma in a single value does not split it', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await paste(page, 'Doe, John')
    await expect(gridCell(page, 0, 2)).toHaveText('Doe, John')
    await expect(pending(page)).toHaveText('1 change')
  })

  test('a number with a thousands separator stays in one cell', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, '1,234')
    await expect(gridCell(page, 0, 1)).toHaveText('1,234')
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await expect(pending(page)).toHaveText('1 change')
  })

  test('one value fills the whole selection', async ({ page }) => {
    await select(page, 0, 2, 1, 2)
    await paste(page, 'Zed')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'a@example.com', 'Zed'])
    await expect.poll(() => rowTexts(page, 1)).toEqual(['2', 'b@example.com', 'Zed'])
    await expect(toasts(page)).toContainText('Pasted 2 cells.')
  })

  test('extra columns past the last one are ignored, and the toast says so', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, 'e@example.com\tEve\tx\ty\tz')
    await expect(toasts(page)).toContainText('Pasted 2 cells.')
    await expect(toasts(page)).toContainText('Ignored 3 extra columns')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'e@example.com', 'Eve'])
  })

  test('NULL into a NOT NULL column is skipped and reported', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await paste(page, 'NULL\tEve')
    await expect(toasts(page)).toContainText('Pasted 1 cell.')
    await expect(toasts(page)).toContainText('Skipped 1 (1 NOT NULL)')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'a@example.com', 'Eve'])
  })

  test('the context menu has Paste, which reads the clipboard', async ({ page }) => {
    await setClipboard(page, 'Menu Pasted')
    await gridCell(page, 0, 2).click({ button: 'right' })
    await expect(menuItem(page, /^Paste/)).toBeEnabled()
    await menuItem(page, /^Paste/).click()
    await expect(gridCell(page, 0, 2)).toHaveText('Menu Pasted')
    await expect(grid(page)).toBeFocused()
  })

  test('an open cell editor receives the paste itself; the grid stays out of it', async ({ page }) => {
    await setClipboard(page, 'typed in the editor')
    await gridCell(page, 0, 2).dblclick()
    const input = grid(page).locator('.edit-host input')
    await input.fill('')
    await page.keyboard.press(`${MOD}+v`)
    await expect(input).toHaveValue('typed in the editor')
    await expect(pending(page)).toHaveCount(0)
    await page.keyboard.press('Enter')
    await expect(pending(page)).toHaveText('1 change')
    await expect(gridCell(page, 0, 2)).toHaveText('typed in the editor')
  })

  test('pasting nothing, or with no cell selected, only says so', async ({ page }) => {
    await setClipboard(page, '')
    await gridCell(page, 0, 2).click()
    await page.keyboard.press(`${MOD}+v`)
    await expect(toasts(page)).toContainText('no text to paste')
    await expect(pending(page)).toHaveCount(0)
  })

  test('with no cell selected nothing is pasted and the toast says where to start', async ({ page }) => {
    await grid(page).focus()
    await paste(page, 'x')
    await expect(toasts(page)).toContainText('Select a cell first')
    await expect(pending(page)).toHaveCount(0)
  })

  test('pastes into a new pending row too', async ({ page }) => {
    await bottomBar(page).getByRole('button', { name: 'Row', exact: true }).click()
    await expect(grid(page)).toBeFocused()
    await paste(page, '9\tnina@example.com\tNina')
    await expect.poll(() => rowTexts(page, 0)).toEqual(['9', 'nina@example.com', 'Nina'])
    await expect(pending(page)).toHaveText('1 change')
    await expect(gridRows(page)).toHaveCount(3)
  })

  test('a paste can be discarded like any other change', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await paste(page, 'Temp')
    await expect(pending(page)).toHaveText('1 change')
    await bottomBar(page).getByRole('button', { name: 'Discard', exact: true }).click()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await expect(pending(page)).toHaveCount(0)
  })
})

test.describe('paste is refused where editing is', () => {
  test('a read-only connection says why', async ({ page }) => {
    await openConnection(page, 'Production')
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 2).click()
    await paste(page, 'Nope')
    await expect(toasts(page)).toContainText("Can't paste: This connection is read-only.")
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await gridCell(page, 0, 2).click({ button: 'right' })
    await expect(menuItem(page, /^Paste/)).toBeDisabled()
  })

  test('a table without a primary key says why', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'no_pk')
    await gridCell(page, 0, 1).click()
    await paste(page, 'Nope')
    await expect(toasts(page)).toContainText("Can't paste: This table has no primary key")
    await expect(bottomBar(page).locator('.pending-label')).toHaveCount(0)
  })

  test('a view says why', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customer_totals')
    await expect(grid(page).locator('.cell:not(.ph)').first()).toBeVisible()
    await gridCell(page, 0, 0).click()
    await paste(page, 'Nope')
    await expect(toasts(page)).toContainText("Can't paste: Views can't be edited.")
  })
})

test.describe('paste after choosing the connection', () => {
  test('the shortcut works on the first table opened straight from the welcome screen', async ({ page }) => {
    await gotoApp(page)
    await card(page, 'Local Postgres').getByRole('button', { name: /^Open / }).click()
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 2).click()
    await paste(page, 'First')
    await expect(gridCell(page, 0, 2)).toHaveText('First')
  })
})

test.describe('copy with headers next to a manual transaction', () => {
  test('⌘⇧C in a result grid copies with headers; it does not commit the open transaction', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await newQueryTab(page)
    await page.getByRole('group', { name: 'Transaction mode' }).getByRole('button', { name: 'Manual' }).click()
    await setSql(page, 'SELECT * FROM customers')
    await runStatement(page)
    const pill = page.locator('.tab-body:not([hidden]) .txpill')
    await expect(pill).toBeVisible()
    await expect(page.locator('.tab-body:not([hidden]) .toolbar .status')).toHaveText(/finished/)
    await gridCell(page, 0, 1).click()
    await page.keyboard.press(`${MOD}+Shift+c`)
    await expect.poll(() => clipboard(page)).toBe('email\na@example.com')
    await expect(pill).toBeVisible()
    await expect(page.locator('.console .sql', { hasText: 'COMMIT' })).toHaveCount(0)
    // outside the grid the same keys still commit
    await page.locator('.tab-body:not([hidden]) .cm-content').click()
    await page.keyboard.press(`${MOD}+Shift+c`)
    await expect(page.locator('.console .sql', { hasText: 'COMMIT' })).toHaveCount(1)
    await expect(pill).toHaveCount(0)
  })
})

test.describe('keyboard shortcuts help', () => {
  test('lists copy and paste in the grid', async ({ page }) => {
    await openConnection(page)
    await page.keyboard.press(`${MOD}+/`)
    const help = page.getByRole('dialog', { name: 'Keyboard shortcuts' })
    await expect(help.getByRole('row').filter({ hasText: 'Paste into the grid' })).toContainText('V')
    await expect(help.getByRole('row').filter({ hasText: 'Copy the selected cells' })).toContainText('Shift+C')
  })
})
