import { expect, type Locator, type Page } from '@playwright/test'

/** The shortcut modifier the app listens for: ⌘ on macOS, Ctrl elsewhere (matches navigator.platform). */
export const MOD = process.platform === 'darwin' ? 'Meta' : 'Control'

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

/** Loads the app (the dev mock backend answers every command) and waits for the connection list. */
export async function gotoApp(page: Page) {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
  await expect(page.getByRole('option').first()).toBeVisible()
}

/** A connection card on the welcome screen, found by its name. */
export function card(page: Page, name: string): Locator {
  return page.getByRole('option', { name: new RegExp(`^${escapeRe(name)}\\b`) })
}

/** Opens a saved connection from the welcome screen and waits for the workspace and its schema. */
export async function openConnection(page: Page, name = 'Local Postgres') {
  await gotoApp(page)
  await card(page, name).getByRole('button', { name: 'Open', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Toggle sidebar' })).toBeVisible()
  await expect(page.getByRole('treeitem').first()).toBeVisible()
}

/** Goes back to the welcome screen from the workspace (the "Connections" toolbar button). */
export async function showConnections(page: Page) {
  await page.getByRole('button', { name: 'Connections', exact: true }).click()
  await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
}

// ---------------------------------------------------------------------------
// Workspace pieces
// ---------------------------------------------------------------------------

export const sidebar = (page: Page) =>
  page.getByRole('complementary').filter({ has: page.getByRole('tab', { name: 'Items' }) })

/** The right-hand panel (Details / Assistant). */
export const inspector = (page: Page) =>
  page.getByRole('complementary').filter({ has: page.getByRole('tab', { name: 'Details' }) })

export const tableRow = (page: Page, name: string) => page.getByRole('treeitem', { name, exact: true })

/** A tab in the strip. Its accessible name also contains the close button's label, so match by title text. */
export const tabNamed = (page: Page, title: string | RegExp) =>
  page.getByRole('tab', { name: title instanceof RegExp ? title : new RegExp(`${escapeRe(title)}$`) })

/** The titles of every tab in the strip, in order. */
export async function tabTitles(page: Page): Promise<string[]> {
  return page.locator('[role=tab] .t').allTextContents()
}

/** Opens a table as a regular (non-preview) tab: double-click in the sidebar. */
export async function openTable(page: Page, name: string) {
  await tableRow(page, name).dblclick()
  await expect(tabNamed(page, name)).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Data', exact: true })).toBeVisible()
}

/** Opens a table and waits until its first rows are painted. */
export async function openTableLoaded(page: Page, name: string) {
  await openTable(page, name)
  await expect(grid(page).locator('.cell:not(.ph)').first()).toBeVisible()
}

/** The bottom bar of a table tab (Data/Structure/Relations, + Row, count, filters). */
export const bottomBar = (page: Page) => page.locator('.tab-body:not([hidden]) .bottombar')

// ---------------------------------------------------------------------------
// Data grid
// ---------------------------------------------------------------------------

export const grid = (page: Page) => page.getByRole('grid')
export const gridRows = (page: Page) => grid(page).locator('.row')
export const gridCell = (page: Page, row: number, col: number) => gridRows(page).nth(row).locator('.cell').nth(col)
export const gridHeader = (page: Page, name: string | RegExp) =>
  grid(page)
    .locator('.h')
    .filter({ has: page.locator('.hname').getByText(name, { exact: typeof name === 'string' }) })

/** Text of every painted cell of one row. */
export async function rowTexts(page: Page, row: number): Promise<string[]> {
  return (await gridRows(page).nth(row).locator('.cell').allTextContents()).map((t) => t.trim())
}

/** Text of one column across the first `n` painted rows. */
export async function columnTexts(page: Page, col: number, n = 5): Promise<string[]> {
  const rows = gridRows(page)
  const count = Math.min(await rows.count(), n)
  const out: string[] = []
  for (let i = 0; i < count; i++) out.push(((await rows.nth(i).locator('.cell').nth(col).textContent()) ?? '').trim())
  return out
}

/** The field box for a column in the Details panel (label + type + value). */
export function detailField(page: Page, column: string): Locator {
  return inspector(page)
    .locator('.field')
    .filter({ has: page.locator('.lname').getByText(column, { exact: true }) })
}

/** Current value of a Details field, whether it renders as text or as an editable input. */
export async function detailValue(page: Page, column: string): Promise<string> {
  const field = detailField(page, column)
  const input = field.locator('input.value, textarea.value')
  if ((await input.count()) > 0) return input.first().inputValue()
  return ((await field.locator('.value').first().textContent()) ?? '').trim()
}

// ---------------------------------------------------------------------------
// SQL editor
// ---------------------------------------------------------------------------

/** The CodeMirror content element of the visible query tab. */
export const sqlEditor = (page: Page) => page.locator('.tab-body:not([hidden]) .cm-content')

export async function newQueryTab(page: Page) {
  await page.keyboard.press(`${MOD}+t`)
  await expect(sqlEditor(page)).toBeVisible()
}

/** Replaces the whole editor text (fast, does not trigger autocomplete). */
export async function setSql(page: Page, sql: string) {
  const ed = sqlEditor(page)
  await ed.click()
  await page.keyboard.press(`${MOD}+a`)
  await page.keyboard.insertText(sql)
}

export async function sqlText(page: Page): Promise<string> {
  // innerText would include CodeMirror's placeholder widget when the editor is empty.
  return sqlEditor(page).evaluate((el) =>
    [...el.querySelectorAll('.cm-line')]
      .map((line) =>
        [...line.childNodes]
          .filter((n) => !(n instanceof HTMLElement && n.classList.contains('cm-placeholder')))
          .map((n) => n.textContent ?? '')
          .join(''),
      )
      .join('\n')
      .replace(/\u00a0/g, ' '),
  )
}

export async function runStatement(page: Page) {
  await page.keyboard.press(`${MOD}+Enter`)
}

// ---------------------------------------------------------------------------
// Mock backend controls
// ---------------------------------------------------------------------------

/** What the native file dialogs return (null = cancelled). Read by the mock at call time. */
export async function setDialog(page: Page, dialog: { open?: string | null; save?: string | null }) {
  await page.evaluate((d) => {
    ;(window as unknown as { __TABLESOFT_MOCK_DIALOG__: unknown }).__TABLESOFT_MOCK_DIALOG__ = d
  }, dialog)
}

interface Call {
  cmd: string
  args: Record<string, unknown>
}

/** Starts recording every backend command (name + arguments) so a test can assert what the UI asked for. */
export async function recordInvokes(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as { __calls?: Call[]; __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => unknown } }
    if (w.__calls) return
    const calls: Call[] = (w.__calls = [])
    const t = w.__TAURI_INTERNALS__
    const orig = t.invoke
    t.invoke = (cmd: string, args?: unknown) => {
      calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? {})) })
      return orig(cmd, args)
    }
  })
}

export async function invokes(page: Page, cmd: string): Promise<Call[]> {
  return page.evaluate((c) => ((window as unknown as { __calls?: Call[] }).__calls ?? []).filter((x) => x.cmd === c), cmd)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

export interface Box {
  x: number
  y: number
  width: number
  height: number
}

export async function box(loc: Locator): Promise<Box> {
  const b = await loc.boundingBox()
  if (!b) throw new Error('element has no bounding box (not rendered?)')
  return b
}

export function overlaps(a: Box, b: Box, tolerance = 0.5): boolean {
  return (
    a.x < b.x + b.width - tolerance &&
    b.x < a.x + a.width - tolerance &&
    a.y < b.y + b.height - tolerance &&
    b.y < a.y + a.height - tolerance
  )
}
