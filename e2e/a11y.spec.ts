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
  sqlEditor,
  tableRow,
} from './helpers'

/*
 * Accessibility gaps, kept as executable documentation.
 *
 * Every test in this file asserts the accessible behaviour the app SHOULD have and is marked `test.fail()`
 * because it does not have it today. When one of these gaps is fixed its test starts to "unexpectedly pass"
 * and Playwright reports it: delete the `test.fail()` line and the test becomes a regression test.
 *
 * Tests that document behaviour that already works carry no `test.fail()`.
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
    // GAP: <div role="grid"> in DataGrid.svelte has no aria-label, so screen readers announce just "grid".
    test.fail()
    await expect(page.getByRole('grid', { name: /customers/ })).toBeVisible(SHORT)
  })

  test('exposes column headers, rows and cells', async ({ page }) => {
    // GAP: DataGrid renders absolutely positioned <div>s with no role=columnheader / row / gridcell, so the
    // grid has aria-rowcount/aria-colcount but no content a screen reader can walk through.
    test.fail()
    await expect(page.getByRole('grid').getByRole('columnheader')).toHaveCount(3, SHORT)
    await expect(page.getByRole('grid').getByRole('row').first()).toBeVisible(SHORT)
    await expect(page.getByRole('grid').getByRole('gridcell').first()).toBeVisible(SHORT)
  })

  test('marks the selected cell (aria-selected or aria-activedescendant)', async ({ page }) => {
    // GAP: selection is only a painted rectangle (.sel / .active), nothing in the accessibility tree changes.
    test.fail()
    await gridCell(page, 0, 1).click()
    const selected = await page.getByRole('grid').evaluate(
      (g) => !!g.getAttribute('aria-activedescendant') || !!g.querySelector('[aria-selected="true"]'),
    )
    expect(selected).toBe(true)
  })

  test('sortable column headers announce their sort state', async ({ page }) => {
    // GAP: the header cells have no aria-sort and the arrow is a text glyph (▲/▼) inside the cell.
    test.fail()
    await page.getByRole('grid').locator('.h').filter({ hasText: 'name' }).click()
    await expect(page.locator('[aria-sort="ascending"]')).toHaveCount(1, SHORT)
  })

  test('the row-gutter "#" corner and the column resize handles are labelled', async ({ page }) => {
    // GAP: .corner (select all) and .resize (column resize) are plain <div>s with click/pointer handlers only.
    test.fail()
    await expect(page.getByRole('button', { name: /select all/i })).toBeVisible(SHORT)
  })
})

test.describe('tabs', () => {
  test('a tab’s accessible name is its title, not "Close tab <title>"', async ({ page }) => {
    // GAP: the close <button aria-label="Close tab"> is nested inside role=tab, so the tab's name becomes
    // "Close tab customers" and there is an interactive element inside a tab (TabBar.svelte).
    test.fail()
    await openConnection(page)
    await openTable(page, 'customers')
    await expect(page.getByRole('tab', { name: 'customers', exact: true })).toBeVisible(SHORT)
  })

  test('close buttons say which tab they close', async ({ page }) => {
    // GAP: every close button is just "Close tab" (TabBar.svelte); with several tabs they are indistinguishable.
    test.fail()
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await expect(page.getByRole('button', { name: 'Close tab customers' })).toBeVisible(SHORT)
  })

  test('the active tab can be reached with the Tab key (roving tabindex)', async ({ page }) => {
    // GAP: tabs, their close buttons and the +/arrow buttons are tabindex="-1", so keyboard users can only
    // switch tabs with ⌘1-9 / Ctrl+Tab and can never focus the strip.
    test.fail()
    await openConnection(page)
    await openTable(page, 'customers')
    await openTable(page, 'orders')
    await expect(page.locator('.strip [role=tab][aria-selected=true]')).toHaveAttribute('tabindex', '0', SHORT)
  })

  test('the Data / Structure / Relations tabs control a tabpanel', async ({ page }) => {
    // GAP: TableView's bottom-bar role=tab buttons have no aria-controls and no role=tabpanel exists.
    test.fail()
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await expect(page.getByRole('tabpanel')).toHaveCount(1, SHORT)
  })
})

test.describe('connection list', () => {
  test('cards live in a listbox', async ({ page }) => {
    // GAP: each card is role="option" but there is no role="listbox" ancestor, which is invalid ARIA and
    // makes the options unreachable for most screen readers (Welcome.svelte .cards).
    test.fail()
    await gotoApp(page)
    await expect(page.getByRole('listbox')).toBeVisible(SHORT)
  })

  test('the Open and Actions buttons say which connection they belong to', async ({ page }) => {
    // GAP: seven buttons all called "Open" and seven all called "Actions".
    test.fail()
    await gotoApp(page)
    await expect(page.getByRole('button', { name: /Open.*Local Postgres/ })).toBeVisible(SHORT)
    await expect(page.getByRole('button', { name: /Actions.*Local Postgres/ })).toBeVisible(SHORT)
  })

  test('the selected card is in the tab order', async ({ page }) => {
    // GAP: cards are tabindex="-1"; only a window-level keydown handler moves the (visual) selection, so the
    // list is not a single tab stop with arrow-key navigation (Welcome.svelte .card).
    test.fail()
    await gotoApp(page)
    await expect(card(page, 'Production')).toHaveAttribute('tabindex', '0', SHORT)
  })

  test('form validation messages are announced (role=alert)', async ({ page }) => {
    // GAP: ConnectionForm shows "Host is required." in a plain <div class="notice bad">.
    test.fail()
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
    // GAP: rows are role="treeitem" but nothing is role="tree" / role="group" (Sidebar.svelte .list).
    test.fail()
    await openConnection(page)
    await expect(page.getByRole('tree')).toBeVisible(SHORT)
  })

  test('the expand/collapse chevron is a real control', async ({ page }) => {
    // GAP: .twist is a <span> with a click handler: no role, no name, not focusable. (Right/Left on a focused
    // row do expand/collapse, but a pointer-less user cannot discover that.)
    test.fail()
    await openConnection(page)
    await expect(tableRow(page, 'customers').getByRole('button', { name: /expand|collapse|columns/i })).toBeVisible(SHORT)
  })

  test('the sidebar search and the Items/Queries/History tabs are in one landmark with a name', async ({ page }) => {
    // GAP: the <aside> landmarks (sidebar and details panel) have no aria-label, so they are both just
    // "complementary".
    test.fail()
    await openConnection(page)
    await expect(page.getByRole('complementary', { name: /sidebar|navigator|database/i })).toBeVisible(SHORT)
  })
})

test.describe('toolbar toggles', () => {
  test('toggle buttons expose their state with aria-pressed or aria-expanded', async ({ page }) => {
    // GAP: "Toggle sidebar", "Toggle query log", "AI assistant" and "Toggle details panel" only change a CSS
    // class (.on); assistive tech cannot tell whether the panel is open (TopBar.svelte).
    test.fail()
    await openConnection(page)
    const sidebarToggle = page.getByRole('button', { name: 'Toggle sidebar' })
    const pressed = await sidebarToggle.getAttribute('aria-pressed')
    const expanded = await sidebarToggle.getAttribute('aria-expanded')
    expect(pressed ?? expanded).not.toBeNull()
  })

  test('the Settings button is named like its tooltip', async ({ page }) => {
    // GAP: the "…" button has aria-label="Menu" but title="Settings"; the name and the tooltip disagree and the
    // button opens an app menu rather than only settings.
    test.fail()
    await openConnection(page)
    await expect(page.getByRole('button', { name: 'Menu' })).toHaveAttribute('title', 'Menu', SHORT)
  })
})

test.describe('command palette', () => {
  test('is a combobox controlling a listbox with an active descendant', async ({ page }) => {
    // GAP: CommandPalette has a plain text input and <div role="option">s without a listbox; arrow keys change a
    // CSS highlight but the input never announces the highlighted option (no aria-activedescendant).
    test.fail()
    await openConnection(page)
    await page.keyboard.press(`${MOD}+k`)
    const palette = page.getByRole('dialog', { name: 'Command palette' })
    await expect(palette.getByRole('combobox')).toBeVisible(SHORT)
    await expect(palette.getByRole('listbox')).toBeVisible(SHORT)
  })
})

test.describe('dialogs', () => {
  test('keep the keyboard focus inside while open (Tab does not escape to the page behind)', async ({ page }) => {
    // GAP: Modal.svelte has no focus trap. With Settings open, repeated Tab walks out of the dialog into the page
    // behind the backdrop (and into the window chrome in a real browser).
    test.fail()
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
    // GAP: Modal.svelte never focuses anything; the trigger button keeps focus (see also the Save-query test in
    // sql-editor.spec.ts and the command palette tests in palette-settings.spec.ts).
    test.fail()
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await expect(dlg).toBeVisible()
    const inside = await dlg.evaluate((el) => el.contains(document.activeElement))
    expect(inside).toBe(true)
  })

  test('return focus to the control that opened them', async ({ page }) => {
    // GAP: after Escape the focus falls back to <body>, so a keyboard user restarts from the top of the page.
    test.fail()
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
    // GAP: ContextMenu / RowMenu only handle Escape; ArrowDown/Enter do nothing, so the table and cell menus are
    // pointer-only. The menu element is focused (tabindex=-1) but items are not.
    test.fail()
    await openConnection(page)
    await tableRow(page, 'customers').click({ button: 'right' })
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('menuitem').first()).toBeFocused(SHORT)
  })
})

test.describe('SQL editor', () => {
  test('the editor has an accessible name', async ({ page }) => {
    // GAP: CodeMirror's contenteditable is role=textbox aria-multiline with no aria-label: announced as "edit text".
    test.fail()
    await openConnection(page)
    await newQueryTab(page)
    await expect(sqlEditor(page)).toHaveAttribute('aria-label', /.+/, SHORT)
  })

  test('the result and message tabs belong to a labelled region', async ({ page }) => {
    // GAP: results/messages are role=tab buttons without a tabpanel.
    test.fail()
    await openConnection(page)
    await newQueryTab(page)
    await expect(page.getByRole('tabpanel')).toBeVisible(SHORT)
  })

  test('the database picker in the editor toolbar has a name', async ({ page }) => {
    // GAP: <label title="Database"><Icon/><select>…</select></label> — the select's name comes from the label's
    // text, which is empty (an icon), and `title` on a label is not used.
    test.fail()
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
    // GAP: the label is a sibling <div class="label"> and the value is an <input> with no aria-label / for=id, so
    // every field is an unnamed textbox.
    test.fail()
    await expect(inspector(page).getByRole('textbox', { name: 'email' })).toBeVisible(SHORT)
  })

  test('the per-field options buttons say which field they belong to', async ({ page }) => {
    // GAP: all of them are "Field options".
    test.fail()
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
    test.fail()
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
