import { test, expect } from './fixtures'
import {
  bottomBar,
  gridCell,
  invokes,
  newQueryTab,
  openConnection,
  openTable,
  openTableLoaded,
  recordInvokes,
  runStatement,
  setSql,
  sidebar,
  tabNamed,
  tabTitles,
  tableRow,
} from './helpers'
import type { Page } from '@playwright/test'

/*
 * Drop, truncate and rename from the sidebar: the item's context menu opens a confirmation dialog that shows the
 * exact statement; on success the tree refreshes, the tabs of the object follow, and a toast says what happened.
 * The mock backend's table_admin mutates its table list, so a dropped or renamed table really leaves the tree.
 */

const dialog = (page: Page, name: string | RegExp) => page.getByRole('dialog', { name })
const toast = (page: Page) => page.locator('.toasts .toast')
const menuItems = async (page: Page) => (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())

async function openAdmin(page: Page, table: string, item: string | RegExp) {
  await tableRow(page, table).click({ button: 'right' })
  await page.getByRole('menuitem', { name: item }).click()
}

/** Makes the next executed (non-preview) table_admin call fail the way a server error would. */
async function failAdmin(page: Page, message: string) {
  await page.evaluate((m) => {
    const t = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: { preview?: boolean }) => unknown } }).__TAURI_INTERNALS__
    const orig = t.invoke
    t.invoke = (cmd, args) =>
      cmd === 'table_admin' && !args?.preview ? Promise.reject({ kind: 'database', message: m, code: null, statement: null }) : orig(cmd, args)
  }, message)
}

/** The table_admin calls that ran (preview=false). */
async function executed(page: Page) {
  return (await invokes(page, 'table_admin')).filter((c) => !c.args.preview)
}

test.describe('the context menu', () => {
  test('a table offers Rename, Truncate and Drop between Import and Copy name', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    expect(await menuItems(page)).toEqual(['Open', 'New query', 'Export…', 'Import CSV…', 'Rename…', 'Truncate…', 'Drop…', 'Copy name', 'Refresh'])
    for (const name of ['Rename…', 'Truncate…', 'Drop…']) await expect(page.getByRole('menuitem', { name })).toBeEnabled()
  })

  test('a view offers Rename and Drop only (no Truncate)', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customer_totals').click({ button: 'right' })
    expect(await menuItems(page)).toEqual(['Open', 'New query', 'Export…', 'Import CSV…', 'Rename…', 'Drop…', 'Copy name', 'Refresh'])
  })

  test('a materialized view offers Rename and Drop only', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'mood_counts').click({ button: 'right' })
    expect(await menuItems(page)).toContain('Rename…')
    expect(await menuItems(page)).toContain('Drop…')
    expect(await menuItems(page)).not.toContain('Truncate…')
  })

  test('a SQLite view can only be dropped (SQLite has no ALTER VIEW)', async ({ page }) => {
    await openConnection(page, 'nasty.db')
    await tableRow(page, 'customer_totals').click({ button: 'right' })
    const items = await menuItems(page)
    expect(items).toContain('Drop…')
    expect(items).not.toContain('Rename…')
    expect(items).not.toContain('Truncate…')
  })

  test('MongoDB calls it a collection: Rename, Empty collection, Drop', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await tableRow(page, 'customers').click({ button: 'right' })
    const items = await menuItems(page)
    expect(items).toContain('Rename…')
    expect(items).toContain('Empty collection…')
    expect(items).toContain('Drop…')
    expect(items).not.toContain('Truncate…')
  })

  test('a MongoDB view can only be dropped (MongoDB cannot rename one)', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await tableRow(page, 'customer_totals').click({ button: 'right' })
    const items = await menuItems(page)
    expect(items).toContain('Drop…')
    expect(items).not.toContain('Rename…')
    await page.getByRole('menuitem', { name: 'Drop…' }).click()
    await expect(dialog(page, 'Drop view')).toBeVisible()
  })

  test('on a read-only connection all three are disabled', async ({ page }) => {
    await openConnection(page, 'Production')
    await tableRow(page, 'customers').click({ button: 'right' })
    for (const name of ['Rename…', 'Truncate…', 'Drop…']) await expect(page.getByRole('menuitem', { name })).toBeDisabled()
  })

  test('Truncate and Drop are marked as dangerous, Rename is not', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: 'Drop…' })).toHaveClass(/danger/)
    await expect(page.getByRole('menuitem', { name: 'Truncate…' })).toHaveClass(/danger/)
    await expect(page.getByRole('menuitem', { name: 'Rename…' })).not.toHaveClass(/danger/)
  })

  test('keyboard: arrows reach the new items and Enter opens the dialog', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'empty_table').click({ button: 'right' })
    await page.keyboard.press('End') // Refresh
    await page.keyboard.press('ArrowUp') // Copy name
    await page.keyboard.press('ArrowUp') // Drop…
    await expect(page.getByRole('menuitem', { name: 'Drop…' })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Drop table')).toBeVisible()
  })
})

test.describe('drop', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await recordInvokes(page)
  })

  test('shows the statement and warns, and Drop stays disabled until the name is typed', async ({ page }) => {
    await openAdmin(page, 'empty_table', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await expect(dlg).toHaveAttribute('aria-modal', 'true')
    await expect(dlg).toContainText('permanently deletes the table public.empty_table and all of its rows.')
    // "Type <name> to confirm" is one line (its parts once became separate flex items, one per line).
    expect((await dlg.locator('label > span').boundingBox())!.height).toBeLessThan(24)
    await expect(dlg).toContainText("This can't be undone.")
    await expect(dlg.locator('.sql')).toHaveText('DROP TABLE "public"."empty_table";')
    // The statement is highlighted like in the console.
    await expect(dlg.locator('.sql .k').first()).toHaveText('DROP')

    const drop = dlg.getByRole('button', { name: 'Drop', exact: true })
    await expect(drop).toBeDisabled()
    await expect(drop).toHaveClass(/danger/)
    const confirm = dlg.getByLabel('Type empty_table to confirm')
    await expect(confirm).toBeFocused()

    await confirm.fill('empty')
    await expect(drop).toBeDisabled()
    await confirm.fill('EMPTY_TABLE') // case matters
    await expect(drop).toBeDisabled()
    await confirm.fill('empty_table')
    await expect(drop).toBeEnabled()
    await confirm.fill('empty_tabl')
    await expect(drop).toBeDisabled()
    expect(await executed(page)).toHaveLength(0)
  })

  test('typing the name and pressing Drop removes the table, closes its tab and says so', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await openTable(page, 'empty_table')
    await expect(tabNamed(page, 'empty_table')).toBeVisible()

    await openAdmin(page, 'empty_table', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await dlg.getByLabel('Type empty_table to confirm').fill('empty_table')
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()

    await expect(dlg).toHaveCount(0)
    await expect(tableRow(page, 'empty_table')).toHaveCount(0)
    await expect(tabNamed(page, 'empty_table')).toHaveCount(0)
    await expect(tabNamed(page, 'customers')).toBeVisible()
    await expect(toast(page).filter({ hasText: 'Dropped table “empty_table”' })).toBeVisible()
    await expect(sidebar(page).getByRole('treeitem')).toHaveCount(16)

    const ran = await executed(page)
    expect(ran).toHaveLength(1)
    expect(ran[0].args).toMatchObject({ table: { name: 'empty_table', schema: 'public' }, action: { kind: 'drop' }, preview: false })
  })

  test('Enter in the name field drops once it matches, and does nothing before', async ({ page }) => {
    await openAdmin(page, 'unique_only', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    const confirm = dlg.getByLabel('Type unique_only to confirm')
    await confirm.fill('uni')
    await confirm.press('Enter')
    await expect(dlg).toBeVisible()
    expect(await executed(page)).toHaveLength(0)
    await confirm.fill('unique_only')
    await confirm.press('Enter')
    await expect(dlg).toHaveCount(0)
    await expect(tableRow(page, 'unique_only')).toHaveCount(0)
  })

  test('closes every tab of the table, including a filtered one, and only those', async ({ page }) => {
    await openTable(page, 'orders')
    await openTable(page, 'customers')
    await openAdmin(page, 'orders', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await dlg.getByLabel('Type orders to confirm').fill('orders')
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()
    await expect(dlg).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['customers'])
    await expect(tabNamed(page, 'customers')).toHaveAttribute('aria-selected', 'true')
  })

  test('Cancel and Escape close it without running anything', async ({ page }) => {
    await openAdmin(page, 'customers', 'Drop…')
    await dialog(page, 'Drop table').getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog(page, 'Drop table')).toHaveCount(0)

    await openAdmin(page, 'customers', 'Drop…')
    await expect(dialog(page, 'Drop table')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog(page, 'Drop table')).toHaveCount(0)

    await expect(tableRow(page, 'customers')).toBeVisible()
    expect(await executed(page)).toHaveLength(0)
  })

  test('a server error is shown in the dialog, which stays open and can be retried', async ({ page }) => {
    await failAdmin(page, 'cannot drop table orders because other objects depend on it')
    await openAdmin(page, 'orders', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await dlg.getByLabel('Type orders to confirm').fill('orders')
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()
    await expect(dlg.getByRole('alert')).toContainText('cannot drop table orders because other objects depend on it')
    await expect(dlg).toBeVisible()
    await expect(tableRow(page, 'orders')).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Drop', exact: true })).toBeEnabled()
    await expect(toast(page)).toHaveCount(0)
  })

  test('a view is dropped with DROP VIEW and its warning does not talk about rows', async ({ page }) => {
    await openAdmin(page, 'customer_totals', 'Drop…')
    const dlg = dialog(page, 'Drop view')
    await expect(dlg.locator('.sql')).toHaveText('DROP VIEW "public"."customer_totals";')
    await expect(dlg).not.toContainText('rows')
    await dlg.getByLabel('Type customer_totals to confirm').fill('customer_totals')
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()
    await expect(tableRow(page, 'customer_totals')).toHaveCount(0)
    await expect(toast(page).filter({ hasText: 'Dropped view “customer_totals”' })).toBeVisible()
  })

  test('a table with a mixed-case name needs it typed exactly', async ({ page }) => {
    await openAdmin(page, 'MixedCase', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await expect(dlg.locator('.sql')).toHaveText('DROP TABLE "public"."MixedCase";')
    const drop = dlg.getByRole('button', { name: 'Drop', exact: true })
    await dlg.getByLabel('Type MixedCase to confirm').fill('mixedcase')
    await expect(drop).toBeDisabled()
    await dlg.getByLabel('Type MixedCase to confirm').fill('MixedCase')
    await expect(drop).toBeEnabled()
  })

  test('MongoDB: "Drop collection"', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await openAdmin(page, 'customers', 'Drop…')
    const dlg = dialog(page, 'Drop collection')
    await expect(dlg).toContainText('permanently deletes the collection')
    await expect(dlg).toContainText('all of its documents')
  })
})

test.describe('delete key in the tree', () => {
  test('Delete on a focused table opens the Drop dialog, and Escape puts the focus back', async ({ page }) => {
    await openConnection(page)
    const row = tableRow(page, 'customers')
    await row.focus()
    await page.keyboard.press('Delete')
    const dlg = dialog(page, 'Drop table')
    await expect(dlg).toBeVisible()
    await expect(dlg).toContainText('customers')
    await expect(dlg.getByRole('button', { name: 'Drop', exact: true })).toBeDisabled()
    await page.keyboard.press('Escape')
    await expect(dlg).toHaveCount(0)
    await expect(row).toBeFocused()
  })

  test('Backspace does the same (the Delete key on a Mac)', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'orders').focus()
    await page.keyboard.press('Backspace')
    await expect(dialog(page, 'Drop table')).toBeVisible()
  })

  test('the whole flow works from the keyboard', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'user').focus()
    await page.keyboard.press('Delete')
    await expect(dialog(page, 'Drop table').getByRole('textbox')).toBeFocused()
    await page.keyboard.type('user')
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Drop table')).toHaveCount(0)
    await expect(tableRow(page, 'user')).toHaveCount(0)
  })

  test('a modifier makes it something else, and Delete in the search box still edits text', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').focus()
    await page.keyboard.press('Shift+Delete')
    await page.keyboard.press('Alt+Backspace')
    await expect(page.getByRole('dialog')).toHaveCount(0)

    const search = sidebar(page).getByPlaceholder('Search for item…')
    await search.fill('cust')
    await search.press('Backspace')
    await expect(search).toHaveValue('cus')
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('on a read-only connection it only explains', async ({ page }) => {
    await openConnection(page, 'Production')
    await tableRow(page, 'customers').focus()
    await page.keyboard.press('Delete')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(toast(page).filter({ hasText: 'This connection is read-only.' })).toBeVisible()
  })

  test('deleting a selected grid row is still the grid\'s own Delete', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await page.getByRole('grid').locator('.cell:not(.ph)').first().click()
    await page.keyboard.press('Delete')
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })
})

test.describe('truncate', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await recordInvokes(page)
  })

  test('shows the statement; the red button needs no typing and Cancel has the focus', async ({ page }) => {
    await openAdmin(page, 'orders', 'Truncate…')
    const dlg = dialog(page, 'Truncate table')
    await expect(dlg).toContainText('permanently removes every row from')
    await expect(dlg.locator('.sql')).toHaveText('TRUNCATE TABLE "public"."orders";')
    await expect(dlg.getByRole('textbox')).toHaveCount(0)
    const go = dlg.getByRole('button', { name: 'Truncate', exact: true })
    await expect(go).toBeEnabled()
    await expect(go).toHaveClass(/danger/)
    await expect(dlg.getByRole('button', { name: 'Cancel' })).toBeFocused()
  })

  test('running it keeps the table, reloads its open tab and says so', async ({ page }) => {
    await openTableLoaded(page, 'orders')
    await openTable(page, 'customers')
    const before = (await invokes(page, 'browse_page')).filter((c) => (c.args.request as { table: { name: string } }).table.name === 'orders').length

    await openAdmin(page, 'orders', 'Truncate…')
    await dialog(page, 'Truncate table').getByRole('button', { name: 'Truncate', exact: true }).click()
    await expect(dialog(page, 'Truncate table')).toHaveCount(0)
    await expect(toast(page).filter({ hasText: 'Truncated table “orders”' })).toBeVisible()
    await expect(tableRow(page, 'orders')).toBeVisible()

    // The open tab is still there, in the same place, and was loaded again.
    expect(await tabTitles(page)).toEqual(['orders', 'customers'])
    await tabNamed(page, 'orders').click()
    await expect(page.getByRole('grid')).toBeVisible()
    await expect
      .poll(async () => (await invokes(page, 'browse_page')).filter((c) => (c.args.request as { table: { name: string } }).table.name === 'orders').length)
      .toBeGreaterThan(before)

    const ran = await executed(page)
    expect(ran).toHaveLength(1)
    expect(ran[0].args).toMatchObject({ table: { name: 'orders' }, action: { kind: 'truncate' } })
  })

  test('Enter does not truncate: it presses the focused Cancel', async ({ page }) => {
    await openAdmin(page, 'orders', 'Truncate…')
    await expect(dialog(page, 'Truncate table').getByRole('button', { name: 'Cancel' })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Truncate table')).toHaveCount(0)
    expect(await executed(page)).toHaveLength(0)
  })

  test('a server error is shown in the dialog', async ({ page }) => {
    await failAdmin(page, 'cannot truncate a table referenced in a foreign key constraint')
    await openAdmin(page, 'customers', 'Truncate…')
    const dlg = dialog(page, 'Truncate table')
    await dlg.getByRole('button', { name: 'Truncate', exact: true }).click()
    await expect(dlg.getByRole('alert')).toContainText('cannot truncate a table referenced in a foreign key constraint')
    await expect(dlg).toBeVisible()
  })

  test('MongoDB: "Empty collection"', async ({ page }) => {
    await openConnection(page, 'Mongo')
    await openAdmin(page, 'customers', 'Empty collection…')
    const dlg = dialog(page, 'Empty collection')
    await expect(dlg).toContainText('permanently removes every document from')
    await dlg.getByRole('button', { name: 'Empty', exact: true }).click()
    await expect(toast(page).filter({ hasText: 'Emptied collection “customers”' })).toBeVisible()
  })
})

test.describe('rename', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await recordInvokes(page)
  })

  test('starts with the current name selected, Rename disabled, and no statement yet', async ({ page }) => {
    await openAdmin(page, 'customers', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    const input = dlg.getByLabel('New name')
    await expect(input).toHaveValue('customers')
    await expect(input).toBeFocused()
    expect(await input.evaluate((el: HTMLInputElement) => el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0))).toBe('customers')
    const go = dlg.getByRole('button', { name: 'Rename', exact: true })
    await expect(go).toBeDisabled()
    await expect(go).toHaveClass(/primary/)
    await expect(go).not.toHaveClass(/danger/)
    await expect(dlg.locator('.sql')).toContainText('Enter a new name')
    await expect(dlg.getByRole('alert')).toHaveCount(0)
  })

  test('validates: empty and unchanged names cannot be submitted', async ({ page }) => {
    await openAdmin(page, 'customers', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    const input = dlg.getByLabel('New name')
    const go = dlg.getByRole('button', { name: 'Rename', exact: true })

    await input.fill('')
    await expect(dlg).toContainText('Enter a new name.')
    await expect(input).toHaveAttribute('aria-invalid', 'true')
    await expect(go).toBeDisabled()

    await input.fill('   ')
    await expect(dlg).toContainText('Enter a new name.')
    await expect(go).toBeDisabled()

    await input.fill('customers ')
    await expect(dlg).toContainText('Choose a different name.')
    await expect(go).toBeDisabled()
    await input.press('Enter')
    await expect(dlg).toBeVisible()

    await input.fill('clients')
    await expect(dlg).not.toContainText('Choose a different name.')
    await expect(input).not.toHaveAttribute('aria-invalid', 'true')
    await expect(go).toBeEnabled()
    expect(await executed(page)).toHaveLength(0)
  })

  test('previews the statement for the name as it is typed', async ({ page }) => {
    await openAdmin(page, 'customers', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    await dlg.getByLabel('New name').fill('clients')
    await expect(dlg.locator('.sql')).toHaveText('ALTER TABLE "public"."customers" RENAME TO "clients";')
    await dlg.getByLabel('New name').fill('buyers')
    await expect(dlg.locator('.sql')).toHaveText('ALTER TABLE "public"."customers" RENAME TO "buyers";')
    // Every preview is a preview: nothing ran.
    expect(await executed(page)).toHaveLength(0)
  })

  test('Enter renames, retitles the open tab, and the tree shows the new name', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await openTable(page, 'orders')
    await openAdmin(page, 'customers', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    // Typing and pressing Enter straight away must not lose to the preview request.
    await dlg.getByLabel('New name').fill('clients')
    await page.keyboard.press('Enter')

    await expect(dlg).toHaveCount(0)
    await expect(tableRow(page, 'clients')).toBeVisible()
    await expect(tableRow(page, 'customers')).toHaveCount(0)
    await expect(toast(page).filter({ hasText: 'Renamed table “customers” to “clients”' })).toBeVisible()
    expect(await tabTitles(page)).toEqual(['clients', 'orders'])
    // The retargeted tab loads the table under its new name.
    await tabNamed(page, 'clients').click()
    await expect(page.getByRole('grid').locator('.cell:not(.ph)').first()).toBeVisible()

    const ran = await executed(page)
    expect(ran).toHaveLength(1)
    expect(ran[0].args).toMatchObject({ table: { name: 'customers', schema: 'public' }, action: { kind: 'rename', newName: 'clients' }, preview: false })
  })

  test('the new name is trimmed', async ({ page }) => {
    await openAdmin(page, 'unique_only', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    await dlg.getByLabel('New name').fill('  spare  ')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(tableRow(page, 'spare')).toBeVisible()
    expect((await executed(page))[0].args.action).toEqual({ kind: 'rename', newName: 'spare' })
  })

  test('the Rename button also works, and Escape closes without renaming', async ({ page }) => {
    await openAdmin(page, 'orders', 'Rename…')
    await dialog(page, 'Rename table').getByLabel('New name').fill('sales')
    await page.keyboard.press('Escape')
    await expect(dialog(page, 'Rename table')).toHaveCount(0)
    await expect(tableRow(page, 'orders')).toBeVisible()
    expect(await executed(page)).toHaveLength(0)

    await openAdmin(page, 'orders', 'Rename…')
    // A fresh dialog starts from the real name again.
    await expect(dialog(page, 'Rename table').getByLabel('New name')).toHaveValue('orders')
    await dialog(page, 'Rename table').getByLabel('New name').fill('sales')
    await dialog(page, 'Rename table').getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(tableRow(page, 'sales')).toBeVisible()
  })

  test('a server error is shown in the dialog and the table keeps its name', async ({ page }) => {
    await failAdmin(page, 'relation "orders2" already exists')
    await openAdmin(page, 'orders', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    await dlg.getByLabel('New name').fill('orders2')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(dlg.getByRole('alert')).toContainText('relation "orders2" already exists')
    await expect(dlg).toBeVisible()
    await expect(tableRow(page, 'orders')).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Rename', exact: true })).toBeEnabled()
  })

  test('a view is renamed with ALTER VIEW', async ({ page }) => {
    await openAdmin(page, 'customer_totals', 'Rename…')
    const dlg = dialog(page, 'Rename view')
    await dlg.getByLabel('New name').fill('totals')
    await expect(dlg.locator('.sql')).toHaveText('ALTER VIEW "public"."customer_totals" RENAME TO "totals";')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(tableRow(page, 'totals')).toBeVisible()
    await expect(sidebar(page).getByRole('button', { name: 'Views', exact: true })).toBeVisible()
  })

  test('the renamed row keeps the keyboard focus position in the tree', async ({ page }) => {
    await openAdmin(page, 'orders', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    await dlg.getByLabel('New name').fill('sales')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(tableRow(page, 'sales')).toBeVisible()
    await expect(tableRow(page, 'sales')).toHaveAttribute('tabindex', '0')
  })
})

test.describe('accessibility', () => {
  test('dialog focus is trapped and returns to the tree row on cancel', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').focus()
    await page.keyboard.press('Delete')
    const dlg = dialog(page, 'Drop table')
    await expect(dlg.getByLabel('Type customers to confirm')).toBeFocused()
    // Tab walks input -> Cancel -> (Drop is disabled) -> back to the input.
    await page.keyboard.press('Tab')
    await expect(dlg.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await page.keyboard.press('Tab')
    await expect(dlg.getByLabel('Type customers to confirm')).toBeFocused()
    await page.keyboard.press('Shift+Tab')
    await expect(dlg.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(dlg).toHaveCount(0)
    await expect(tableRow(page, 'customers')).toBeFocused()
  })

  test('the statement is a labelled group and errors are announced as alerts', async ({ page }) => {
    await openConnection(page)
    await openAdmin(page, 'customers', 'Truncate…')
    await expect(dialog(page, 'Truncate table').getByRole('group', { name: 'Statement to run' })).toBeVisible()
  })

  test('the dialog is gone after success and focus is not lost to the page body', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'user').focus()
    await page.keyboard.press('Delete')
    await page.keyboard.type('user')
    await page.keyboard.press('Enter')
    await expect(tableRow(page, 'user')).toHaveCount(0)
    await expect.poll(() => page.evaluate(() => document.activeElement?.tagName)).not.toBe('BODY')
  })
})

const drafts = (page: Page) => page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('tablesail:draft:')))

/** Stages an edit of the first customer's name in the open customers tab. */
async function stageEdit(page: Page, value: string) {
  await gridCell(page, 0, 2).dblclick()
  await page.locator('.edit-host').locator('input, select').fill(value)
  await page.keyboard.press('Enter')
  await expect(bottomBar(page).locator('.pending-label')).toHaveText('1 change')
  await expect.poll(async () => (await drafts(page)).length).toBeGreaterThan(0)
}

test.describe('pending changes of the table', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await stageEdit(page, 'Alicia')
  })

  test('a rename carries them to the renamed table, which offers them back', async ({ page }) => {
    await openAdmin(page, 'customers', 'Rename…')
    const dlg = dialog(page, 'Rename table')
    await dlg.getByLabel('New name').fill('clients')
    await dlg.getByRole('button', { name: 'Rename', exact: true }).click()
    await expect(dlg).toHaveCount(0)

    const prompt = dialog(page, 'Unsaved changes found')
    await expect(prompt).toBeVisible()
    await expect(prompt).toContainText('clients')
    await prompt.getByRole('button', { name: 'Restore' }).click()
    await expect(bottomBar(page).locator('.pending-label')).toHaveText('1 change')
    await expect(gridCell(page, 0, 2)).toHaveText('Alicia')
    expect((await drafts(page)).every((k) => k.endsWith('.clients'))).toBe(true)
  })

  test('a drop forgets them, so a table created later under that name starts clean', async ({ page }) => {
    await openAdmin(page, 'customers', 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await dlg.getByLabel('Type customers to confirm').fill('customers')
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()
    await expect(tableRow(page, 'customers')).toHaveCount(0)
    expect(await drafts(page)).toEqual([])
  })

  test('a truncate offers them back on the reloaded table', async ({ page }) => {
    await openAdmin(page, 'customers', 'Truncate…')
    await dialog(page, 'Truncate table').getByRole('button', { name: 'Truncate', exact: true }).click()
    await expect(dialog(page, 'Unsaved changes found')).toBeVisible()
  })
})

test.describe('an open transaction and locks', () => {
  /** A Manual-mode query tab with an UPDATE run in it, so the transaction is open. */
  async function openTransaction(page: Page) {
    await newQueryTab(page)
    await page.getByRole('group', { name: 'Transaction mode' }).getByRole('button', { name: 'Manual' }).click()
    await setSql(page, "UPDATE customers SET name = 'x' WHERE id = 1")
    await runStatement(page)
    await expect(page.locator('.tab-body:not([hidden]) .txpill')).toContainText('1 statement')
  }

  test('the dialog says so while a transaction is open on the connection, and only then', async ({ page }) => {
    await openConnection(page)
    await openAdmin(page, 'orders', 'Drop…')
    await expect(dialog(page, 'Drop table')).toBeVisible()
    await expect(dialog(page, 'Drop table').getByRole('note')).toHaveCount(0)
    await page.keyboard.press('Escape')

    await openTransaction(page)
    await openAdmin(page, 'orders', 'Rename…')
    await expect(dialog(page, 'Rename table').getByRole('note')).toContainText('A transaction is open on this connection')
  })

  /** Holds the next executed table_admin until `window.__admin.resolve()` / `.reject(message)`. */
  async function holdAdmin(page: Page) {
    await page.evaluate(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (c: string, a?: { preview?: boolean }) => Promise<unknown> }
        __admin: { resolve: () => void; reject: (m: string) => void }
      }
      const orig = w.__TAURI_INTERNALS__.invoke
      w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
        if (cmd !== 'table_admin' || args?.preview) return orig(cmd, args)
        return new Promise((resolve, reject) => {
          w.__admin = {
            resolve: () => resolve(orig(cmd, args)),
            reject: (m) => reject({ kind: 'database', message: m, code: null, statement: null }),
          }
        })
      }
    })
  }

  async function startDrop(page: Page, name: string) {
    await openAdmin(page, name, 'Drop…')
    const dlg = dialog(page, 'Drop table')
    await dlg.getByLabel(`Type ${name} to confirm`).fill(name)
    await dlg.getByRole('button', { name: 'Drop', exact: true }).click()
    return dlg
  }

  test('a statement that waits for a lock cannot be closed at first, then can, and still finishes', async ({ page }) => {
    await openConnection(page)
    await holdAdmin(page)
    const dlg = await startDrop(page, 'empty_table')
    // Busy: nothing to press, Escape does nothing.
    await expect(dlg.getByRole('button', { name: 'Cancel' })).toBeDisabled()
    await page.keyboard.press('Escape')
    await expect(dlg).toBeVisible()
    await expect(dlg.getByRole('status')).toHaveCount(0)

    await expect(dlg.getByRole('status')).toContainText('Still waiting for the database', { timeout: 10_000 })
    const close = dlg.getByRole('button', { name: 'Close' })
    await expect(close).toBeEnabled()
    await close.click()
    await expect(dlg).toHaveCount(0)
    // The statement was not cancelled: when the lock goes, the drop completes as usual.
    await expect(tableRow(page, 'empty_table')).toBeVisible()
    await page.evaluate(() => (window as unknown as { __admin: { resolve: () => void } }).__admin.resolve())
    await expect(tableRow(page, 'empty_table')).toHaveCount(0)
    await expect(toast(page).filter({ hasText: 'Dropped table “empty_table”' })).toBeVisible()
  })

  test('Escape closes it too once it is waiting, and a failure afterwards is reported in a toast', async ({ page }) => {
    await openConnection(page)
    await holdAdmin(page)
    const dlg = await startDrop(page, 'empty_table')
    await expect(dlg.getByRole('status')).toBeVisible({ timeout: 10_000 })
    await page.keyboard.press('Escape')
    await expect(dlg).toHaveCount(0)
    await page.evaluate(() => (window as unknown as { __admin: { reject: (m: string) => void } }).__admin.reject('canceling statement due to lock timeout'))
    await expect(toast(page).filter({ hasText: 'lock timeout' })).toBeVisible()
    await expect(tableRow(page, 'empty_table')).toBeVisible()
  })

  test('finishing after the dialog was closed does not close a dialog opened meanwhile', async ({ page }) => {
    await openConnection(page)
    await holdAdmin(page)
    const dlg = await startDrop(page, 'empty_table')
    await expect(dlg.getByRole('status')).toBeVisible({ timeout: 10_000 })
    await dlg.getByRole('button', { name: 'Close' }).click()
    await openAdmin(page, 'orders', 'Truncate…')
    await expect(dialog(page, 'Truncate table')).toBeVisible()
    await page.evaluate(() => (window as unknown as { __admin: { resolve: () => void } }).__admin.resolve())
    await expect(tableRow(page, 'empty_table')).toHaveCount(0)
    await expect(dialog(page, 'Truncate table')).toBeVisible()
  })
})
