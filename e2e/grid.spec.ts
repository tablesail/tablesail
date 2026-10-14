import { test, expect } from './fixtures'
import {
  MOD,
  bottomBar,
  box,
  columnTexts,
  detailField,
  detailValue,
  grid,
  gridCell,
  gridHeader,
  gridRows,
  inspector,
  invokes,
  openConnection,
  openTable,
  openTableLoaded,
  recordInvokes,
  rowTexts,
  tabNamed,
} from './helpers'
import type { Page } from '@playwright/test'

const toggleDetails = (page: Page) => page.getByRole('button', { name: 'Toggle details panel' })
const quick = (page: Page) => bottomBar(page).getByPlaceholder('Filter rows…')
const count = (page: Page) => bottomBar(page).getByText(/^(≈|)[\d,]+\+? rows?$/)

/** Text of the first cell of the active row (the id column in every sample table). */
async function activeId(page: Page): Promise<string> {
  return ((await grid(page).locator('.row.cur .cell').first().textContent()) ?? '').trim()
}

const BIG_COLUMNS = ['id', 'uuid', 'name', 'amount', 'is_active', 'created_at', 'payload']

/**
 * Index (in big_table's column order) of the column the active-cell outline sits in, or -1 when no outline is
 * painted. Computed in one in-page call so a re-render between two lookups cannot make it flaky.
 */
async function activeColumn(page: Page): Promise<number> {
  const name = await grid(page).evaluate((g) => {
    const active = g.querySelector('.body .active')
    if (!active) return null
    const x = active.getBoundingClientRect().x + 2
    for (const h of g.querySelectorAll('.h')) {
      const r = h.getBoundingClientRect()
      if (x >= r.x - 1 && x < r.x + r.width) return h.querySelector('.hname')?.textContent ?? null
    }
    return null
  })
  return name === null ? -1 : BIG_COLUMNS.indexOf(name)
}

async function headerWidth(page: Page, name: string): Promise<number> {
  return (await box(gridHeader(page, name))).width
}

test.describe('grid basics', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('header shows column names with a PK badge, rows show formatted values', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await expect(grid(page).locator('.h')).toHaveCount(3)
    await expect(gridHeader(page, 'id')).toContainText('PK')
    await expect(gridHeader(page, 'email')).not.toContainText('PK')
    await expect(gridHeader(page, 'name')).toBeVisible()
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'a@example.com', 'Alice'])
    await expect.poll(() => rowTexts(page, 1)).toEqual(['2', 'b@example.com', 'Bob'])
  })

  test('typed values are formatted: NULL, booleans, decimals, timestamps', async ({ page }) => {
    await openTableLoaded(page, 'big_table')
    const row = await rowTexts(page, 0)
    expect(row[0]).toBe('1')
    expect(row[2]).toBe('row 1')
    expect(row[3]).toBe('0.01')
    expect(row[4]).toBe('false')
    expect(row[5]).toMatch(/^2020-01-01 00:01:00/)
    expect(row[6]).toBe('NULL')
    // amount is NULL every 10th row
    expect((await rowTexts(page, 9))[3]).toBe('NULL')
  })

  test('the bottom bar counts the rows: exact for small tables, approximate for huge ones, zero for empty', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await expect(count(page)).toContainText('2 rows')
    await openTable(page, 'big_table')
    await expect(count(page)).toContainText('≈1,000,000 rows')
    await openTable(page, 'empty_table')
    await expect(count(page)).toContainText('0 rows')
    await expect(grid(page).getByText('This table is empty')).toBeVisible()
  })

  test('the row count shows a query time too', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await expect(bottomBar(page).getByText(/^\d+ ms$|^<1 ms$/)).toBeVisible()
  })

  test('the gutter numbers rows from 1', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await expect(grid(page).locator('.g').nth(0)).toHaveText('1')
    await expect(grid(page).locator('.g').nth(1)).toHaveText('2')
  })

  test('a single row is "1 row", not "1 rows"', async ({ page }) => {
    // BUG: TableView.svelte countText is `${n} rows` unconditionally, so a one-row result reads "1 rows".
    await openTableLoaded(page, 'customers')
    await quick(page).fill('Bob')
    await quick(page).press('Enter')
    await expect(bottomBar(page).getByText('1 row', { exact: true })).toBeVisible({ timeout: 2000 })
  })

  test('read-only tables, views and connections say why they cannot be edited', async ({ page }) => {
    await openTableLoaded(page, 'no_pk')
    await expect(page.getByText('no primary key', { exact: false }).first()).toBeVisible()
    await expect(bottomBar(page).getByRole('button', { name: 'Row', exact: true })).toBeDisabled()

    await openTableLoaded(page, 'customer_totals')
    await expect(page.getByText("Views can't be edited.")).toBeVisible()
  })
})

test.describe('row selection and the Details panel', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('the Details panel is closed until a row is selected', async ({ page }) => {
    await expect(inspector(page)).toHaveCount(0)
  })

  test('clicking a row selects it and opens the Details panel automatically', async ({ page }) => {
    await gridCell(page, 0, 1).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
    await expect(grid(page).locator('.row.cur')).toHaveCount(1)
    await expect(grid(page).locator('.g.on')).toHaveText('1')
    await expect.poll(() => activeId(page)).toBe('1')
  })

  test('details list every field with its label, type and value', async ({ page }) => {
    await gridCell(page, 1, 0).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).locator('.field')).toHaveCount(3)
    for (const [name, type, value] of [
      ['id', 'integer', '2'],
      ['email', 'citext', 'b@example.com'],
      ['name', 'text', 'Bob'],
    ]) {
      const field = detailField(page, name)
      await expect(field.locator('.lname')).toHaveText(name)
      await expect(field.locator('.ltype')).toHaveText(type)
      await expect.poll(() => detailValue(page, name)).toBe(value)
    }
    await expect(inspector(page).getByText('public.customers')).toBeVisible()
  })

  test('the panel follows the selection', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await expect.poll(() => detailValue(page, 'name')).toBe('Alice')
    await gridCell(page, 1, 0).click()
    await expect.poll(() => detailValue(page, 'name')).toBe('Bob')
    await page.keyboard.press('ArrowUp')
    await expect.poll(() => detailValue(page, 'name')).toBe('Alice')
  })

  test('re-selecting the same row after closing the panel does not reopen it; a different row does', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await expect(inspector(page)).toBeVisible()
    await toggleDetails(page).click()
    await expect(inspector(page)).toHaveCount(0)

    await gridCell(page, 0, 2).click() // same row, other column
    await expect(page.getByRole('grid').locator('.row.cur')).toHaveCount(1)
    await expect(inspector(page)).toHaveCount(0)
    await page.keyboard.press('ArrowRight') // still the same row
    await expect(inspector(page)).toHaveCount(0)

    await gridCell(page, 1, 0).click() // a different row
    await expect(inspector(page)).toBeVisible()
    await expect.poll(() => detailValue(page, 'name')).toBe('Bob')
  })

  test('the panel can be closed and stays closed while the same row stays selected', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await toggleDetails(page).click()
    await expect(inspector(page)).toHaveCount(0)
    await page.waitForTimeout(300)
    await expect(inspector(page)).toHaveCount(0)
  })

  test('the field search narrows the list', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await inspector(page).getByPlaceholder('Search for field…').fill('mai')
    await expect(inspector(page).locator('.field')).toHaveCount(1)
    await expect(detailField(page, 'email')).toBeVisible()
    await inspector(page).getByPlaceholder('Search for field…').fill('zzz')
    await expect(inspector(page).getByText('No matching fields.')).toBeVisible()
  })

  test('opening another tab shows that tab’s row, not the previous one', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await openTable(page, 'orders')
    await expect(inspector(page).getByText('No row selected')).toBeVisible()
    await gridCell(page, 0, 0).click()
    await expect(detailField(page, 'total')).toBeVisible()
    await tabNamed(page, 'customers').click()
    await expect(detailField(page, 'email')).toBeVisible()
  })

  test('foreign key fields offer a jump to the related row', async ({ page }) => {
    await openTableLoaded(page, 'orders')
    await gridCell(page, 0, 1).click()
    // ^: the field's "Field options for customer_id" button also contains the column name.
    const related = inspector(page).getByRole('button', { name: /^customer_id/ })
    await expect(related).toContainText('→ customers')
    await related.click()
    await expect(tabNamed(page, 'customers (filtered)')).toHaveCount(1)
    await expect(page.getByText(/customers\b/).first()).toBeVisible()
  })
})

test.describe('JSON values in the Details panel', () => {
  test('a JSON field is pretty-printed and its options menu offers Pretty, Minify and Copy', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 2, 6).click() // row 3 is the first with a payload
    await expect(inspector(page)).toBeVisible()
    const field = detailField(page, 'payload')
    await expect(field.locator('.ltype')).toHaveText('jsonb')

    const body = field.locator('pre.value, textarea.value')
    await expect.poll(async () => ((await body.first().evaluate((el) => (el as HTMLTextAreaElement).value ?? el.textContent)) as string)).toContain('\n')
    const text = (await body.first().evaluate((el) => (el as HTMLTextAreaElement).value ?? el.textContent)) as string
    expect(JSON.parse(text)).toEqual({ n: 3, tags: ['a', 'b'] })
    expect(text).toMatch(/^\{\n {2}"n": 3,/)

    await field.getByRole('button', { name: 'Field options' }).click()
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toContain('✓  Pretty')
    expect(items).toContain('Minify')
    expect(items).toContain('Copy value')
  })

  test('Minify puts the JSON on one line, Pretty restores it, Copy value fills the clipboard', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 2, 6).click()
    const field = detailField(page, 'payload')
    const body = field.locator('pre.value, textarea.value')
    const read = () => body.first().evaluate((el) => ((el as HTMLTextAreaElement).value ?? el.textContent) as string)

    await field.getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Minify' }).click()
    await expect.poll(read).toBe('{"n":3,"tags":["a","b"]}')

    await field.getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: /Pretty/ }).click()
    await expect.poll(read).toContain('\n  "n": 3')

    await field.getByRole('button', { name: 'Field options' }).click()
    await page.getByRole('menuitem', { name: 'Copy value' }).click()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('"n": 3')
  })

  test('on a read-only connection JSON is shown highlighted but is not editable', async ({ page }) => {
    await openConnection(page, 'Production')
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 2, 6).click()
    const field = detailField(page, 'payload')
    await expect(field.locator('pre.value.json')).toBeVisible()
    await expect(field.locator('textarea')).toHaveCount(0)
    await field.getByRole('button', { name: 'Field options' }).click()
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).not.toContain('Set NULL')
    expect(items).toContain('Minify')
  })

  test('a NULL JSON field shows NULL', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 6).click()
    await expect.poll(() => detailValue(page, 'payload')).toMatch(/NULL|^$/)
  })
})

test.describe('keyboard navigation', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
    await gridCell(page, 0, 0).click()
    await expect(grid(page)).toBeFocused()
    // The first selection opens the Details panel, which narrows the grid; let that settle before typing.
    await expect(inspector(page)).toBeVisible()
    await expect.poll(() => grid(page).evaluate((el) => el.clientWidth)).toBeLessThan(1000)
  })

  test('arrow keys move the active cell', async ({ page }) => {
    await expect.poll(() => activeId(page)).toBe('1')
    await expect.poll(() => activeColumn(page)).toBe(0)
    await page.keyboard.press('ArrowDown')
    await expect.poll(() => activeId(page)).toBe('2')
    await page.keyboard.press('ArrowDown')
    await expect.poll(() => activeId(page)).toBe('3')
    await page.keyboard.press('ArrowUp')
    await expect.poll(() => activeId(page)).toBe('2')
    await page.keyboard.press('ArrowRight')
    await page.keyboard.press('ArrowRight')
    await expect.poll(() => activeColumn(page)).toBe(2)
    await page.keyboard.press('ArrowLeft')
    await expect.poll(() => activeColumn(page)).toBe(1)
  })

  test('the selection stops at the edges instead of wrapping', async ({ page }) => {
    await page.keyboard.press('ArrowUp')
    await expect.poll(() => activeId(page)).toBe('1')
    await page.keyboard.press('ArrowLeft')
    await expect.poll(() => activeColumn(page)).toBe(0)
    await page.keyboard.press(`${MOD}+ArrowRight`)
    await expect.poll(() => activeColumn(page)).toBe(BIG_COLUMNS.length - 1)
    await page.keyboard.press('ArrowRight')
    await expect.poll(() => activeColumn(page)).toBe(BIG_COLUMNS.length - 1)
  })

  test('PageDown and PageUp move by about a screenful', async ({ page }) => {
    await page.keyboard.press('PageDown')
    const afterDown = Number(await activeId(page))
    expect(afterDown).toBeGreaterThan(15)
    expect(afterDown).toBeLessThan(60)
    await page.keyboard.press('PageUp')
    await expect.poll(() => activeId(page)).toBe('1')
  })

  test('Home/End jump to the first/last column; ⌘↓ and ⌘↑ jump to the last and first row', async ({ page }) => {
    await page.keyboard.press('End')
    await expect.poll(() => activeColumn(page)).toBe(6)
    await page.keyboard.press('Home')
    await expect.poll(() => activeColumn(page)).toBe(0)

    await page.keyboard.press(`${MOD}+ArrowDown`)
    await expect.poll(() => activeId(page)).toBe('1000000')
    await page.keyboard.press(`${MOD}+ArrowUp`)
    await expect.poll(() => activeId(page)).toBe('1')
  })

  test('Tab and Shift+Tab move along the row and wrap to the next/previous row', async ({ page }) => {
    await page.keyboard.press('Tab')
    await expect.poll(() => activeColumn(page)).toBe(1)
    await page.keyboard.press('Shift+Tab')
    await expect.poll(() => activeColumn(page)).toBe(0)
    await page.keyboard.press('End')
    await page.keyboard.press('Tab')
    await expect.poll(() => activeColumn(page)).toBe(0)
    await expect.poll(() => activeId(page)).toBe('2')
    await page.keyboard.press('Shift+Tab')
    await expect.poll(() => activeColumn(page)).toBe(6)
    await expect.poll(() => activeId(page)).toBe('1')
  })

  test('Shift+arrows extend the selection and the bottom bar says how big it is', async ({ page }) => {
    await page.keyboard.press('Shift+ArrowDown')
    await page.keyboard.press('Shift+ArrowDown')
    await page.keyboard.press('Shift+ArrowRight')
    await expect(bottomBar(page).getByText('3 × 2 selected')).toBeVisible()
    await page.keyboard.press('Shift+ArrowUp')
    await expect(bottomBar(page).getByText('2 × 2 selected')).toBeVisible()
  })

  test('⌘A selects every cell', async ({ page }) => {
    await page.keyboard.press(`${MOD}+a`)
    await expect(bottomBar(page).getByText(/^[\d,]+ × 7 selected$/)).toBeVisible()
  })

  test('clicking a row number selects the whole row; clicking # selects everything', async ({ page }) => {
    await grid(page).locator('.g').nth(2).click()
    await expect(bottomBar(page).getByText('1 × 7 selected')).toBeVisible()
    await grid(page).locator('.corner').click()
    await expect(bottomBar(page).getByText(/ × 7 selected$/)).toBeVisible()
  })

  test('shift-click extends from the anchor cell', async ({ page }) => {
    await gridCell(page, 3, 2).click({ modifiers: ['Shift'] })
    await expect(bottomBar(page).getByText('4 × 3 selected')).toBeVisible()
  })

  test('dragging across cells selects a rectangle', async ({ page }) => {
    const a = await box(gridCell(page, 1, 1))
    const b = await box(gridCell(page, 3, 3))
    await page.mouse.move(a.x + 8, a.y + 8)
    await page.mouse.down()
    await page.mouse.move(b.x + 8, b.y + 8, { steps: 8 })
    await page.mouse.up()
    await expect(bottomBar(page).getByText('3 × 3 selected')).toBeVisible()
  })

  test('arrow keys do not scroll the page behind the grid', async ({ page }) => {
    for (let i = 0; i < 30; i++) await page.keyboard.press('ArrowDown')
    expect(await page.evaluate(() => window.scrollY)).toBe(0)
    await expect.poll(() => activeId(page)).toBe('31')
  })
})

test.describe('copy', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('⌘C copies the selected cell as text', async ({ page }) => {
    await gridCell(page, 0, 2).click()
    await page.keyboard.press(`${MOD}+c`)
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('Alice')
  })

  test('⌘C on a range copies tab-separated rows', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await page.keyboard.press('Shift+ArrowDown')
    await page.keyboard.press('Shift+ArrowRight')
    await page.keyboard.press('Shift+ArrowRight')
    await page.keyboard.press(`${MOD}+c`)
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('1\ta@example.com\tAlice\n2\tb@example.com\tBob')
  })

  test('the cell menu has Copy and Copy with headers', async ({ page }) => {
    await gridCell(page, 1, 1).click({ button: 'right' })
    await page.getByRole('menuitem', { name: /Copy with headers/ }).click()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('email\nb@example.com')
  })

  test('the cell context menu lists the editing and copy actions', async ({ page }) => {
    await gridCell(page, 0, 1).click({ button: 'right' })
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items.map((t) => t.replace(/\s+.*$/, ''))).toEqual(['Edit', 'Set', 'Copy', 'Copy', 'Copy', 'Paste', 'Filter', 'Duplicate', 'Delete'])
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
  })

  test('Filter by this value from the cell menu filters the table', async ({ page }) => {
    await gridCell(page, 1, 2).click({ button: 'right' })
    await page.getByRole('menuitem', { name: /Filter by this value/ }).click()
    await expect(count(page)).toContainText('1 row')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Bob'])
    await expect(bottomBar(page).getByRole('button', { name: /Filters/ })).toContainText('1')
  })
})

test.describe('sorting', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('clicking a header sorts ascending, then descending, then off', async ({ page }) => {
    await recordInvokes(page)
    await expect(gridHeader(page, 'name').locator('.sort')).toHaveCount(0)

    await gridHeader(page, 'name').click()
    await expect(gridHeader(page, 'name').locator('.sort')).toHaveText('▲')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Alice', 'Bob'])

    await gridHeader(page, 'name').click()
    await expect(gridHeader(page, 'name').locator('.sort')).toHaveText('▼')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Bob', 'Alice'])

    await gridHeader(page, 'name').click()
    await expect(gridHeader(page, 'name').locator('.sort')).toHaveCount(0)
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Alice', 'Bob'])

    const sorts = (await invokes(page, 'browse_page')).map((c) => (c.args.request as { sort: unknown[] }).sort)
    expect(sorts).toEqual([[{ column: 'name', descending: false }], [{ column: 'name', descending: true }], []])
  })

  test('sorting another column replaces the first sort', async ({ page }) => {
    await gridHeader(page, 'name').click()
    await gridHeader(page, 'id').click()
    await expect(gridHeader(page, 'id').locator('.sort')).toHaveText('▲')
    await expect(gridHeader(page, 'name').locator('.sort')).toHaveCount(0)
    await gridHeader(page, 'id').click()
    await expect.poll(() => columnTexts(page, 0)).toEqual(['2', '1'])
  })

  test('sorting numbers sorts numerically, not alphabetically', async ({ page }) => {
    await openTableLoaded(page, 'orders')
    await gridHeader(page, 'id').click()
    await gridHeader(page, 'id').click()
    await expect.poll(() => columnTexts(page, 0, 3)).toEqual(['3', '2', '1'])
  })
})

test.describe('column sizing', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('dragging the header edge resizes a column and a minimum width is enforced', async ({ page }) => {
    const before = await headerWidth(page, 'email')
    const handle = gridHeader(page, 'email').locator('.resize')
    const h = await box(handle)
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2)
    await page.mouse.down()
    await page.mouse.move(h.x + h.width / 2 + 90, h.y + h.height / 2, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => headerWidth(page, 'email')).toBeGreaterThan(before + 70)

    const h2 = await box(gridHeader(page, 'email').locator('.resize'))
    await page.mouse.move(h2.x + h2.width / 2, h2.y + h2.height / 2)
    await page.mouse.down()
    await page.mouse.move(h2.x - 600, h2.y + h2.height / 2, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => headerWidth(page, 'email')).toBeLessThanOrEqual(60)
    expect(await headerWidth(page, 'email')).toBeGreaterThanOrEqual(46)
  })

  test('resizing a column does not sort it', async ({ page }) => {
    const h = await box(gridHeader(page, 'email').locator('.resize'))
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2)
    await page.mouse.down()
    await page.mouse.move(h.x + h.width / 2 + 40, h.y + h.height / 2, { steps: 4 })
    await page.mouse.up()
    await expect(gridHeader(page, 'email').locator('.sort')).toHaveCount(0)
  })

  test('double-clicking the header edge fits the column to its content', async ({ page }) => {
    const handle = gridHeader(page, 'email').locator('.resize')
    const h = await box(handle)
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2)
    await page.mouse.down()
    await page.mouse.move(h.x + h.width / 2 + 200, h.y + h.height / 2, { steps: 6 })
    await page.mouse.up()
    const wide = await headerWidth(page, 'email')
    await gridHeader(page, 'email').locator('.resize').dblclick()
    await expect.poll(() => headerWidth(page, 'email')).toBeLessThan(wide - 100)
    // Still wide enough to read "a@example.com" without ellipsis.
    expect(await headerWidth(page, 'email')).toBeGreaterThan(100)
  })
})

test.describe('quick filter', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('Enter applies a text filter across text columns, clearing the box restores every row', async ({ page }) => {
    await quick(page).fill('bob')
    await quick(page).press('Enter')
    await expect(count(page)).toContainText('1 row')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Bob'])

    await quick(page).fill('')
    await expect(count(page)).toContainText('2 rows')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Alice', 'Bob'])
  })

  test('matching nothing shows an empty-state message', async ({ page }) => {
    await quick(page).fill('zzzz')
    await quick(page).press('Enter')
    await expect(grid(page).getByText('No rows match the filter')).toBeVisible()
    await expect(count(page)).toContainText('0 rows')
  })

  test('matches on any text column', async ({ page }) => {
    await quick(page).fill('a@example')
    await quick(page).press('Enter')
    await expect.poll(() => columnTexts(page, 1)).toEqual(['a@example.com'])
  })

  test('the box is a search field with a placeholder and is not applied while typing', async ({ page }) => {
    await recordInvokes(page)
    await quick(page).pressSequentially('bo')
    await page.waitForTimeout(400)
    const filtered = (await invokes(page, 'browse_page')).filter((c) => (c.args.request as { filters: unknown[] }).filters.length > 0)
    expect(filtered).toHaveLength(0)
  })
})

test.describe('filters bar', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  const filtersButton = (page: Page) => bottomBar(page).getByRole('button', { name: /Filters/ })
  const bar = (page: Page) => page.locator('.tab-body:not([hidden]) .filters')

  test('Filters opens a bar with a first row ready; ⌘F toggles it too', async ({ page }) => {
    await expect(bar(page)).toHaveCount(0)
    await filtersButton(page).click()
    await expect(bar(page)).toBeVisible()
    await expect(bar(page).getByRole('combobox', { name: 'Column' })).toHaveValue('id')
    await expect(bar(page).getByRole('combobox', { name: 'Operator' })).toHaveValue('eq')
    await filtersButton(page).click()
    await expect(bar(page)).toHaveCount(0)
    await page.keyboard.press(`${MOD}+f`)
    await expect(bar(page)).toBeVisible()
    await page.keyboard.press(`${MOD}+f`)
    await expect(bar(page)).toHaveCount(0)
  })

  test('apply a column/operator/value filter; the button shows how many are active', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Column' }).selectOption('name')
    await bar(page).getByRole('combobox', { name: 'Operator' }).selectOption('contains')
    await bar(page).getByPlaceholder('value').fill('ali')
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await expect(count(page)).toContainText('1 row')
    await expect.poll(() => columnTexts(page, 2)).toEqual(['Alice'])
    await expect(filtersButton(page).locator('.count')).toHaveText('1')
  })

  test('Enter in the value box applies; the filter value is not applied before that', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Column' }).selectOption('email')
    await bar(page).getByRole('combobox', { name: 'Operator' }).selectOption('startsWith')
    await bar(page).getByPlaceholder('value').fill('b@')
    await expect(count(page)).toContainText('2 rows')
    await bar(page).getByPlaceholder('value').press('Enter')
    await expect(count(page)).toContainText('1 row')
  })

  test('operators without a value (is NULL) hide the value box', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Operator' }).selectOption('isNull')
    await expect(bar(page).getByPlaceholder('value')).toHaveCount(0)
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await expect(grid(page).getByText('No rows match the filter')).toBeVisible()
  })

  test('two filters combine with Match all / any', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Column' }).selectOption('name')
    await bar(page).getByPlaceholder('value').fill('Alice')
    await bar(page).getByRole('button', { name: '+ Filter' }).click()
    await expect(bar(page).getByRole('combobox', { name: 'Column' })).toHaveCount(2)
    await bar(page).getByRole('combobox', { name: 'Column' }).nth(1).selectOption('name')
    await bar(page).getByPlaceholder('value').nth(1).fill('Bob')
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await expect(count(page)).toContainText('0 rows')

    await bar(page).getByLabel('Match').selectOption({ label: 'any' })
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await expect(count(page)).toContainText('2 rows')
  })

  test('removing a filter row and Clear both reset the table', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Column' }).selectOption('name')
    await bar(page).getByPlaceholder('value').fill('Bob')
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await expect(count(page)).toContainText('1 row')

    await bar(page).getByRole('button', { name: 'Clear' }).click()
    await expect(count(page)).toContainText('2 rows')
    await expect(filtersButton(page).locator('.count')).toHaveCount(0)

    await bar(page).getByRole('button', { name: '+ Filter' }).click()
    await expect(bar(page).getByRole('combobox', { name: 'Column' })).toHaveCount(1)
    await bar(page).getByRole('button', { name: 'Remove filter' }).click()
    await expect(bar(page).getByRole('combobox', { name: 'Column' })).toHaveCount(0)
  })

  test('a filtered table can be exported with the filter (the export dialog says so)', async ({ page }) => {
    await filtersButton(page).click()
    await bar(page).getByRole('combobox', { name: 'Column' }).selectOption('name')
    await bar(page).getByPlaceholder('value').fill('Bob')
    await bar(page).getByRole('button', { name: 'Apply' }).click()
    await bottomBar(page).getByRole('button', { name: 'Export' }).click()
    await expect(page.getByRole('dialog', { name: 'Export' })).toContainText('with the current filter')
  })
})

test.describe('structure and relations', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('the Structure tab lists columns, indexes and foreign keys, and the DDL', async ({ page }) => {
    await openTableLoaded(page, 'orders')
    await bottomBar(page).getByRole('tab', { name: 'Structure' }).click()
    await expect(bottomBar(page).getByRole('tab', { name: 'Structure' })).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByRole('heading', { name: /Columns\s*5/ })).toBeVisible()
    const cols = page.getByRole('table').first()
    await expect(cols.getByRole('row')).toHaveCount(6) // header + 5 columns
    await expect(cols.getByRole('cell', { name: 'customer_id' })).toBeVisible()
    await expect(cols.getByRole('cell', { name: 'numeric(12,2)' })).toBeVisible()
    await expect(page.getByRole('heading', { name: /Indexes/ })).toBeVisible()
    await expect(page.getByText('orders_pkey')).toBeVisible()
    await expect(page.getByRole('heading', { name: /Foreign keys\s*1/ })).toBeVisible()
    await expect(page.getByText('orders_customer_id_fkey')).toBeVisible()
    await expect(page.getByText('CREATE TABLE public.orders')).toBeVisible()
    await expect(bottomBar(page).getByText('5 columns · 2 indexes · 1 foreign keys')).toBeVisible()
  })

  test('Structure and Data are separate tabs: switching back shows the grid again', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await bottomBar(page).getByRole('tab', { name: 'Structure' }).click()
    await expect(grid(page)).toHaveCount(0)
    await bottomBar(page).getByRole('tab', { name: 'Data' }).click()
    await expect(grid(page)).toBeVisible()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('the Relations tab draws the table and what it references; the target opens that table', async ({ page }) => {
    await openTableLoaded(page, 'orders')
    await bottomBar(page).getByRole('tab', { name: 'Relations' }).click()
    await expect(page.getByText('Relationships')).toBeVisible()
    await expect(page.getByText('1 foreign key', { exact: true })).toBeVisible()
    const target = page.getByRole('button', { name: /public\.customers|customers/ }).filter({ hasText: 'on delete cascade' })
    await expect(target).toBeVisible()
    await target.click()
    await expect(tabNamed(page, 'customers')).toBeVisible()
  })

  test('a table without foreign keys says so', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await bottomBar(page).getByRole('tab', { name: 'Relations' }).click()
    await expect(page.getByText('No foreign keys on this table')).toBeVisible()
  })

  test('the bottom-bar actions that need a grid disappear on Structure', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await bottomBar(page).getByRole('tab', { name: 'Structure' }).click()
    await expect(bottomBar(page).getByRole('button', { name: 'Row', exact: true })).toHaveCount(0)
    await expect(bottomBar(page).getByPlaceholder('Filter rows…')).toHaveCount(0)
    await expect(bottomBar(page).getByRole('button', { name: 'Columns' })).toHaveCount(0)
  })
})

test.describe('columns button (hide columns)', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  const columnsButton = (page: Page) => bottomBar(page).getByRole('button', { name: /Columns/ })
  const popover = (page: Page) => page.getByRole('dialog', { name: 'Columns' })

  test('lists every column with a checkbox, primary keys fixed on', async ({ page }) => {
    await columnsButton(page).click()
    await expect(popover(page)).toBeVisible()
    await expect(popover(page).getByRole('checkbox')).toHaveCount(3)
    const id = popover(page).getByRole('checkbox', { name: /^id/ })
    await expect(id).toBeChecked()
    await expect(id).toBeDisabled()
    await expect(popover(page).getByRole('checkbox', { name: /^email/ })).toBeChecked()
    await expect(popover(page).getByRole('checkbox', { name: /^email/ })).toBeEnabled()
    await expect(popover(page).getByRole('button', { name: 'Show all' })).toBeDisabled()
  })

  test('unchecking a column hides it from the grid, shows a count badge and asks the server for the rest', async ({ page }) => {
    await recordInvokes(page)
    await columnsButton(page).click()
    await popover(page).getByRole('checkbox', { name: /^email/ }).uncheck()
    await expect(grid(page).locator('.h')).toHaveCount(2)
    await expect(gridHeader(page, 'email')).toHaveCount(0)
    await expect.poll(() => rowTexts(page, 0)).toEqual(['1', 'Alice'])
    await expect(columnsButton(page).locator('.count')).toHaveText('1')

    const pages = await invokes(page, 'browse_page')
    expect((pages.at(-1)!.args.request as { columns: string[] }).columns).toEqual(['id', 'name'])
    await expect(popover(page).getByRole('button', { name: 'Show all' })).toBeEnabled()
  })

  test('Show all brings every column back', async ({ page }) => {
    await columnsButton(page).click()
    await popover(page).getByRole('checkbox', { name: /^email/ }).uncheck()
    await popover(page).getByRole('checkbox', { name: /^name/ }).uncheck()
    await expect(grid(page).locator('.h')).toHaveCount(1)
    await expect(columnsButton(page).locator('.count')).toHaveText('2')
    await popover(page).getByRole('button', { name: 'Show all' }).click()
    await expect(grid(page).locator('.h')).toHaveCount(3)
    await expect(columnsButton(page).locator('.count')).toHaveCount(0)
  })

  test('the choice is remembered per table across reloads', async ({ page }) => {
    await columnsButton(page).click()
    await popover(page).getByRole('checkbox', { name: /^email/ }).uncheck()
    const key = await page.evaluate(() => Object.keys(localStorage).find((k) => k.startsWith('tablesail.hiddenColumns.')))
    expect(key).toBe('tablesail.hiddenColumns.c-pg.tablesail.public.customers')

    await page.reload()
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await expect(gridHeader(page, 'email')).toHaveCount(0)
    await expect(columnsButton(page).locator('.count')).toHaveText('1')
    // Another table is unaffected.
    await openTableLoaded(page, 'orders')
    await expect(columnsButton(page).locator('.count')).toHaveCount(0)
  })

  test('clicking outside closes the popover', async ({ page }) => {
    await columnsButton(page).click()
    await expect(popover(page)).toBeVisible()
    await page.mouse.click(700, 300)
    await expect(popover(page)).toHaveCount(0)
  })

  test('Escape closes the popover', async ({ page }) => {
    // BUG: the Columns popover (role=dialog) has no Escape handling or focus management (TableView.svelte .colpop).
    await columnsButton(page).click()
    await expect(popover(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(popover(page)).toHaveCount(0, { timeout: 1500 })
  })

  test('hiding a column keeps the first-row details panel working', async ({ page }) => {
    await columnsButton(page).click()
    await popover(page).getByRole('checkbox', { name: /^email/ }).uncheck()
    await page.mouse.click(700, 300)
    await gridCell(page, 0, 0).click()
    await expect(detailField(page, 'name')).toBeVisible()
    await expect(detailField(page, 'email')).toHaveCount(0)
  })
})

test.describe('huge tables (virtual scrolling)', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'big_table')
  })

  test('the clicked cell stays in view when the first selection opens the Details panel', async ({ page }) => {
    // BUG: the grid narrows by 300px when the Details panel opens on the first click, but it does not
    // re-run ensureVisible for the selected cell, so a cell near the right edge is pushed out of view
    // (DataGrid.svelte: no scroll-into-view on resize). Pressing End right after that click has the same
    // problem: it selects the last column without scrolling to it.
    await page.setViewportSize({ width: 1200, height: 800 })
    const target = gridCell(page, 0, 5) // created_at: visible while the panel is closed
    await target.click()
    await expect(inspector(page)).toBeVisible()
    await expect.poll(() => grid(page).evaluate((el) => el.clientWidth), { timeout: 2000 }).toBeLessThan(900)
    const g = await box(grid(page))
    const a = await box(target)
    expect(a.x + a.width / 2).toBeLessThan(g.x + g.width)
  })

  test('only the visible rows are in the DOM', async ({ page }) => {
    const n = await gridRows(page).count()
    expect(n).toBeGreaterThan(15)
    expect(n).toBeLessThan(80)
    await expect(count(page)).toContainText('≈1,000,000 rows')
  })

  test('jumping to the last row paints it quickly', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    const t0 = Date.now()
    await page.keyboard.press(`${MOD}+ArrowDown`)
    await expect(grid(page).locator('.cell', { hasText: /^1000000$/ })).toBeVisible()
    expect(Date.now() - t0).toBeLessThan(3000)
    // The bottom rows hold real data, not placeholders.
    await expect(grid(page).locator('.row').last().locator('.cell.ph')).toHaveCount(0)
    await expect(grid(page).locator('.cell', { hasText: 'row 1000000' })).toBeVisible()
  })

  test('wheel scrolling to the bottom keeps rendering rows', async ({ page }) => {
    await grid(page).hover()
    for (let i = 0; i < 12; i++) await page.mouse.wheel(0, 600_000)
    await expect.poll(async () => Number(((await gridRows(page).last().locator('.cell').first().textContent()) ?? '0').trim()), { timeout: 10_000 }).toBeGreaterThan(100_000)
    await expect(grid(page).locator('.row .cell:not(.ph)').first()).toBeVisible()
    expect(await gridRows(page).count()).toBeLessThan(80)
  })

  test('scrolling and selecting deep in the table works, and the Details panel follows', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    await page.keyboard.press(`${MOD}+ArrowDown`)
    await expect.poll(() => activeId(page)).toBe('1000000')
    await expect.poll(() => detailValue(page, 'name')).toBe('row 1000000')
    await page.keyboard.press('ArrowUp')
    await expect.poll(() => activeId(page)).toBe('999999')
  })

  test('PageDown through many pages stays responsive', async ({ page }) => {
    await gridCell(page, 0, 0).click()
    const t0 = Date.now()
    for (let i = 0; i < 40; i++) await page.keyboard.press('PageDown')
    await expect.poll(async () => Number(await activeId(page))).toBeGreaterThan(900)
    expect(Date.now() - t0).toBeLessThan(8000)
  })

  test('a horizontal scroll is possible on a wide table and the header scrolls with it', async ({ page }) => {
    await openTableLoaded(page, 'wide_table')
    await page.setViewportSize({ width: 800, height: 700 })
    await gridCell(page, 0, 0).click()
    await page.keyboard.press('End')
    // The last column becomes visible and its header is inside the grid viewport.
    const g = await box(grid(page))
    const lastHeader = await box(grid(page).locator('.h').last())
    expect(lastHeader.x).toBeLessThan(g.x + g.width)
  })
})
