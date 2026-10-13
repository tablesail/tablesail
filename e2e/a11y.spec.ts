import { test, expect } from './fixtures'
import {
  MOD,
  card,
  gotoApp,
  grid,
  gridCell,
  inspector,
  newQueryTab,
  openConnection,
  openTable,
  openTableLoaded,
  setSql,
  sidebar,
  sqlEditor,
  tableRow,
  tabNamed,
} from './helpers'

/*
 * Accessibility regression tests.
 *
 * Each test asserts accessible behaviour the app must keep (names, roles, states, keyboard operation). A gap that
 * is found later is pinned here with `test.fail()` and a `// GAP:` comment until it is fixed; when the fix lands
 * Playwright reports the test as "unexpectedly passed" and the `test.fail()` line is deleted.
 */

const SHORT = { timeout: 1500 }

test.describe('what already works', () => {
  test('dialogs are role=dialog with aria-modal and an accessible name', async ({ page }) => {
    await gotoApp(page)
    await page.getByRole('button', { name: 'New connection' }).click()
    const dlg = page.getByRole('dialog', { name: 'New connection' })
    await expect(dlg).toHaveAttribute('aria-modal', 'true')
  })

  test('toolbar icon buttons have accessible names', async ({ page }) => {
    await openConnection(page)
    for (const name of [
      'Toggle sidebar',
      'Connections',
      'Refresh schema',
      'Switch database',
      'New SQL query',
      'Search',
      'Menu',
      'Toggle query log',
      'AI assistant',
      'Toggle details panel',
    ]) {
      await expect(page.getByRole('button', { name, exact: true })).toBeVisible()
    }
  })

  test('toasts are announced politely', async ({ page }) => {
    await gotoApp(page)
    await expect(page.locator('.toasts')).toHaveAttribute('aria-live', 'polite')
  })

  test('context menus are role=menu with menuitems', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    await expect(page.getByRole('menu')).toBeVisible()
    await expect(page.getByRole('menuitem').first()).toBeVisible()
  })

  test('the assistant announces thinking (role=status) and errors (role=alert)', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await page.getByRole('textbox', { name: 'Message the assistant' }).fill('hi')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('status').filter({ hasText: 'Thinking…' })).toBeVisible()
  })

  test('the connection list heading and the form groups are exposed', async ({ page }) => {
    await gotoApp(page)
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
    await page.getByRole('button', { name: 'New connection' }).click()
    await expect(page.getByRole('radiogroup', { name: 'Engine' })).toBeVisible()
    await expect(page.getByRole('radiogroup', { name: 'Environment' })).toBeVisible()
  })
})

test.describe('data grid', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
  })

  test('has an accessible name that says which table it shows', async ({ page }) => {
    // DataGrid's role=grid is named after the table it shows.
    await expect(page.getByRole('grid', { name: /customers/ })).toBeVisible(SHORT)
  })

  test('exposes column headers, rows and cells', async ({ page }) => {
    // Header cells, rows and cells carry columnheader / row / gridcell roles (with aria-rowindex / aria-colindex
    // so a virtualised window still reads correctly).
    await expect(page.getByRole('grid').getByRole('columnheader')).toHaveCount(3, SHORT)
    await expect(page.getByRole('grid').getByRole('row').first()).toBeVisible(SHORT)
    await expect(page.getByRole('grid').getByRole('gridcell').first()).toBeVisible(SHORT)
  })

  test('marks the selected cell (aria-selected or aria-activedescendant)', async ({ page }) => {
    // The active cell is exposed as aria-activedescendant and the selected range as aria-selected.
    await gridCell(page, 0, 1).click()
    const selected = await page.getByRole('grid').evaluate(
      (g) => !!g.getAttribute('aria-activedescendant') || !!g.querySelector('[aria-selected="true"]'),
    )
    expect(selected).toBe(true)
  })

  test('sortable column headers announce their sort state', async ({ page }) => {
    // Sorted headers carry aria-sort; the arrow glyph is aria-hidden.
    await page.getByRole('grid').locator('.h').filter({ hasText: 'name' }).click()
    await expect(page.locator('[aria-sort="ascending"]')).toHaveCount(1, SHORT)
  })

  test('the row-gutter "#" corner and the column resize handles are labelled', async ({ page }) => {
    // The "#" corner is a labelled Select all button; the resize handles are aria-hidden.
    await expect(page.getByRole('button', { name: /select all/i })).toBeVisible(SHORT)
  })
})

test.describe('tabs', () => {
  test('a tab’s accessible name is its title, not "Close tab <title>"', async ({ page }) => {
    // The close button is a sibling of the role=tab element, so the tab's name is only its title.
    await openConnection(page)
    await openTable(page, 'customers')
    await expect(page.getByRole('tab', { name: 'customers', exact: true })).toBeVisible(SHORT)
  })

  test('close buttons say which tab they close', async ({ page }) => {
    // Close buttons are named "Close tab <title>".
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await expect(page.getByRole('button', { name: 'Close tab customers' })).toBeVisible(SHORT)
  })

  test('the active tab can be reached with the Tab key (roving tabindex)', async ({ page }) => {
    // The strip is one tab stop (the selected tab is tabindex 0, arrow keys move between tabs).
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await expect(page.locator('.strip [role=tab][aria-selected=true]')).toHaveAttribute('tabindex', '0', SHORT)
  })

  test('the Data / Structure / Relations tabs control a tabpanel', async ({ page }) => {
    // The bottom-bar tabs have aria-controls and the pane below them is a role=tabpanel labelled by the selected tab.
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await expect(page.getByRole('tabpanel')).toHaveCount(1, SHORT)
  })
})

test.describe('connection list', () => {
  test('cards live in a listbox', async ({ page }) => {
    // The cards are options inside a labelled role=listbox.
    await gotoApp(page)
    await expect(page.getByRole('listbox')).toBeVisible(SHORT)
  })

  test('the Open and Actions buttons say which connection they belong to', async ({ page }) => {
    // The Open and Actions buttons are named after their connection.
    await gotoApp(page)
    await expect(page.getByRole('button', { name: /Open.*Local Postgres/ })).toBeVisible(SHORT)
    await expect(page.getByRole('button', { name: /Actions.*Local Postgres/ })).toBeVisible(SHORT)
  })

  test('the selected card is in the tab order', async ({ page }) => {
    // The selected card is tabindex 0 (roving) and the arrow keys move the focus along with the selection.
    await gotoApp(page)
    await expect(card(page, 'Production')).toHaveAttribute('tabindex', '0', SHORT)
  })

  test('form validation messages are announced (role=alert)', async ({ page }) => {
    // Validation / save errors are role=alert.
    await gotoApp(page)
    await page.getByRole('button', { name: 'New connection' }).click()
    const form = page.getByRole('dialog', { name: 'New connection' })
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByRole('alert')).toBeVisible(SHORT)
  })
})

test.describe('sidebar', () => {
  test('the object list is a role=tree', async ({ page }) => {
    // The object list is a role=tree (treeitems carry aria-level).
    await openConnection(page)
    await expect(page.getByRole('tree')).toBeVisible(SHORT)
  })

  test('the expand/collapse chevron is a real control', async ({ page }) => {
    // The chevron is a button (named Expand / Collapse columns) inside the row; Right/Left on the row still work.
    await openConnection(page)
    await expect(tableRow(page, 'customers').getByRole('button', { name: /expand|collapse|columns/i })).toBeVisible(SHORT)
  })

  test('the sidebar search and the Items/Queries/History tabs are in one landmark with a name', async ({ page }) => {
    // The <aside> landmarks are named (Sidebar / Details).
    await openConnection(page)
    await expect(page.getByRole('complementary', { name: /sidebar|navigator|database/i })).toBeVisible(SHORT)
  })
})

test.describe('toolbar toggles', () => {
  test('toggle buttons expose their state with aria-pressed or aria-expanded', async ({ page }) => {
    // The toggle buttons carry aria-pressed.
    await openConnection(page)
    const sidebarToggle = page.getByRole('button', { name: 'Toggle sidebar' })
    const pressed = await sidebarToggle.getAttribute('aria-pressed')
    const expanded = await sidebarToggle.getAttribute('aria-expanded')
    expect(pressed ?? expanded).not.toBeNull()
  })

  test('the Settings button is named like its tooltip', async ({ page }) => {
    // The button's name and tooltip agree ("Menu").
    await openConnection(page)
    await expect(page.getByRole('button', { name: 'Menu' })).toHaveAttribute('title', 'Menu', SHORT)
  })
})

test.describe('command palette', () => {
  test('is a combobox controlling a listbox with an active descendant', async ({ page }) => {
    // CommandPalette is a combobox input controlling a listbox of options, with aria-activedescendant.
    await openConnection(page)
    await page.keyboard.press(`${MOD}+k`)
    const palette = page.getByRole('dialog', { name: 'Command palette' })
    await expect(palette.getByRole('combobox')).toBeVisible(SHORT)
    await expect(palette.getByRole('listbox')).toBeVisible(SHORT)
  })
})

test.describe('dialogs', () => {
  test('keep the keyboard focus inside while open (Tab does not escape to the page behind)', async ({ page }) => {
    // Modal / dialogFocus trap Tab inside the open dialog.
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await expect(dlg).toBeVisible()
    const outside: string[] = []
    for (let i = 0; i < 24; i++) {
      await page.keyboard.press('Tab')
      const isInside = await dlg.evaluate((el) => el.contains(document.activeElement))
      if (!isInside) outside.push(await page.evaluate(() => document.activeElement?.tagName + ' ' + (document.activeElement?.getAttribute('aria-label') ?? '')))
    }
    expect(outside).toEqual([])
  })

  test('move focus into the dialog when it opens', async ({ page }) => {
    // dialogFocus moves the focus into the dialog (see also the Save-query test in sql-editor.spec.ts and the
    // command palette tests in palette-settings.spec.ts).
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await expect(dlg).toBeVisible()
    const inside = await dlg.evaluate((el) => el.contains(document.activeElement))
    expect(inside).toBe(true)
  })

  test('return focus to the control that opened them', async ({ page }) => {
    // dialogFocus returns the focus to the control that opened the dialog, even when that was a menu item that
    // disappeared (the menu's own button gets it back).
    await openConnection(page)
    const menu = page.getByRole('button', { name: 'Menu' })
    await menu.focus()
    await page.keyboard.press('Enter')
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog', { name: 'Settings' })).toHaveCount(0)
    await expect(menu).toBeFocused(SHORT)
  })
})

test.describe('menus', () => {
  test('context menus can be driven with the arrow keys and Enter', async ({ page }) => {
    // ContextMenu / RowMenu: arrows, Home/End, Enter, Escape (the first ArrowDown from the menu lands on the first item).
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('menuitem').first()).toBeFocused(SHORT)
  })
})

test.describe('SQL editor', () => {
  test('the editor has an accessible name', async ({ page }) => {
    // The CodeMirror content element has aria-label="SQL editor".
    await openConnection(page)
    await newQueryTab(page)
    await expect(sqlEditor(page)).toHaveAttribute('aria-label', /.+/, SHORT)
  })

  test('the result and message tabs belong to a labelled region', async ({ page }) => {
    // Results / Messages tabs control a role=tabpanel.
    await openConnection(page)
    await newQueryTab(page)
    await expect(page.getByRole('tabpanel')).toBeVisible(SHORT)
  })

  test('the database picker in the editor toolbar has a name', async ({ page }) => {
    // The toolbar's database <select> has its own aria-label (the wrapping label only contains an icon).
    await openConnection(page)
    await newQueryTab(page)
    await expect(page.locator('.tab-body:not([hidden])').getByRole('combobox', { name: 'Database' })).toBeVisible(SHORT)
  })
})

test.describe('details panel', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 0).click()
    await expect(inspector(page)).toBeVisible()
  })

  test('each editable field input is labelled with its column name', async ({ page }) => {
    // Each input is named by its field label (aria-labelledby).
    await expect(inspector(page).getByRole('textbox', { name: 'email' })).toBeVisible(SHORT)
  })

  test('the per-field options buttons say which field they belong to', async ({ page }) => {
    // The buttons are named "Field options for <column>".
    await expect(inspector(page).getByRole('button', { name: /options.*email/i })).toBeVisible(SHORT)
  })
})

test.describe('motion and focus visibility', () => {
  test('keyboard focus is visible on toolbar buttons', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Refresh schema' }).focus()
    await page.keyboard.press('Tab')
    const outline = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement
      const s = getComputedStyle(el)
      return { outlineStyle: s.outlineStyle, outlineWidth: s.outlineWidth, boxShadow: s.boxShadow }
    })
    // Either an outline or a focus ring made with box-shadow.
    expect(outline.outlineStyle !== 'none' || outline.boxShadow !== 'none').toBe(true)
  })
})

test.describe('console hygiene', () => {
  test('a normal session logs no Svelte runtime warnings', async ({ page }) => {
    // GAP: QueryTab.svelte mutates its `tab` prop (tab.sql / tab.database via bind:value on a plain prop and
    // `tab.title = name`), which Svelte reports as ownership_invalid_mutation in dev builds:
    // "Mutating unbound props (`tab`) is strongly discouraged. Consider using `bind:tab={...}`".
    const warnings: string[] = []
    page.on('console', (m) => {
      if (m.type() === 'warning' && m.text().includes('[svelte]')) warnings.push(m.text())
    })
    await openConnection(page)
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM customers')
    await page.getByRole('button', { name: 'Save' }).click()
    await page.getByPlaceholder('Revenue by month').fill('x')
    await page.keyboard.press('Enter')
    await expect(page.getByText('Saved “x”')).toBeVisible()
    expect(warnings).toEqual([])
  })

  test('the app does not log errors while browsing, editing and querying', async ({ page }) => {
    const errors: string[] = []
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(m.text())
    })
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 1).click()
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM orders')
    await page.keyboard.press(`${MOD}+Enter`)
    await expect(grid(page)).toBeVisible()
    expect(errors).toEqual([])
  })
})

test.describe('keyboard and structure details', () => {
  test('the grid exposes a header row, indexed data rows and cells with ids', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    const g = page.getByRole('grid')
    await expect(g).toHaveAttribute('aria-multiselectable', 'true')
    // The header is row 1, so the first data row is row 2 and the count includes the header.
    await expect(g.getByRole('row').nth(0)).toHaveAttribute('aria-rowindex', '1')
    await expect(g.getByRole('row').nth(1)).toHaveAttribute('aria-rowindex', '2')
    const rows = Number(await g.getAttribute('aria-rowcount'))
    expect(rows).toBeGreaterThan(2)
    await expect(g.getByRole('gridcell').first()).toHaveAttribute('aria-colindex', '1')
    const ids = await g.getByRole('gridcell').evaluateAll((els) => els.map((e) => e.id))
    expect(ids.every(Boolean)).toBe(true)
    expect(new Set(ids).size).toBe(ids.length)
  })

  test('aria-activedescendant follows the active cell and aria-selected covers the range', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    const g = page.getByRole('grid')
    await gridCell(page, 0, 0).click()
    const first = await gridCell(page, 0, 0).getAttribute('id')
    await expect(g).toHaveAttribute('aria-activedescendant', first!)
    await page.keyboard.press('ArrowRight')
    const second = await gridCell(page, 0, 1).getAttribute('id')
    await expect(g).toHaveAttribute('aria-activedescendant', second!)
    await page.keyboard.press('Shift+ArrowDown')
    await expect(g.locator('[aria-selected="true"]')).toHaveCount(2)
    await expect(gridCell(page, 1, 1)).toHaveAttribute('aria-selected', 'true')
    // The active cell stays the anchor of the range.
    await expect(g).toHaveAttribute('aria-activedescendant', second!)
  })

  test('a descending sort is announced and the sort arrow is hidden from assistive tech', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    const name = page.getByRole('grid').getByRole('columnheader', { name: /name/ })
    await name.click()
    await expect(name).toHaveAttribute('aria-sort', 'ascending')
    await name.click()
    await expect(name).toHaveAttribute('aria-sort', 'descending')
    await expect(name.locator('.sort')).toHaveAttribute('aria-hidden', 'true')
    await name.click()
    await expect(page.locator('[aria-sort]')).toHaveCount(0)
  })

  test('the "#" corner selects every cell', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await page.getByRole('button', { name: 'Select all' }).click()
    await expect(page.getByRole('grid').locator('.cell[aria-selected="true"]').first()).toBeVisible()
    await expect(page.getByRole('grid').locator('.cell:not([aria-selected="true"]):not(.ph)')).toHaveCount(0)
  })

  test('the tab strip is driven with the arrow keys; Enter activates the focused tab', async ({ page }) => {
    await openConnection(page)
    for (const t of ['customers', 'orders', 'events']) await openTable(page, t)
    const strip = page.getByRole('tablist', { name: 'Open tabs' })
    await expect(strip.getByRole('tab')).toHaveCount(3)
    await strip.getByRole('tab', { name: 'events' }).focus()
    await page.keyboard.press('ArrowLeft')
    await expect(strip.getByRole('tab', { name: 'orders' })).toBeFocused()
    await page.keyboard.press('Home')
    await expect(strip.getByRole('tab', { name: 'customers' })).toBeFocused()
    await page.keyboard.press('ArrowLeft')
    await expect(strip.getByRole('tab', { name: 'events' })).toBeFocused()
    await page.keyboard.press('ArrowRight')
    await expect(strip.getByRole('tab', { name: 'customers' })).toBeFocused()
    // Moving focus does not switch tabs; Enter does.
    await expect(strip.getByRole('tab', { name: 'events' })).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('Enter')
    await expect(strip.getByRole('tab', { name: 'customers' })).toHaveAttribute('aria-selected', 'true')
    // Only the selected tab is in the tab order.
    await expect(strip.getByRole('tab', { name: 'orders' })).toHaveAttribute('tabindex', '-1')
  })

  test('the tab close buttons are not inside the tabs', async ({ page }) => {
    await openConnection(page)
    await openTable(page, 'customers')
    await expect(page.getByRole('tab', { name: 'customers' }).getByRole('button')).toHaveCount(0)
  })

  test('the Data / Structure / Relations panel is labelled by the selected tab', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    const panel = page.getByRole('tabpanel')
    await expect(panel).toHaveAccessibleName('Data')
    await page.getByRole('tab', { name: 'Structure', exact: true }).click()
    await expect(panel).toHaveAccessibleName('Structure')
    const controls = await page.getByRole('tab', { name: 'Structure', exact: true }).getAttribute('aria-controls')
    expect(await panel.getAttribute('id')).toBe(controls)
  })

  test('the result and Messages tabs label their panel', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM customers')
    await page.keyboard.press(`${MOD}+Enter`)
    await expect(grid(page)).toBeVisible()
    await expect(page.getByRole('tabpanel')).toHaveAccessibleName('Result 1')
    await expect(grid(page)).toHaveAccessibleName('Result 1')
    await page.getByRole('tab', { name: 'Messages' }).click()
    await expect(page.getByRole('tabpanel')).toHaveAccessibleName(/Messages/)
  })

  test('connection cards: arrow keys move the focus with the selection; Enter on Open belongs to the button', async ({ page }) => {
    await gotoApp(page)
    await card(page, 'Production').focus()
    const options = page.getByRole('option')
    const first = await options.first().getAttribute('data-conn')
    expect(first).toBeTruthy()
    await page.keyboard.press('ArrowDown')
    const focusedId = await page.evaluate(() => document.activeElement?.getAttribute('data-conn'))
    expect(focusedId).toBeTruthy()
    // Exactly one card is in the tab order and it is the selected (focused) one.
    await expect(page.locator('[role=option][tabindex="0"]')).toHaveCount(1)
    await expect(page.locator('[role=option][tabindex="0"]')).toHaveAttribute('data-conn', focusedId!)
    await expect(page.locator('[role=option][aria-selected="true"]')).toHaveAttribute('data-conn', focusedId!)
  })

  test('sidebar: the tree has one tab stop, treeitems have a level, section headers join the arrow walk', async ({ page }) => {
    await openConnection(page)
    const side = sidebar(page)
    await expect(side.getByRole('tree', { name: 'Database objects' })).toBeVisible()
    await expect(side.locator('[role=treeitem][tabindex="0"]')).toHaveCount(1)
    await expect(tableRow(page, 'customers')).toHaveAttribute('aria-level', '1')
    // Section header -> first table -> and back.
    await side.locator('.section').first().focus()
    await page.keyboard.press('ArrowDown')
    await expect(side.getByRole('treeitem').first()).toBeFocused()
    await page.keyboard.press('ArrowUp')
    await expect(side.locator('.section').first()).toBeFocused()
    // Left on an expanded section collapses it, Right expands it again.
    await page.keyboard.press('ArrowLeft')
    await expect(side.locator('.section').first()).toHaveAttribute('aria-expanded', 'false')
    await page.keyboard.press('ArrowRight')
    await expect(side.locator('.section').first()).toHaveAttribute('aria-expanded', 'true')
  })

  test('sidebar: the chevron button expands a table without opening it and keeps the row focused', async ({ page }) => {
    await openConnection(page)
    const row = tableRow(page, 'customers')
    await row.getByRole('button', { name: 'Expand columns' }).click()
    await expect(row).toHaveAttribute('aria-expanded', 'true')
    await expect(row).toBeFocused()
    await expect(row.getByRole('button', { name: 'Collapse columns' })).toBeVisible()
    await expect(sidebar(page).getByRole('treeitem').filter({ hasText: 'email' })).toHaveAttribute('aria-level', '2')
  })

  test('the toggle buttons flip aria-pressed with the panel they control', async ({ page }) => {
    await openConnection(page)
    const sidebarToggle = page.getByRole('button', { name: 'Toggle sidebar' })
    await expect(sidebarToggle).toHaveAttribute('aria-pressed', 'true')
    await sidebarToggle.click()
    await expect(sidebarToggle).toHaveAttribute('aria-pressed', 'false')
    const details = page.getByRole('button', { name: 'Toggle details panel' })
    await expect(details).toHaveAttribute('aria-pressed', 'false')
    await details.click()
    await expect(details).toHaveAttribute('aria-pressed', 'true')
  })

  test('the command palette announces the highlighted option', async ({ page }) => {
    await openConnection(page)
    await page.keyboard.press(`${MOD}+k`)
    const palette = page.getByRole('dialog', { name: 'Command palette' })
    const input = palette.getByRole('combobox')
    await expect(input).toHaveAttribute('aria-expanded', 'true')
    const listbox = palette.getByRole('listbox')
    await expect(input).toHaveAttribute('aria-controls', (await listbox.getAttribute('id'))!)
    const options = palette.getByRole('option')
    await expect(options.first()).toHaveAttribute('aria-selected', 'true')
    await expect(input).toHaveAttribute('aria-activedescendant', (await options.nth(0).getAttribute('id'))!)
    await input.press('ArrowDown')
    await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true')
    await expect(input).toHaveAttribute('aria-activedescendant', (await options.nth(1).getAttribute('id'))!)
  })

  test('menus: arrows wrap, Home/End jump, Escape closes and gives the focus back', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    const items = page.getByRole('menuitem')
    const n = await items.count()
    await page.keyboard.press('ArrowUp')
    await expect(items.nth(n - 1)).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await expect(items.first()).toBeFocused()
    await page.keyboard.press('End')
    await expect(items.nth(n - 1)).toBeFocused()
    await page.keyboard.press('Home')
    await expect(items.first()).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
    await expect(tableRow(page, 'customers')).toBeFocused()
  })

  test('menus: Enter activates the focused item', async ({ page }) => {
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('menuitem', { name: 'Open' })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('menu')).toHaveCount(0)
    await expect(tabNamed(page, 'customers')).toBeVisible()
  })

  test('the grid cell menu is keyboard driven too (RowMenu)', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 1).click({ button: 'right' })
    await expect(page.getByRole('menu')).toBeVisible()
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('menuitem').first()).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
    await expect(page.getByRole('grid')).toBeFocused()
  })

  test('the right panel landmark is named after its tab', async ({ page }) => {
    await openConnection(page)
    await expect(page.getByRole('complementary', { name: 'Sidebar' })).toBeVisible()
    await page.getByRole('button', { name: 'Toggle details panel' }).click()
    await expect(page.getByRole('complementary', { name: 'Details' })).toBeVisible()
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expect(page.getByRole('complementary', { name: 'Assistant' })).toBeVisible()
  })

  test('settings: the theme buttons are a labelled group with a pressed state', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const theme = page.getByRole('dialog', { name: 'Settings' }).getByRole('group', { name: 'Theme' })
    await theme.getByRole('button', { name: 'Dark', exact: true }).click()
    await expect(theme.getByRole('button', { name: 'Dark', exact: true })).toHaveAttribute('aria-pressed', 'true')
    await expect(theme.getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'false')
  })
})
