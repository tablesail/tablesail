import { test, expect } from './fixtures'
import {
  MOD,
  gotoApp,
  gridCell,
  inspector,
  openConnection,
  openTableLoaded,
  setSql,
  newQueryTab,
  sidebar,
  sqlText,
  tabNamed,
  tabTitles,
  card,
} from './helpers'
import type { Page } from '@playwright/test'

const paletteInput = (page: Page) => page.getByPlaceholder('What would you like to do?')
const palette = (page: Page) => page.getByRole('dialog', { name: 'Command palette' })
const options = (page: Page) => palette(page).getByRole('option')
const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })

/** Opens the palette and clicks its input, so the tests do not depend on the autofocus bug covered separately. */
async function openPalette(page: Page, shortcut = `${MOD}+k`) {
  await page.keyboard.press(shortcut)
  await expect(palette(page)).toBeVisible()
  await paletteInput(page).click()
}

test.describe('command palette', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
  })

  test('⌘K opens it; with no query it lists quick actions and a few tables', async ({ page }) => {
    await openPalette(page)
    const labels = (await options(page).locator('span').allTextContents()).map((t) => t.trim())
    expect(labels).toEqual(
      expect.arrayContaining([
        'New SQL editor',
        'Refresh schema',
        'Toggle row inspector',
        'Show query history',
        'Show saved queries',
        'Toggle appearance',
        'Add database connection',
        'Keyboard shortcuts',
        'Settings',
      ]),
    )
    const tables = labels.filter((l) => l.startsWith('Open '))
    expect(tables).toHaveLength(5)
    await expect(palette(page).getByText('Quick actions', { exact: true })).toBeVisible()
    await expect(palette(page).getByText('Tables', { exact: true })).toBeVisible()
    await expect(palette(page).getByText('Esc to close')).toBeVisible()
  })

  test('the first result is highlighted and shows its shortcut hint', async ({ page }) => {
    await openPalette(page)
    await expect(options(page).first()).toHaveAttribute('aria-selected', 'true')
    await expect(options(page).first()).toContainText('New SQL editor')
    await expect(options(page).first()).toContainText('⌘T')
  })

  test('⌘P opens it too', async ({ page }) => {
    await openPalette(page, `${MOD}+p`)
    await expect(paletteInput(page)).toBeVisible()
  })

  test('⌘K again closes it', async ({ page }) => {
    await openPalette(page)
    await page.keyboard.press(`${MOD}+k`)
    await expect(palette(page)).toHaveCount(0)
  })

  test('Escape closes it, and so does clicking the backdrop and the toolbar search button', async ({ page }) => {
    await openPalette(page)
    await page.keyboard.press('Escape')
    await expect(palette(page)).toHaveCount(0)

    await page.getByRole('button', { name: 'Search' }).click()
    await expect(palette(page)).toBeVisible()
    await page.mouse.click(10, 10)
    await expect(palette(page)).toHaveCount(0)
  })

  test('typing fuzzy-filters tables; Enter opens the highlighted one as a tab', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('ordr')
    await expect(options(page).first()).toContainText('Open orders')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toHaveCount(0)
    await expect(tabNamed(page, 'orders')).toBeVisible()
    await expect(page.getByRole('grid')).toBeVisible()
    await expect(page.getByRole('button', { name: /: public\.orders/ })).toBeVisible()
  })

  test('palette tables open as regular tabs, not previews', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('customers')
    await page.keyboard.press('Enter')
    await openPalette(page)
    await paletteInput(page).fill('orders')
    await page.keyboard.press('Enter')
    expect(await tabTitles(page)).toEqual(['customers', 'orders'])
  })

  test('clicking a result runs it', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('events')
    await options(page).filter({ hasText: 'Open events' }).click()
    await expect(tabNamed(page, 'events')).toBeVisible()
  })

  test('arrow keys move the highlight and Enter runs the highlighted entry', async ({ page }) => {
    await openPalette(page)
    await page.keyboard.press('ArrowDown')
    await expect(options(page).nth(1)).toHaveAttribute('aria-selected', 'true')
    await expect(options(page).first()).toHaveAttribute('aria-selected', 'false')
    await page.keyboard.press('ArrowUp')
    await expect(options(page).first()).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('ArrowUp') // does not wrap below zero
    await expect(options(page).first()).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('Enter')
    await expect(tabNamed(page, 'Query 1')).toBeVisible()
  })

  test('hovering an option highlights it', async ({ page }) => {
    await openPalette(page)
    await options(page).nth(3).hover()
    await expect(options(page).nth(3)).toHaveAttribute('aria-selected', 'true')
  })

  test('a query with no match says so, and typing again resets the highlight', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('qqqqqzzzz')
    await expect(palette(page).getByText('No matching actions')).toBeVisible()
    await page.keyboard.press('Enter') // nothing to run: stays open without error
    await expect(palette(page)).toBeVisible()
    await paletteInput(page).fill('sett')
    await expect(options(page).first()).toContainText('Settings')
    await expect(options(page).first()).toHaveAttribute('aria-selected', 'true')
  })

  test('the palette is case-insensitive', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('MIXEDCASE')
    await expect(options(page).first()).toContainText('Open MixedCase')
  })

  test('tables of other schemas are qualified', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('invoices')
    await expect(options(page).first()).toContainText('Open billing.invoices')
  })

  test('quick actions: New SQL editor, Toggle row inspector, history, saved queries', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('new sql')
    await page.keyboard.press('Enter')
    await expect(tabNamed(page, 'Query 1')).toBeVisible()

    await openPalette(page)
    await paletteInput(page).fill('row inspector')
    await page.keyboard.press('Enter')
    await expect(inspector(page)).toBeVisible()

    await openPalette(page)
    await paletteInput(page).fill('query history')
    await page.keyboard.press('Enter')
    await expect(sidebar(page).getByRole('tab', { name: 'History' })).toHaveAttribute('aria-selected', 'true')

    await openPalette(page)
    await paletteInput(page).fill('saved queries')
    await page.keyboard.press('Enter')
    await expect(sidebar(page).getByRole('tab', { name: 'Queries' })).toHaveAttribute('aria-selected', 'true')
  })

  test('quick action: Toggle appearance flips the theme', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('appearance')
    await page.keyboard.press('Enter')
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  })

  test('quick action: Keyboard shortcuts and Settings open their dialogs', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('keyboard')
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Keyboard shortcuts')).toBeVisible()
    await page.keyboard.press('Escape')

    await openPalette(page)
    await paletteInput(page).fill('settings')
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Settings')).toBeVisible()
  })

  test('quick action: Add database connection goes to the connection list', async ({ page }) => {
    await openPalette(page)
    await paletteInput(page).fill('add database')
    await options(page).filter({ hasText: 'Add database connection' }).click() // by mouse: see the Enter race below
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
  })

  test.fixme('Enter on "Add database connection" shows the list without also opening a connection', async ({ page }) => {
    // BUG (rare race, seen about once in 900 runs): the palette runs the action from its input's keydown, which
    // mounts the connection list synchronously enough for Welcome.svelte's window-level keydown handler (Enter
    // opens the selected card) to receive that very same Enter. Production, the first card, was opened instead
    // of just showing the list. Fix idea: ignore keydowns that started before the list mounted, or run the
    // palette action on keyup / in a microtask.
    await openPalette(page)
    await paletteInput(page).fill('add database')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await expect(page.getByText('Connecting…')).toHaveCount(0)
    await expect(page.getByText('Connected', { exact: true })).toHaveCount(1) // only the one that was already open
  })

  test('saved queries are listed and open in a new tab with their SQL', async ({ page }) => {
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM events')
    await page.getByRole('button', { name: 'Save' }).click()
    await dialog(page, 'Save query').getByPlaceholder('Revenue by month').fill('All events')
    await page.keyboard.press('Enter')
    await expect(dialog(page, 'Save query')).toHaveCount(0)

    await openPalette(page)
    await paletteInput(page).fill('all events')
    await expect(palette(page).getByText('Saved queries', { exact: true })).toBeVisible()
    await page.keyboard.press('Enter')
    expect(await tabTitles(page)).toHaveLength(2)
    expect(await sqlText(page)).toBe('SELECT * FROM events')
  })

  test('other open connections can be switched to from the palette', async ({ page }) => {
    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('button', { name: /^MySQL 8\.4\.6 : Local MySQL/ })).toBeVisible()
    await openPalette(page)
    await paletteInput(page).fill('switch to')
    await expect(options(page).first()).toContainText('Switch to Local Postgres')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ })).toBeVisible()
  })

  test('opening a table from the palette while the grid has focus works and keeps the editor state', async ({ page }) => {
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 0).click()
    await openPalette(page)
    await paletteInput(page).fill('orders')
    await page.keyboard.press('Enter')
    await expect(tabNamed(page, 'orders')).toBeVisible()
    await tabNamed(page, 'customers').click()
    await expect(inspector(page)).toBeVisible()
  })
})

test.describe('command palette focus', () => {
  test('⌘K from the data grid puts the cursor in the palette input', async ({ page }) => {
    // BUG: CommandPalette relies on the `autofocus` attribute, which Svelte only honours when <body> has focus.
    // With the grid focused the palette opens but the grid keeps the keyboard: typed text edits cells and
    // Escape (handled on the input only) cannot close the palette (CommandPalette.svelte).
    test.fail()
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 1).click()
    await page.keyboard.press(`${MOD}+k`)
    await expect(palette(page)).toBeVisible()
    await expect(paletteInput(page)).toBeFocused({ timeout: 1500 })
  })

  test('⌘K from the SQL editor puts the cursor in the palette input', async ({ page }) => {
    // BUG: same cause as above, with the CodeMirror editor holding the focus.
    test.fail()
    await openConnection(page)
    await newQueryTab(page)
    await page.keyboard.press(`${MOD}+k`)
    await expect(palette(page)).toBeVisible()
    await expect(paletteInput(page)).toBeFocused({ timeout: 1500 })
  })

  test('Escape closes the palette wherever the focus is', async ({ page }) => {
    // BUG: Escape is only handled by the palette's own <input onkeydown>, so with focus elsewhere
    // (grid, editor, a button) the palette stays open and its backdrop blocks the whole window.
    test.fail()
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 1).click()
    await page.keyboard.press(`${MOD}+k`)
    await expect(palette(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(palette(page)).toHaveCount(0, { timeout: 1500 })
  })
})

test.describe('palette on the connection list', () => {
  test('⌘K works before any connection is open, with only the global actions', async ({ page }) => {
    await gotoApp(page)
    await openPalette(page)
    const labels = (await options(page).locator('span').allTextContents()).map((t) => t.trim())
    expect(labels).toEqual(['Toggle appearance', 'Add database connection', 'Keyboard shortcuts', 'Settings'])
    await expect(palette(page).getByText('Tables', { exact: true })).toHaveCount(0)
  })

  test('the search button in the toolbar opens it', async ({ page }) => {
    await gotoApp(page)
    await page.getByRole('button', { name: 'Search' }).click()
    await expect(palette(page)).toBeVisible()
  })
})

test.describe('keyboard shortcuts help', () => {
  test('⌘/ opens a table of shortcuts and ⌘/ again (or Escape) closes it', async ({ page }) => {
    await openConnection(page)
    await page.keyboard.press(`${MOD}+/`)
    const help = dialog(page, 'Keyboard shortcuts')
    await expect(help).toBeVisible()
    const rows = help.getByRole('row')
    await expect(rows.filter({ hasText: 'New query tab' })).toContainText('T')
    await expect(rows.filter({ hasText: 'Close tab' })).toContainText('W')
    await expect(rows.filter({ hasText: 'Command palette' })).toContainText('K')
    await expect(rows.filter({ hasText: 'Toggle light / dark theme' })).toContainText('Shift+L')
    await expect(rows.filter({ hasText: 'Run statement under cursor' })).toContainText('Enter')
    await expect(rows.filter({ hasText: 'Run whole script' })).toContainText('Shift+Enter')
    await expect(rows.filter({ hasText: 'Next / previous tab' })).toContainText('Ctrl+Tab')

    await page.keyboard.press(`${MOD}+/`)
    await expect(help).toHaveCount(0)
    await page.keyboard.press(`${MOD}+/`)
    await expect(help).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(help).toHaveCount(0)
  })

  test('also reachable from the connection list', async ({ page }) => {
    await gotoApp(page)
    await page.keyboard.press(`${MOD}+/`)
    await expect(dialog(page, 'Keyboard shortcuts')).toBeVisible()
  })

  test('the help lists the shortcuts that exist in the table view and the connection list', async ({ page }) => {
    // BUG: ShortcutsHelp.svelte only lists 9 global shortcuts. Missing although implemented: ⌘S commit,
    // ⌘F filters, ⌘R refresh, ⌘D duplicate row, ⌘⇧N set NULL, ⌘A / ⌘C in the grid, Delete, ⌘N new connection,
    // Enter/F2 to edit and the sidebar arrow keys.
    test.fail()
    await openConnection(page)
    await page.keyboard.press(`${MOD}+/`)
    const help = dialog(page, 'Keyboard shortcuts')
    await expect(help).toContainText('Commit', { timeout: 1500 })
    await expect(help).toContainText('Filters', { timeout: 1500 })
    await expect(help).toContainText('Duplicate', { timeout: 1500 })
  })
})

test.describe('settings', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expect(dialog(page, 'Settings')).toBeVisible()
  })

  const settings = (page: Page) => dialog(page, 'Settings')

  test('shows appearance, history, assistant, shortcuts and updates sections', async ({ page }) => {
    for (const heading of ['Appearance', 'Query history', 'AI assistant', 'Keyboard shortcuts', 'Updates']) {
      await expect(settings(page).getByText(heading, { exact: true })).toBeVisible()
    }
  })

  test('appearance: System / Light / Dark switch the theme, and the choice persists', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Dark', exact: true }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
    await expect.poll(() => page.evaluate(() => localStorage.getItem('tablesoft.theme'))).toBe('dark')
    await settings(page).getByRole('button', { name: 'Light', exact: true }).click()
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
    await settings(page).getByRole('button', { name: 'System', exact: true }).click()
    await expect.poll(() => page.evaluate(() => localStorage.getItem('tablesoft.theme'))).toBe('system')
  })

  test('appearance: the active mode is marked', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Dark', exact: true }).click()
    await expect(settings(page).getByRole('button', { name: 'Dark', exact: true })).toHaveClass(/on/)
    await expect(settings(page).getByRole('button', { name: 'Light', exact: true })).not.toHaveClass(/on/)
  })

  test('appearance: the mode buttons expose their pressed state to assistive tech', async ({ page }) => {
    // BUG: the System/Light/Dark segmented buttons only change a CSS class (`.on`); there is no aria-pressed /
    // role=radio, so a screen reader cannot tell which one is selected (SettingsDialog.svelte `.seg`).
    test.fail()
    await settings(page).getByRole('button', { name: 'Dark', exact: true }).click()
    await expect(settings(page).getByRole('button', { name: 'Dark', exact: true })).toHaveAttribute('aria-pressed', 'true', { timeout: 1500 })
  })

  test('query history: shows the count and Clear history empties it', async ({ page }) => {
    await expect(settings(page).getByText('0 queries kept on this computer.')).toBeVisible()
    await expect(settings(page).getByRole('button', { name: 'Clear history' })).toBeDisabled()
    await page.keyboard.press('Escape')
    await newQueryTab(page)
    await setSql(page, 'SELECT * FROM customers')
    await page.keyboard.press(`${MOD}+Enter`)
    await expect(page.getByText('2 rows', { exact: true })).toBeVisible()
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expect(settings(page).getByText('1 queries kept on this computer.')).toBeVisible()
    await settings(page).getByRole('button', { name: 'Clear history' }).click()
    await expect(settings(page).getByText('0 queries kept on this computer.')).toBeVisible()
  })

  test('AI key: a saved key is reflected in the placeholder; Remove clears it and disables Remove', async ({ page }) => {
    const key = settings(page).getByLabel('Anthropic API key')
    await expect(key).toHaveAttribute('placeholder', 'Key saved in Keychain')
    await expect(settings(page).getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
    await expect(settings(page).getByRole('button', { name: 'Remove' })).toBeEnabled()

    await settings(page).getByRole('button', { name: 'Remove' }).click()
    await expect(key).toHaveAttribute('placeholder', 'sk-ant-…')
    await expect(settings(page).getByRole('button', { name: 'Remove' })).toBeDisabled()
  })

  test('AI key: typing a key enables Save; saving shows a toast and the key box is cleared', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Remove' }).click()
    const key = settings(page).getByLabel('Anthropic API key')
    await expect(key).toHaveAttribute('placeholder', 'sk-ant-…') // wait for the removal to finish
    await expect(key).toHaveAttribute('type', 'password')
    await key.fill('sk-ant-test-123')
    await expect(settings(page).getByRole('button', { name: 'Save', exact: true })).toBeEnabled()
    await settings(page).getByRole('button', { name: 'Save', exact: true }).click()
    await expect(page.getByText('API key saved in Keychain')).toBeVisible()
    await expect(key).toHaveValue('')
    await expect(key).toHaveAttribute('placeholder', 'Key saved in Keychain')
  })

  test('AI key: Enter in the key box saves, and a blank key cannot be saved', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Remove' }).click()
    const key = settings(page).getByLabel('Anthropic API key')
    await expect(key).toHaveAttribute('placeholder', 'sk-ant-…')
    await key.fill('   ')
    await expect(settings(page).getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
    await key.fill('sk-ant-enter')
    await key.press('Enter')
    await expect(page.getByText('API key saved in Keychain')).toBeVisible()
  })

  test('AI model: lists the models and remembers the choice across reloads', async ({ page }) => {
    const model = settings(page).getByRole('combobox')
    await expect(model.locator('option')).toHaveText(['Claude Sonnet 5.5', 'Claude Opus 5.5', 'Claude Haiku 5.5'])
    await expect(model).toHaveValue('claude-sonnet-5-5')
    await model.selectOption('claude-opus-5-5')
    await expect.poll(() => page.evaluate(() => localStorage.getItem('tablesoft.assistant.model'))).toBe('claude-opus-5-5')

    await page.reload()
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expect(settings(page).getByRole('combobox')).toHaveValue('claude-opus-5-5')
  })

  test('Show and About… open their dialogs and close Settings', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Show' }).click()
    await expect(settings(page)).toHaveCount(0)
    await expect(dialog(page, 'Keyboard shortcuts')).toBeVisible()
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await settings(page).getByRole('button', { name: 'About…' }).click()
    await expect(dialog(page, 'About TableSoft')).toBeVisible()
  })

  test('Done and Escape close the dialog', async ({ page }) => {
    await settings(page).getByRole('button', { name: 'Done' }).click()
    await expect(settings(page)).toHaveCount(0)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await page.keyboard.press('Escape')
    await expect(settings(page)).toHaveCount(0)
  })
})

test.describe('about', () => {
  test('shows the version and engines, and checks for updates', async ({ page }) => {
    await openConnection(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: 'About / Check for updates…' }).click()
    const about = dialog(page, 'About TableSoft')
    await expect(about).toContainText('v0.1.0')
    await expect(about).toContainText('Engines: sqlite, postgres, mysql, mariadb, mongodb')
    await about.getByRole('button', { name: 'Check for updates' }).click()
    await expect(about.getByText(/latest version|not configured/)).toBeVisible()
    await about.getByRole('button', { name: 'Close' }).click()
    await expect(about).toHaveCount(0)
  })
})

test.describe('settings from the connection list', () => {
  test('the toolbar menu offers Settings and shortcuts before connecting', async ({ page }) => {
    await gotoApp(page)
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expect(dialog(page, 'Settings')).toBeVisible()
  })
})
