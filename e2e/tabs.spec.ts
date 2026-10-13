import { test, expect } from './fixtures'
import {
  MOD,
  box,
  card,
  gridCell,
  gridHeader,
  inspector,
  newQueryTab,
  setSql,
  sqlText,
  openConnection,
  openTable,
  openTableLoaded,
  showConnections,
  sidebar,
  tabNamed,
  tabTitles,
  tableRow,
} from './helpers'
import type { Page } from '@playwright/test'

const prev = (page: Page) => page.getByRole('button', { name: 'Previous tab' })
const next = (page: Page) => page.getByRole('button', { name: 'Next tab' })
const label = (page: Page, title: string) => tabNamed(page, title).locator('.t')

/** A tab's x button; it sits next to the tab (not inside it) and is named "Close tab <title>". */
const closeButton = (page: Page, title: string) => page.getByRole('button', { name: `Close tab ${title}`, exact: true })

test.describe('table tabs', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('opening a table adds an active tab titled with the table name and a tooltip with its schema', async ({ page }) => {
    await openTable(page, 'customers')
    const tab = tabNamed(page, 'customers')
    await expect(tab).toHaveAttribute('aria-selected', 'true')
    await expect(tab).toHaveAttribute('title', 'public.customers')
    await expect(page.getByRole('grid')).toBeVisible()
    await expect(page.getByRole('tab', { name: 'Data', exact: true })).toHaveAttribute('aria-selected', 'true')
  })

  test('tabs share the strip in equal widths and always fit inside it', async ({ page }) => {
    for (const t of ['customers', 'orders', 'events']) await openTable(page, t)
    const widths: number[] = []
    for (const t of ['customers', 'orders', 'events']) widths.push((await box(tabNamed(page, t))).width)
    expect(Math.max(...widths) - Math.min(...widths)).toBeLessThanOrEqual(1.5)
    // Three tabs of a wide window are reasonably wide, not squashed.
    expect(widths[0]).toBeGreaterThan(120)

    const strip = await box(page.locator('.strip'))
    const first = await box(tabNamed(page, 'customers'))
    const last = await box(tabNamed(page, 'events'))
    expect(first.x).toBeGreaterThanOrEqual(strip.x - 1)
    expect(last.x + last.width).toBeLessThanOrEqual(strip.x + strip.width + 1)
  })

  test('with many tabs they never overlap each other', async ({ page }) => {
    const names = ['all_types', 'big_table', 'categories', 'composite_pk', 'computed', 'customers', 'empty_table', 'events', 'no_pk', 'orders', 'select', 'unique_only']
    for (const t of names) await openTable(page, t)
    await expect(page.locator('.strip [role=tab]')).toHaveCount(12)
    let prevRight = -Infinity
    for (const t of names) {
      const b = await box(tabNamed(page, t))
      expect(b.x, `${t} starts after the previous tab`).toBeGreaterThanOrEqual(prevRight - 1)
      expect(b.width).toBeGreaterThan(30)
      prevRight = b.x + b.width
    }
  })

  test('the active tab is always visible inside the strip, however many tabs are open', async ({ page }) => {
    // BUG: .tab has min-width:110px and .strip scrolls with a hidden scrollbar, so from about the 11th tab on the
    // newly opened (active) tab sits beyond the right edge and nothing scrolls it into view (TabBar.svelte).
    const names = ['all_types', 'big_table', 'categories', 'composite_pk', 'computed', 'customers', 'empty_table', 'events', 'no_pk', 'orders', 'select', 'unique_only']
    for (const t of names) await openTable(page, t)
    const strip = await box(page.locator('.strip'))
    const active = await box(tabNamed(page, 'unique_only'))
    expect(active.x + active.width).toBeLessThanOrEqual(strip.x + strip.width + 1)
  })

  test('tabs keep the order they were opened in', async ({ page }) => {
    for (const t of ['orders', 'customers', 'events']) await openTable(page, t)
    expect(await tabTitles(page)).toEqual(['orders', 'customers', 'events'])
  })

  test('previous/next arrows step between tabs and disable at the ends', async ({ page }) => {
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await openTable(page, 'events')
    await expect(tabNamed(page, 'events')).toHaveAttribute('aria-selected', 'true')
    await expect(next(page)).toBeDisabled()
    await expect(prev(page)).toBeEnabled()

    await prev(page).click()
    await expect(tabNamed(page, 'orders')).toHaveAttribute('aria-selected', 'true')
    await expect(next(page)).toBeEnabled()
    await prev(page).click()
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
    await expect(prev(page)).toBeDisabled()
    await next(page).click()
    await expect(tabNamed(page, 'orders')).toHaveAttribute('aria-selected', 'true')
  })

  test('arrows are disabled with zero or one tab', async ({ page }) => {
    await expect(prev(page)).toBeDisabled()
    await expect(next(page)).toBeDisabled()
    await openTable(page, 'customers')
    await expect(prev(page)).toBeDisabled()
    await expect(next(page)).toBeDisabled()
  })

  test('the x button closes a tab and activates its neighbour', async ({ page }) => {
    for (const t of ['customers', 'orders', 'events']) await openTable(page, t)
    await tabNamed(page, 'orders').click()
    await closeButton(page, 'orders').click()
    await expect(tabNamed(page, 'orders')).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['customers', 'events'])
    await expect(tabNamed(page, 'events')).toHaveAttribute('aria-selected', 'true')
  })

  test('closing an inactive tab leaves the active one alone', async ({ page }) => {
    for (const t of ['customers', 'orders']) await openTable(page, t)
    await closeButton(page, 'customers').click()
    expect(await tabTitles(page)).toEqual(['orders'])
    await expect(tabNamed(page, 'orders')).toHaveAttribute('aria-selected', 'true')
  })

  test('middle-click closes a tab', async ({ page }) => {
    for (const t of ['customers', 'orders']) await openTable(page, t)
    await tabNamed(page, 'customers').click({ button: 'middle' })
    await expect(tabNamed(page, 'customers')).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['orders'])
  })

  test('closing the last tab returns to the empty workspace', async ({ page }) => {
    await openTable(page, 'customers')
    await closeButton(page, 'customers').click()
    await expect(page.getByRole('tab', { name: /customers/ })).toHaveCount(0)
    await expect(page.getByText('Pick a table in the sidebar, or start a query.')).toBeVisible()
    await expect(page.getByRole('grid')).toHaveCount(0)
  })

  test('opening the same table again focuses the existing tab instead of adding one', async ({ page }) => {
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await tableRow(page, 'customers').dblclick()
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('the same table from the command palette also reuses its tab', async ({ page }) => {
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await page.keyboard.press(`${MOD}+k`)
    await page.getByPlaceholder('What would you like to do?').fill('customers')
    await page.keyboard.press('Enter')
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('each tab keeps its own state (filter text, scroll) when switching away and back', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    const quick = () => page.locator('.tab-body:not([hidden])').getByPlaceholder('Filter rows…')
    await quick().fill('Alice')
    await page.keyboard.press('Enter')
    await openTable(page, 'orders')
    await expect(quick()).toHaveValue('')
    await tabNamed(page, 'customers').click()
    await expect(quick()).toHaveValue('Alice')
  })

  test('⌘T opens a numbered query tab and focuses its editor', async ({ page }) => {
    await newQueryTab(page)
    await expect(tabNamed(page, 'Query 1')).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press(`${MOD}+t`)
    await expect(tabNamed(page, 'Query 2')).toHaveAttribute('aria-selected', 'true')
    expect(await tabTitles(page)).toEqual(['Query 1', 'Query 2'])
    await expect(page.locator('.tab-body:not([hidden]) .cm-content')).toBeFocused()
  })

  test('the + button, the toolbar SQL button and the sidebar + each open a query tab', async ({ page }) => {
    await page.getByRole('button', { name: 'New query' }).first().click() // tab strip or sidebar, whichever is first
    await page.getByRole('button', { name: 'New SQL query' }).click()
    await sidebar(page).getByRole('button', { name: 'New query' }).click()
    expect((await tabTitles(page)).sort()).toEqual(['Query 1', 'Query 2', 'Query 3'])
  })

  test('⌘W closes the active tab; ⌘1…9 jump to a tab; Ctrl+Tab cycles', async ({ page }) => {
    for (const t of ['customers', 'orders', 'events']) await openTable(page, t)
    await page.keyboard.press(`${MOD}+1`)
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press(`${MOD}+2`)
    await expect(tabNamed(page, 'orders')).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press(`${MOD}+9`)
    await expect(tabNamed(page, 'events')).toHaveAttribute('aria-selected', 'true')

    await page.keyboard.press('Control+Tab')
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('Control+Shift+Tab')
    await expect(tabNamed(page, 'events')).toHaveAttribute('aria-selected', 'true')

    await page.keyboard.press(`${MOD}+w`)
    await expect(tabNamed(page, 'events')).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('tabs belong to their connection', async ({ page }) => {
    await openTable(page, 'customers')
    await showConnections(page)
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('tab', { name: /customers/ })).toHaveCount(0)
    await showConnections(page)
    await card(page, 'Local Postgres').dblclick()
    await expect(tabNamed(page, 'customers')).toBeVisible()
  })

  test('a table opened with a filter is told apart from the plain table tab', async ({ page }) => {
    // BUG: "View related" (Details panel foreign-key button) opens a second tab for the same table with a filter,
    // but both tabs are titled "customers" with the tooltip "public.customers", so nothing in the strip says which
    // is filtered (tabs.svelte.ts openTable keeps title = table.name for filtered tabs).
    await openTableLoaded(page, 'customers')
    await openTableLoaded(page, 'orders')
    await gridCell(page, 0, 1).click()
    await inspector(page).getByRole('button', { name: /^customer_id/ }).click()
    const titles = await tabTitles(page)
    expect(new Set(titles).size).toBe(titles.length)
  })

  test('query tabs and table tabs mix; a table opened from the sidebar never replaces a query tab', async ({ page }) => {
    await newQueryTab(page)
    await tableRow(page, 'customers').click() // preview
    await tableRow(page, 'orders').click() // replaces the preview, not the query tab
    expect(await tabTitles(page)).toEqual(['Query 1', 'orders'])
  })
})

test.describe('preview tabs', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('a single click opens an italic preview tab', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(tabNamed(page, 'customers')).toBeVisible()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'italic')
    await expect(page.getByRole('grid')).toBeVisible()
  })

  test('a single click on another table replaces the preview (the tab count stays 1)', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(tabNamed(page, 'customers')).toBeVisible()
    await tableRow(page, 'orders').click()
    await expect(tabNamed(page, 'orders')).toBeVisible()
    await expect(tabNamed(page, 'customers')).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['orders'])
    await expect(label(page, 'orders')).toHaveCSS('font-style', 'italic')
    await expect(page.getByRole('button', { name: /: public\.orders/ })).toBeVisible()
  })

  test('clicking the previewed table again keeps a single tab', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await tableRow(page, 'customers').click()
    expect(await tabTitles(page)).toEqual(['customers'])
  })

  test('double-clicking a sidebar item opens a normal (non-italic) tab that is not replaced', async ({ page }) => {
    await tableRow(page, 'customers').dblclick()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'orders').click()
    await expect(tabNamed(page, 'orders')).toBeVisible()
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await expect(label(page, 'orders')).toHaveCSS('font-style', 'italic')
  })

  test('double-clicking the preview tab itself keeps it', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'italic')
    await tabNamed(page, 'customers').dblclick()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'orders').click()
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('double-clicking the sidebar item of the current preview keeps that tab', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await tableRow(page, 'customers').dblclick()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    expect(await tabTitles(page)).toEqual(['customers'])
  })

  test('editing a cell keeps the preview', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await gridCell(page, 0, 2).dblclick()
    await page.keyboard.type('Alicia')
    await page.keyboard.press('Enter')
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'orders').click()
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('applying a filter keeps the preview', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await page.getByPlaceholder('Filter rows…').fill('Bob')
    await page.keyboard.press('Enter')
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'orders').click()
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('sorting keeps the preview', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
    await gridHeader(page, 'name').click()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'orders').click()
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('only one preview exists at a time, and query tabs are never previews', async ({ page }) => {
    await newQueryTab(page)
    await expect(label(page, 'Query 1')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'customers').click()
    await tableRow(page, 'orders').click()
    await tableRow(page, 'events').click()
    expect(await tabTitles(page)).toEqual(['Query 1', 'events'])
  })

  test('a preview that is closed with its x does not leave a ghost', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await closeButton(page, 'customers').click()
    expect(await tabTitles(page)).toEqual([])
    await tableRow(page, 'orders').click()
    expect(await tabTitles(page)).toEqual(['orders'])
  })
})

test.describe('restoring tabs', () => {
  const savedTabs = (page: Page) => page.evaluate(() => localStorage.getItem('tablesoft.tabs.c-pg'))

  async function disconnect(page: Page) {
    await page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ }).click()
    await page.getByRole('menuitem', { name: 'Disconnect' }).click()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
  }

  async function reopen(page: Page) {
    await card(page, 'Local Postgres').getByRole('button', { name: /^Open / }).click()
    await expect(page.getByRole('button', { name: 'Toggle sidebar' })).toBeVisible()
  }

  test('disconnecting and reconnecting brings the same tabs back, with the active one selected', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM events')
    await tabNamed(page, 'orders').click()

    await disconnect(page)
    await reopen(page)
    expect(await tabTitles(page)).toEqual(['customers', 'orders', 'Query 1'])
    await expect(tabNamed(page, 'orders')).toHaveAttribute('aria-selected', 'true')
    await tabNamed(page, 'Query 1').click()
    expect(await sqlText(page)).toBe('SELECT * FROM events')
    await tabNamed(page, 'customers').click()
    await expect(page.getByRole('grid')).toBeVisible()
  })

  test('the preview tab is still a preview after restoring', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await tableRow(page, 'orders').click()
    await expect(label(page, 'orders')).toHaveCSS('font-style', 'italic')
    await disconnect(page)
    await reopen(page)
    await expect(label(page, 'orders')).toHaveCSS('font-style', 'italic')
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')
    await tableRow(page, 'events').click()
    expect(await tabTitles(page)).toEqual(['customers', 'events'])
  })

  test('tabs and query text survive a page reload', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await newQueryTab(page)
    await setSql(page, 'SELECT 42 AS answer')
    // Saving is debounced (400ms); wait for it instead of sleeping.
    await expect.poll(() => savedTabs(page)).toContain('SELECT 42 AS answer')

    await page.reload()
    await openConnection(page)
    expect(await tabTitles(page)).toEqual(['customers', 'Query 1'])
    await expect(tabNamed(page, 'Query 1')).toHaveAttribute('aria-selected', 'true')
    expect(await sqlText(page)).toBe('SELECT 42 AS answer')
  })

  test('closed tabs stay closed after a reload', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await closeButton(page, 'customers').click()
    await expect.poll(() => savedTabs(page)).not.toContain('customers')
    await page.reload()
    await openConnection(page)
    expect(await tabTitles(page)).toEqual(['orders'])
  })

  test('each saved connection keeps its own tabs', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await disconnect(page)
    await card(page, 'Local MySQL').getByRole('button', { name: /^Open / }).click()
    await expect(page.getByRole('button', { name: /^MySQL 8\.4\.6/ })).toBeVisible()
    expect(await tabTitles(page)).toEqual([])
    await openTable(page, 'orders')
    await showConnections(page)
    await reopen(page)
    expect(await tabTitles(page)).toEqual(['customers'])
  })

  test('a connection with no saved tabs starts empty', async ({ page }) => {
    await openConnection(page)
    expect(await tabTitles(page)).toEqual([])
    await expect(page.getByText('Pick a table in the sidebar, or start a query.')).toBeVisible()
  })
})

test.describe('sidebar keyboard navigation', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('arrow keys walk the list and preview each table; Enter keeps the current one', async ({ page }) => {
    await tableRow(page, 'all_types').click()
    await tableRow(page, 'all_types').focus()
    await page.keyboard.press('ArrowDown')
    await expect(tableRow(page, 'big_table')).toBeFocused()
    await expect(tabNamed(page, 'big_table')).toBeVisible()
    await expect(label(page, 'big_table')).toHaveCSS('font-style', 'italic')
    expect(await tabTitles(page)).toEqual(['big_table'])
    await page.keyboard.press('ArrowDown')
    await expect(tableRow(page, 'categories')).toBeFocused()
    expect(await tabTitles(page)).toEqual(['categories'])
    await page.keyboard.press('ArrowUp')
    await expect(tableRow(page, 'big_table')).toBeFocused()

    await page.keyboard.press('Enter')
    await expect(label(page, 'big_table')).toHaveCSS('font-style', 'normal')
    await page.keyboard.press('ArrowDown')
    expect(await tabTitles(page)).toEqual(['big_table', 'categories'])
  })

  test('Right and Left expand and collapse a table’s columns', async ({ page }) => {
    await tableRow(page, 'customers').click()
    await tableRow(page, 'customers').focus()
    await page.keyboard.press('ArrowRight')
    await expect(tableRow(page, 'customers')).toHaveAttribute('aria-expanded', 'true')
    await expect(sidebar(page).locator('.col')).toHaveCount(3)
    await page.keyboard.press('ArrowLeft')
    await expect(tableRow(page, 'customers')).toHaveAttribute('aria-expanded', 'false')
    await expect(sidebar(page).locator('.col')).toHaveCount(0)
  })

  test('in the search box: ArrowDown moves into the list, Enter opens the first match, Escape clears', async ({ page }) => {
    const search = sidebar(page).getByPlaceholder('Search for item…')
    await search.fill('ord')
    await page.keyboard.press('ArrowDown')
    await expect(sidebar(page).getByRole('treeitem').first()).toBeFocused()
    await expect(tabNamed(page, 'orders')).toBeVisible()

    await search.fill('cust')
    await page.keyboard.press('Enter')
    await expect(tabNamed(page, 'customers')).toBeVisible()
    await expect(label(page, 'customers')).toHaveCSS('font-style', 'normal')

    await search.focus()
    await page.keyboard.press('Escape')
    await expect(search).toHaveValue('')
    await expect(sidebar(page).getByRole('treeitem')).toHaveCount(17)
  })

  test('the clear button empties the search and only shows while there is text', async ({ page }) => {
    const search = sidebar(page).getByPlaceholder('Search for item…')
    await expect(sidebar(page).getByRole('button', { name: 'Clear search' })).toHaveCount(0)
    await search.fill('cust')
    await sidebar(page).getByRole('button', { name: 'Clear search' }).click()
    await expect(search).toHaveValue('')
    await expect(sidebar(page).getByRole('treeitem')).toHaveCount(17)
    await expect(sidebar(page).getByRole('button', { name: 'Clear search' })).toHaveCount(0)
  })

  test('sidebar rows can be reached with the Tab key', async ({ page }) => {
    // The tree is a single tab stop (roving tabindex); section headers are reached with the arrow keys.
    await sidebar(page).getByPlaceholder('Search for item…').focus()
    await page.keyboard.press('Tab')
    await expect(sidebar(page).getByRole('treeitem').first()).toBeFocused()
  })
})
