import { test, expect } from './fixtures'
import {
  MOD,
  box,
  card,
  inspector,
  openConnection,
  openTable,
  openTableLoaded,
  recordInvokes,
  invokes,
  showConnections,
  sidebar,
  tableRow,
} from './helpers'

test.describe('toolbar status pill', () => {
  test('shows engine/version : connection : database before any table is open', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    const pill = page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres : tablesail/ })
    await expect(pill).toBeVisible()
    await expect(pill).toHaveText(/PostgreSQL 17\.6\s*:\s*Local Postgres\s*:\s*tablesail\s*local$/)
    await expect(pill).not.toContainText('public.')
  })

  test('adds schema.table for the active table tab and follows tab changes', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await expect(page.getByRole('button', { name: /: tablesail : public\.customers/ })).toBeVisible()
    await openTable(page, 'orders')
    await expect(page.getByRole('button', { name: /: tablesail : public\.orders/ })).toBeVisible()
    await page.getByRole('tab', { name: /customers/ }).click()
    await expect(page.getByRole('button', { name: /: tablesail : public\.customers/ })).toBeVisible()
  })

  test('shows the query tab title for a SQL editor', async ({ page }) => {
    await openConnection(page)
    await page.keyboard.press(`${MOD}+t`)
    await expect(page.getByRole('button', { name: /: tablesail : Query 1/ })).toBeVisible()
  })

  for (const [connection, version, tag] of [
    ['Local MySQL', 'MySQL 8.4.6', 'local'],
    ['MariaDB', 'MariaDB 11.8.3', 'dev'],
    ['Mongo', 'MongoDB 8.2.1', 'dev'],
    ['Staging API', 'PostgreSQL 17.6', 'staging'],
    ['Production', 'PostgreSQL 17.6', 'production'],
  ] as const) {
    test(`${connection}: pill carries the engine version and the ${tag} environment tag`, async ({ page }) => {
      await openConnection(page, connection)
      const pill = page.getByRole('button', { name: new RegExp(`^${version.replace(/\./g, '\\.')} : ${connection}`) })
      await expect(pill).toBeVisible()
      await expect(pill.locator('.tag')).toHaveText(tag)
    })
  }

  test('production and read-only connections are marked in the toolbar', async ({ page }) => {
    await openConnection(page, 'Production')
    await expect(page.getByTitle('Read-only connection')).toBeVisible()
    await expect(page.getByRole('button', { name: /: Production/ }).locator('.tag')).toHaveText('production')
  })

  test('clicking the pill lists open connections and offers to open another or disconnect', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ }).click()
    await expect(page.getByRole('menuitem', { name: /✓\s+Local Postgres/ })).toBeVisible()
    await page.getByRole('menuitem', { name: 'Open another connection…' }).click()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await expect(card(page, 'Local Postgres')).toContainText('Connected')

    await card(page, 'Local MySQL').dblclick()
    await page.getByRole('button', { name: /^MySQL 8\.4\.6 : Local MySQL/ }).click()
    await expect(page.getByRole('menuitem', { name: /Local Postgres/ })).toBeVisible()
    await page.getByRole('menuitem', { name: /Local Postgres/ }).click()
    await expect(page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ })).toBeVisible()
  })

  test('Disconnect closes the session and returns to the connection list', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ }).click()
    await page.getByRole('menuitem', { name: 'Disconnect' }).click()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await expect(card(page, 'Local Postgres')).not.toContainText('Connected')
  })

  test('tabs survive a trip to the connection list and back', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await showConnections(page)
    await card(page, 'Local Postgres').dblclick()
    await expect(page.getByRole('tab', { name: /customers/ })).toBeVisible()
    await expect(page.getByRole('grid')).toBeVisible()
  })
})

test.describe('empty workspace', () => {
  test('offers New query and lists the main shortcuts', async ({ page }) => {
    await openConnection(page, 'Local Postgres')
    await expect(page.getByRole('heading', { name: 'Local Postgres', level: 2 })).toBeVisible()
    await expect(page.getByText('Pick a table in the sidebar, or start a query.')).toBeVisible()
    await page.getByRole('button', { name: 'New query' }).last().click()
    await expect(page.getByRole('tab', { name: /Query 1/ })).toBeVisible()
  })
})

test.describe('sidebar', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('has Items, Queries and History tabs; Items is the default and the search placeholder follows the tab', async ({ page }) => {
    const side = sidebar(page)
    const tabs = side.getByRole('tab')
    await expect(tabs).toHaveText(['Items', 'Queries', 'History'])
    await expect(side.getByRole('tab', { name: 'Items' })).toHaveAttribute('aria-selected', 'true')
    await expect(side.getByPlaceholder('Search for item…')).toBeVisible()

    await side.getByRole('tab', { name: 'Queries' }).click()
    await expect(side.getByRole('tab', { name: 'Queries' })).toHaveAttribute('aria-selected', 'true')
    await expect(side.getByPlaceholder('Search queries…')).toBeVisible()
    await expect(side.getByText('Save a query from the SQL editor to keep it here.')).toBeVisible()
    await expect(side.getByRole('treeitem')).toHaveCount(0)

    await side.getByRole('tab', { name: 'History' }).click()
    await expect(side.getByPlaceholder('Search history…')).toBeVisible()
    await expect(side.getByText('Queries you run appear here.')).toBeVisible()

    await side.getByRole('tab', { name: 'Items' }).click()
    await expect(side.getByRole('treeitem').first()).toBeVisible()
  })

  test('lists tables, views and materialized views in collapsible sections', async ({ page }) => {
    const side = sidebar(page)
    await expect(side.getByRole('button', { name: 'Tables' })).toHaveAttribute('aria-expanded', 'true')
    await expect(side.getByRole('button', { name: 'Views', exact: true })).toBeVisible()
    await expect(side.getByRole('button', { name: 'Materialized Views' })).toBeVisible()
    await expect(side.getByRole('treeitem')).toHaveCount(17)
    await expect(tableRow(page, 'customer_totals')).toBeVisible()
    await expect(tableRow(page, 'mood_counts')).toBeVisible()

    await side.getByRole('button', { name: 'Tables' }).click()
    await expect(side.getByRole('button', { name: 'Tables' })).toHaveAttribute('aria-expanded', 'false')
    await expect(side.getByRole('treeitem')).toHaveCount(2)
    await side.getByRole('button', { name: 'Tables' }).click()
    await expect(side.getByRole('treeitem')).toHaveCount(17)
  })

  test('search filters the tree as you type, and says when nothing matches', async ({ page }) => {
    const side = sidebar(page)
    const search = side.getByPlaceholder('Search for item…')
    await search.fill('cust')
    await expect(side.getByRole('treeitem')).toHaveText([/customers$/, /customer_totals$/])

    await search.fill('ORDER') // case-insensitive
    await expect(side.getByRole('treeitem')).toHaveText([/orders$/])

    await search.fill('zzzz')
    await expect(side.getByText('No matching items.')).toBeVisible()
    await expect(side.getByRole('treeitem')).toHaveCount(0)

    await search.fill('')
    await expect(side.getByRole('treeitem')).toHaveCount(17)
  })

  test('a search looks across schemas and qualifies names from other schemas', async ({ page }) => {
    const side = sidebar(page)
    await side.getByPlaceholder('Search for item…').fill('invoices')
    const row = side.getByRole('treeitem', { name: /invoices/ })
    await expect(row).toBeVisible()
    await expect(row).toContainText('billing.')
  })

  test('switching tabs clears the search box', async ({ page }) => {
    const side = sidebar(page)
    await side.getByPlaceholder('Search for item…').fill('cust')
    await side.getByRole('tab', { name: 'History' }).click()
    await expect(side.getByPlaceholder('Search history…')).toHaveValue('')
    await side.getByRole('tab', { name: 'Items' }).click()
    await expect(side.getByRole('treeitem')).toHaveCount(17)
  })

  test('expanding a table shows its columns with types and the primary key', async ({ page }) => {
    const row = tableRow(page, 'customers')
    await expect(row).toHaveAttribute('aria-expanded', 'false')
    await row.locator('.twist').click()
    await expect(row).toHaveAttribute('aria-expanded', 'true')
    const cols = sidebar(page).locator('.col')
    await expect(cols).toHaveCount(3)
    await expect(cols.nth(0)).toContainText('id')
    await expect(cols.nth(0)).toContainText('integer')
    await expect(cols.nth(1)).toContainText('email')
    await expect(cols.nth(1)).toContainText('citext')
    await expect(cols.nth(2)).toContainText('name')
    await expect(cols.nth(0).locator('.cicon')).toHaveClass(/pk/)
    // Expanding does not open the table.
    await expect(page.getByRole('tab', { name: /customers/ })).toHaveCount(0)

    await row.locator('.twist').click()
    await expect(row).toHaveAttribute('aria-expanded', 'false')
    await expect(cols).toHaveCount(0)
  })

  test('the schema picker switches the listed objects', async ({ page }) => {
    const side = sidebar(page)
    const schema = side.getByRole('combobox', { name: 'Schema' })
    await expect(schema).toHaveValue('public')
    await expect(schema.locator('option')).toHaveText(['public', 'billing', 'empty_schema'])

    await schema.selectOption('billing')
    await expect(side.getByRole('treeitem')).toHaveText(['invoices'])
    await expect(tableRow(page, 'customers')).toHaveCount(0)

    await schema.selectOption('empty_schema')
    await expect(side.getByText('Nothing here yet.')).toBeVisible()

    await schema.selectOption('public')
    await expect(side.getByRole('treeitem')).toHaveCount(17)
  })

  test('the database picker switches database and updates the status pill', async ({ page }) => {
    const side = sidebar(page)
    const db = side.getByRole('combobox', { name: 'Database' })
    await expect(db.locator('option')).toHaveText(['tablesail', 'tablesail_other', 'postgres'])
    await db.selectOption('tablesail_other')
    await expect(page.getByRole('button', { name: /: tablesail_other$|: tablesail_other / })).toBeVisible()
    await expect(side.getByRole('treeitem').first()).toBeVisible()
  })

  test('the toolbar database button offers the same databases', async ({ page }) => {
    await page.getByRole('button', { name: 'Switch database' }).click()
    await expect(page.getByRole('menuitem')).toHaveText([/^✓\s+tablesail\s*$/, /^\s*tablesail_other\s*$/, /^\s*postgres\s*$/])
    await page.getByRole('menuitem', { name: /tablesail_other/ }).click()
    await expect(page.getByRole('button', { name: /: tablesail_other/ })).toBeVisible()
    await expect(sidebar(page).getByRole('combobox', { name: 'Database' })).toHaveValue('tablesail_other')
  })

  test('table context menu: Open, New query, Export, Import, Rename, Truncate, Drop, Copy name, Refresh', async ({ page }) => {
    await tableRow(page, 'customers').click({ button: 'right' })
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toEqual(['Open', 'New query', 'Export…', 'Import CSV…', 'Rename…', 'Truncate…', 'Drop…', 'Copy name', 'Refresh'])
    await page.getByRole('menuitem', { name: 'New query' }).click()
    await expect(page.getByRole('tab', { name: /Query 1/ })).toBeVisible()
    await expect(page.locator('.tab-body:not([hidden]) .cm-content')).toContainText('customers')
  })

  test('the sidebar plus button opens a new query', async ({ page }) => {
    await sidebar(page).getByRole('button', { name: 'New query' }).click()
    await expect(page.getByRole('tab', { name: /Query 1/ })).toBeVisible()
  })

  test('Refresh asks the backend for a fresh schema snapshot', async ({ page }) => {
    await recordInvokes(page)
    await page.getByRole('button', { name: 'Refresh schema' }).click()
    await expect.poll(async () => (await invokes(page, 'schema_snapshot')).some((c) => c.args.refresh === true)).toBe(true)
  })
})

test.describe('panels and toggles', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('the sidebar toggle hides and restores the sidebar', async ({ page }) => {
    const side = sidebar(page)
    expect((await box(side)).width).toBeGreaterThan(150)
    await page.getByRole('button', { name: 'Toggle sidebar' }).click()
    await expect.poll(async () => (await side.boundingBox())?.width ?? 0).toBeLessThan(2)
    await page.getByRole('button', { name: 'Toggle sidebar' }).click()
    await expect.poll(async () => (await side.boundingBox())?.width ?? 0).toBeGreaterThan(150)
  })

  test('the sidebar can be resized by dragging its divider, and the width is remembered', async ({ page }) => {
    const side = sidebar(page)
    const before = (await box(side)).width
    const divider = page.locator('.resize').first()
    const d = await box(divider)
    await page.mouse.move(d.x + d.width / 2, d.y + 200)
    await page.mouse.down()
    await page.mouse.move(d.x + 120, d.y + 200, { steps: 6 })
    await page.mouse.up()
    await expect.poll(async () => (await box(side)).width).toBeGreaterThan(before + 60)
    const stored = await page.evaluate(() => Number(localStorage.getItem('tablesail.sidebarWidth')))
    expect(stored).toBeGreaterThan(before + 60)
  })

  test('the details toggle opens and closes the right panel', async ({ page }) => {
    await expect(inspector(page)).toHaveCount(0)
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
    await expect(inspector(page).getByText('No row selected')).toBeVisible()
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(inspector(page)).toHaveCount(0)
  })

  test('the assistant button opens the Assistant tab; the details button then switches tab instead of closing', async ({ page }) => {
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expect(inspector(page).getByRole('tab', { name: 'Assistant' })).toHaveAttribute('aria-selected', 'true')
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expect(inspector(page)).toHaveCount(0)
  })

  test('the panel tabs switch between Details and Assistant', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await inspector(page).getByRole('tab', { name: 'Assistant' }).click()
    await expect(inspector(page).getByPlaceholder('Ask about your data…')).toBeVisible()
    await inspector(page).getByRole('tab', { name: 'Details' }).click()
    await expect(inspector(page).getByPlaceholder('Search for field…')).toBeVisible()
  })

  test('the details toggle still does something in a window narrower than 1120px', async ({ page }) => {
    // BUG: below 1120px Workspace.svelte hides the inspector with CSS (`.body > .inspector { display: none }`) but
    // leaves inspector.open true, so "Toggle details panel", the palette's "Toggle row inspector" and the
    // auto-open on row selection all silently do nothing: the Details panel (and its field editing) is
    // unreachable in a window between the 800px minimum and 1120px, with no hint why.
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(inspector(page)).toBeVisible({ timeout: 1500 })
  })

  test('below 1120px wide the panel stays open but slimmer, so the grid keeps room', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(inspector(page)).toBeVisible()
    const wide = (await inspector(page).boundingBox())!.width
    await page.setViewportSize({ width: 1000, height: 700 })
    await expect(inspector(page)).toBeVisible()
    await expect.poll(async () => (await inspector(page).boundingBox())!.width).toBeLessThan(wide)
    await page.setViewportSize({ width: 1440, height: 900 })
    await expect.poll(async () => (await inspector(page).boundingBox())!.width).toBe(wide)
  })
})

test.describe('query log pane', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  const log = (page: import('@playwright/test').Page) => page.locator('.console')

  test('toggle shows timestamped SQL lines, closes again, and the toggle button does both', async ({ page }) => {
    await expect(log(page)).toHaveCount(0)
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page)).toBeVisible()
    // The connection itself already ran statements.
    await expect(log(page).locator('.stamp').first()).toHaveText(/^-- \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{4}$/)
    await expect(log(page).locator('.sql', { hasText: 'information_schema.schemata' }).first()).toBeVisible()
    await expect(log(page).locator('.sql', { hasText: 'FROM pg_class' }).first()).toBeVisible()

    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page)).toHaveCount(0)
  })

  test('new statements are appended as they happen (opening a table)', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page).locator('.sql', { hasText: 'information_schema.schemata' }).first()).toBeVisible()
    await openTable(page, 'customers')
    await expect(log(page).locator('.sql', { hasText: 'SELECT * FROM "public"."customers"' })).toBeVisible()
    await expect(log(page).locator('.sql', { hasText: 'SELECT count(*) FROM "public"."customers"' })).toBeVisible()
  })

  test('Clear log empties it and shows the placeholder; the log keeps working after', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page).locator('.sql').first()).toBeVisible()
    await log(page).getByRole('button', { name: 'Clear log' }).click()
    await expect(log(page).locator('.sql')).toHaveCount(0)
    await expect(log(page).getByText('-- Statements sent to the server will appear here.')).toBeVisible()
    await expect(log(page).getByRole('button', { name: 'Clear log' })).toBeDisabled()

    await openTable(page, 'orders')
    await expect(log(page).locator('.sql', { hasText: '"public"."orders"' }).first()).toBeVisible()
  })

  test('the Close console button hides the pane', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await log(page).getByRole('button', { name: 'Close console' }).click()
    await expect(log(page)).toHaveCount(0)
  })

  test('syntax highlighting can be switched off and the choice is remembered', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page).locator('.sql .k').first()).toBeVisible()
    await log(page).getByLabel('Enable syntax highlighting').uncheck()
    await expect(log(page).locator('.sql .k')).toHaveCount(0)
    await page.reload()
    await openConnection(page)
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    await expect(log(page).locator('.sql').first()).toBeVisible()
    await expect(log(page).getByLabel('Enable syntax highlighting')).not.toBeChecked()
  })

  test('the pane can be resized with the handle above it', async ({ page }) => {
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    const handle = page.locator('.log-resize')
    await expect(handle).toBeVisible()
    const before = (await box(log(page))).height
    const h = await box(handle)
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2)
    await page.mouse.down()
    await page.mouse.move(h.x + h.width / 2, h.y - 100, { steps: 6 })
    await page.mouse.up()
    await expect.poll(async () => (await box(log(page))).height).toBeGreaterThan(before + 60)
  })

  test('the pane does not cover the table bottom bar', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await page.getByRole('button', { name: 'Toggle query log' }).click()
    const bar = await box(page.locator('.tab-body:not([hidden]) .bottombar'))
    const pane = await box(log(page))
    expect(bar.y + bar.height).toBeLessThanOrEqual(pane.y + 1)
  })
})

test.describe('theme', () => {
  test('⌘⇧L flips data-theme between light and dark and remembers it across reloads', async ({ page }) => {
    await openConnection(page)
    const html = page.locator('html')
    await expect(html).toHaveAttribute('data-theme', 'light')
    await page.keyboard.press(`${MOD}+Shift+L`)
    await expect(html).toHaveAttribute('data-theme', 'dark')
    await page.keyboard.press(`${MOD}+Shift+L`)
    await expect(html).toHaveAttribute('data-theme', 'light')

    await page.keyboard.press(`${MOD}+Shift+L`)
    await expect(html).toHaveAttribute('data-theme', 'dark')
    await page.reload()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await expect(html).toHaveAttribute('data-theme', 'dark')
  })

  test('the dark theme actually changes the colours', async ({ page }) => {
    await openConnection(page)
    const bg = () => page.evaluate(() => getComputedStyle(document.querySelector('.toolbar')!).backgroundColor)
    const light = await bg()
    await page.keyboard.press(`${MOD}+Shift+L`)
    await expect.poll(bg).not.toBe(light)
  })

  test('works on the connection list too', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await page.keyboard.press(`${MOD}+Shift+L`)
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  })

  test('follows the system preference until a choice is made', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'dark' })
    await page.goto('/')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  })

  test('the toolbar menu toggles the appearance and can return to the system setting', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await expect(page.getByRole('menuitem', { name: 'Use system appearance' })).toBeDisabled()
    await page.getByRole('menuitem', { name: /Dark appearance/ }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')

    await page.getByRole('button', { name: 'Menu' }).click()
    await expect(page.getByRole('menuitem', { name: /Light appearance/ })).toBeVisible()
    await page.getByRole('menuitem', { name: 'Use system appearance' }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  })
})

test.describe('toolbar menu', () => {
  test('lists the app-level actions', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toEqual([
      expect.stringMatching(/^Open SQL file…/),
      expect.stringMatching(/^Save to file…/),
      'Import SQL dump…',
      expect.stringMatching(/^(Dark|Light) appearance/),
      'Use system appearance',
      'Query history',
      'Saved queries',
      expect.stringMatching(/^Keyboard shortcuts/),
      expect.stringMatching(/^Settings…/),
      'About / Check for updates…',
    ])
  })

  test('Query history and Saved queries switch the sidebar to those tabs', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: 'Query history' }).click()
    await expect(sidebar(page).getByRole('tab', { name: 'History' })).toHaveAttribute('aria-selected', 'true')
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: 'Saved queries' }).click()
    await expect(sidebar(page).getByRole('tab', { name: 'Queries' })).toHaveAttribute('aria-selected', 'true')
  })

  test('About opens the about dialog', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: 'About / Check for updates…' }).click()
    await expect(page.getByRole('dialog').first()).toBeVisible()
    await expect(page.getByRole('dialog').first()).toContainText('0.1.0')
  })

  test('the Settings… item advertises ⌘, and the shortcut actually opens Settings', async ({ page }) => {
    // BUG: the menu hint says "⌘," but shortcuts.ts has no handler for it, and ShortcutsHelp does not list it.
    await openConnection(page)
    await page.keyboard.press(`${MOD}+,`)
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible()
  })

  test('the Refresh button advertises ⌘R and the shortcut refreshes the schema when no table is open', async ({ page }) => {
    // BUG: the toolbar's Refresh button title says "Refresh (⌘R)" but with no table tab open ⌘R does
    // nothing app-wide (only a table tab handles it, for its own data) — the schema is not refreshed.
    await openConnection(page)
    await recordInvokes(page)
    await page.keyboard.press(`${MOD}+r`)
    await expect.poll(async () => (await invokes(page, 'schema_snapshot')).some((c) => c.args.refresh === true)).toBe(true)
  })
})
