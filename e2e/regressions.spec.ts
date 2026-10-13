import { test, expect } from './fixtures'
import {
  MOD,
  bottomBar,
  box,
  gotoApp,
  grid,
  gridCell,
  inspector,
  newQueryTab,
  openConnection,
  openTableLoaded,
  overlaps,
  setSql,
  sidebar,
  sqlEditor,
  sqlText,
  tabTitles,
} from './helpers'
import type { Locator, Page } from '@playwright/test'

// ---------------------------------------------------------------------------
// WebKit (the Tauri web view) autocapitalizes and autocorrects text fields unless told not to.
// main.ts switches it off on focusin for every field; these tests check the attributes and the typed text.
// ---------------------------------------------------------------------------

async function expectPlainTextField(field: Locator, opts: { password?: boolean } = {}) {
  await field.focus()
  await expect(field).toHaveAttribute('autocapitalize', 'off')
  await expect(field).toHaveAttribute('autocorrect', 'off')
  await expect(field).toHaveAttribute('spellcheck', 'false')
  if (!opts.password) await expect(field).toHaveAttribute('autocomplete', 'off')
}

test.describe('no autocapitalize / autocorrect / spellcheck', () => {
  test('SQL editor: attributes are off and typed text is left exactly as typed', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await sqlEditor(page).click()
    await expect(sqlEditor(page)).toHaveAttribute('autocapitalize', 'off')
    await expect(sqlEditor(page)).toHaveAttribute('autocorrect', 'off')
    await expect(sqlEditor(page)).toHaveAttribute('spellcheck', 'false')
    await page.keyboard.type('select * from customers where name = \'jOhN o\\\'Neil\' -- teh quick brwn fox')
    expect(await sqlText(page)).toBe("select * from customers where name = 'jOhN o\\'Neil' -- teh quick brwn fox")
  })

  test('SQL editor: a new tab opened later is covered too', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await newQueryTab(page)
    await sqlEditor(page).click()
    await expect(sqlEditor(page)).toHaveAttribute('autocapitalize', 'off')
    await page.keyboard.type('mAxx')
    expect(await sqlText(page)).toBe('mAxx')
  })

  test('connection form fields', async ({ page }) => {
    await gotoApp(page)
    await page.getByRole('button', { name: 'New connection' }).click()
    const form = page.getByRole('dialog', { name: 'New connection' })
    await expectPlainTextField(form.getByRole('textbox', { name: 'Name', exact: true }))
    await expectPlainTextField(form.getByRole('textbox', { name: 'Host', exact: true }))
    await expectPlainTextField(form.getByRole('textbox', { name: 'User', exact: true }))
    await expectPlainTextField(form.getByRole('textbox', { name: /^Database/ }))
    await expectPlainTextField(form.getByLabel('Password', { exact: true }), { password: true })
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('DB.Example.COM')
    await expect(form.getByRole('textbox', { name: 'Host', exact: true })).toHaveValue('DB.Example.COM')
  })

  test('search boxes: connections, sidebar, table quick filter, Details field search', async ({ page }) => {
    await gotoApp(page)
    await expectPlainTextField(page.getByPlaceholder('Search connections…'))
    await openConnection(page)
    await expectPlainTextField(sidebar(page).getByPlaceholder('Search for item…'))
    await openTableLoaded(page, 'customers')
    await expectPlainTextField(bottomBar(page).getByPlaceholder('Filter rows…'))
    await gridCell(page, 0, 0).click()
    await expectPlainTextField(inspector(page).getByPlaceholder('Search for field…'))
  })

  test('inline cell editor, Details inputs and the filter value box', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await gridCell(page, 0, 2).dblclick()
    const cellInput = page.locator('.edit-host input')
    await expectPlainTextField(cellInput)
    await cellInput.fill('McDonald')
    await page.keyboard.press('Enter')
    await expect(gridCell(page, 0, 2)).toHaveText('McDonald')
    await expectPlainTextField(inspector(page).locator('.field').filter({ hasText: 'name' }).locator('input.value'))

    await bottomBar(page).getByRole('button', { name: /Filters/ }).click()
    await expectPlainTextField(page.getByPlaceholder('value'))
  })

  test('command palette, save dialog, assistant composer and settings key box', async ({ page }) => {
    await openConnection(page)
    await page.keyboard.press(`${MOD}+k`)
    await expectPlainTextField(page.getByPlaceholder('What would you like to do?'))
    await page.keyboard.press('Escape')

    await newQueryTab(page)
    await setSql(page, 'SELECT 1')
    await page.getByRole('button', { name: 'Save' }).click()
    await expectPlainTextField(page.getByPlaceholder('Revenue by month'))
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expectPlainTextField(page.getByRole('textbox', { name: 'Message the assistant' }))
    await page.getByRole('textbox', { name: 'Message the assistant' }).fill('Hello World, teh test')
    await expect(page.getByRole('textbox', { name: 'Message the assistant' })).toHaveValue('Hello World, teh test')

    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    await expectPlainTextField(page.getByLabel('Anthropic API key'), { password: true })
  })

  test('typing mixed-case text into a plain field keeps its case', async ({ page }) => {
    await openConnection(page)
    const search = sidebar(page).getByPlaceholder('Search for item…')
    await search.focus()
    await page.keyboard.type('mIxEdCaSe')
    await expect(search).toHaveValue('mIxEdCaSe')
  })
})

// ---------------------------------------------------------------------------
// Clicking a "new query" button and then pressing Space must not open more tabs.
// ---------------------------------------------------------------------------

test.describe('Space after clicking a + / new-query button', () => {
  const targets: [string, (page: Page) => Locator][] = [
    ['the tab strip +', (page) => page.locator('.bar .add')],
    ['the sidebar +', (page) => sidebar(page).getByRole('button', { name: 'New query' })],
    ['the toolbar SQL button', (page) => page.getByRole('button', { name: 'New SQL query' })],
    ['the empty-workspace New query button', (page) => page.getByRole('button', { name: 'New query' }).last()],
  ]

  for (const [name, button] of targets) {
    test(`${name}: Space does not open another tab`, async ({ page }) => {
      await openConnection(page)
      await button(page).click()
      await expect(sqlEditor(page)).toBeVisible()
      await page.keyboard.press('Space')
      await page.keyboard.press('Space')
      expect(await tabTitles(page)).toEqual(['Query 1'])
    })
  }

  test('Enter after clicking the tab strip + does not open another tab either', async ({ page }) => {
    await openConnection(page)
    await page.locator('.bar .add').click()
    await expect(sqlEditor(page)).toBeVisible()
    await page.keyboard.press('Enter')
    expect(await tabTitles(page)).toEqual(['Query 1'])
  })

  test('after the click the new editor gets the keyboard, so Space types a space', async ({ page }) => {
    await openConnection(page)
    await page.locator('.bar .add').click()
    await expect(sqlEditor(page)).toBeVisible()
    await expect(sqlEditor(page)).toBeFocused()
    await page.keyboard.type('a b')
    expect(await sqlText(page)).toBe('a b')
  })

  test('the same holds for + Row: Space after clicking it does not add a second draft row', async ({ page }) => {
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await bottomBar(page).getByRole('button', { name: 'Row', exact: true }).click()
    await page.keyboard.press('Space')
    await expect(bottomBar(page).getByText('1 change', { exact: true })).toBeVisible()
  })

  test('the same holds for the toolbar toggles: Space after clicking the sidebar toggle does not flip it back', async ({ page }) => {
    // BUG: a clicked toolbar button keeps focus, so the very next Space press (for instance the first space of
    // a query typed straight away) re-activates it: the sidebar flickers closed and open (TopBar.svelte
    // buttons do not blur / prevent focus on mousedown like TabBar's + does).
    await openConnection(page)
    await page.getByRole('button', { name: 'Toggle sidebar' }).click()
    await expect.poll(async () => (await sidebar(page).boundingBox())?.width ?? 0).toBeLessThan(2)
    await page.keyboard.press('Space')
    await expect.poll(async () => (await sidebar(page).boundingBox())?.width ?? 0, { timeout: 1500 }).toBeLessThan(2)
  })
})

// ---------------------------------------------------------------------------
// Nothing may overlap, or be pushed out of, the table's bottom bar.
// ---------------------------------------------------------------------------

interface BarReport {
  bar: { x: number; y: number; width: number; height: number }
  beyond: string[]
  overlapping: string[]
  clipped: string[]
}

/** Geometry of every direct child of the visible table bottom bar. */
async function barReport(page: Page): Promise<BarReport> {
  return page.evaluate(() => {
    const bar = document.querySelector('.tab-body:not([hidden]) .bottombar')!
    const b = bar.getBoundingClientRect()
    const name = (el: Element) => {
      const aria = el.getAttribute('aria-label') || el.getAttribute('title')
      return (aria || el.className.toString().replace(/svelte-\w+/g, '').trim() || el.tagName) + ` “${(el.textContent ?? '').trim().slice(0, 20)}”`
    }
    const kids = [...bar.children].map((el) => ({ el, r: el.getBoundingClientRect() }))
    const beyond: string[] = []
    const overlapping: string[] = []
    const clipped: string[] = []
    for (const k of kids) {
      if (k.r.right > b.right + 1 || k.r.left < b.left - 1) beyond.push(name(k.el))
      if (k.el.scrollWidth > k.el.clientWidth + 1) clipped.push(name(k.el))
    }
    for (let i = 0; i < kids.length; i++) {
      for (let j = i + 1; j < kids.length; j++) {
        const a = kids[i].r
        const c = kids[j].r
        if (a.left < c.right - 0.5 && c.left < a.right - 0.5) overlapping.push(`${name(kids[i].el)} / ${name(kids[j].el)}`)
      }
    }
    return { bar: { x: b.x, y: b.y, width: b.width, height: b.height }, beyond, overlapping, clipped }
  })
}

async function editFirstName(page: Page, value: string) {
  await gridCell(page, 0, 2).dblclick()
  await page.locator('.edit-host input').fill(value)
  await page.keyboard.press('Enter')
}

for (const [width, height] of [
  [1280, 720],
  [1440, 900],
] as const) {
  test.describe(`bottom bar at ${width}x${height}`, () => {
    test.use({ viewport: { width, height } })

    test('table view: every control fits inside the bar, none overlaps another, and the bar sits at the window bottom', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      const r = await barReport(page)
      expect(r.beyond).toEqual([])
      expect(r.overlapping).toEqual([])
      expect(r.clipped).toEqual([])
      expect(Math.abs(r.bar.y + r.bar.height - height)).toBeLessThanOrEqual(1)
    })

    test('with the Details panel open the controls still fit and do not overlap', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      await gridCell(page, 0, 0).click()
      await expect(inspector(page)).toBeVisible()
      await expect.poll(() => grid(page).evaluate((el) => el.clientWidth)).toBeLessThan(width - 400)
      const r = await barReport(page)
      expect(r.beyond).toEqual([])
      expect(r.overlapping).toEqual([])
    })

    test('with the Details panel open the row count and timing are not cut off', async ({ page }) => {
      // BUG: .info (flex: 1 1 0; min-width: 60px) is squeezed to ~60px when the panel is open, so the text
      // "≈1,000,000 rows  3 ms" is silently clipped (TableView.svelte .bottombar .info).
      test.fail()
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      await gridCell(page, 0, 0).click()
      await expect(inspector(page)).toBeVisible()
      await expect.poll(async () => (await barReport(page)).clipped, { timeout: 2000 }).toEqual([])
    })

    test('with pending changes (Preview / Discard / Commit) every control still fits the bar', async ({ page }) => {
      // BUG: editing a cell opens the Details panel and adds the "N change / Preview / Discard / Commit" group;
      // together they push Columns, Refresh, Export and Import beyond the right edge of the bar, where they are
      // clipped (TableView.svelte .bottombar has no overflow handling / responsive collapse).
      test.fail()
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      await editFirstName(page, 'zz')
      await expect(bottomBar(page).getByText('1 change', { exact: true })).toBeVisible()
      await expect(inspector(page)).toBeVisible()
      const r = await barReport(page)
      expect(r.beyond).toEqual([])
      expect(r.overlapping).toEqual([])
    })

    test('the data grid ends above the bottom bar (nothing paints over it)', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      const bar = await box(bottomBar(page))
      const g = await box(page.getByRole('grid'))
      expect(g.y + g.height).toBeLessThanOrEqual(bar.y + 1)
    })

    test('the query log pane sits below the bar without covering it', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      await page.getByRole('button', { name: 'Toggle query log' }).click()
      const bar = await box(bottomBar(page))
      const pane = await box(page.locator('.console'))
      expect(overlaps(bar, pane)).toBe(false)
      expect(pane.y).toBeGreaterThanOrEqual(bar.y + bar.height - 1)
    })

    test('the Columns popover opens above the bar and stays inside the window', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'big_table')
      await bottomBar(page).getByRole('button', { name: /Columns/ }).click()
      const pop = await box(page.getByRole('dialog', { name: 'Columns' }))
      const bar = await box(bottomBar(page))
      expect(pop.y + pop.height).toBeLessThanOrEqual(bar.y + 4) // anchored just above the Columns button, inside the bar's top edge
      expect(pop.x).toBeGreaterThanOrEqual(0)
      expect(pop.x + pop.width).toBeLessThanOrEqual(width + 1)
      expect(pop.y).toBeGreaterThanOrEqual(0)
    })

    test('the sidebar footer (+ and the database / schema pickers) fits the sidebar', async ({ page }) => {
      await openConnection(page)
      const side = await box(sidebar(page))
      const plus = await box(sidebar(page).getByRole('button', { name: 'New query' }))
      const db = await box(sidebar(page).getByRole('combobox', { name: 'Database' }))
      const schema = await box(sidebar(page).getByRole('combobox', { name: 'Schema' }))
      for (const b of [plus, db, schema]) {
        expect(b.x).toBeGreaterThanOrEqual(side.x - 1)
        expect(b.x + b.width).toBeLessThanOrEqual(side.x + side.width + 1)
        expect(b.y + b.height).toBeLessThanOrEqual(height + 1)
      }
      expect(overlaps(plus, db)).toBe(false)
      expect(overlaps(db, schema)).toBe(false)
    })

    test('the toolbar controls and the status pill never overlap', async ({ page }) => {
      await openConnection(page)
      await openTableLoaded(page, 'customers')
      const kids = await page.locator('header.toolbar > *').evaluateAll((els) =>
        els.map((el) => {
          const r = el.getBoundingClientRect()
          return { x: r.x, y: r.y, width: r.width, height: r.height }
        }),
      )
      for (let i = 0; i < kids.length; i++) {
        expect(kids[i].x + kids[i].width).toBeLessThanOrEqual(width + 1)
        for (let j = i + 1; j < kids.length; j++) expect(overlaps(kids[i], kids[j])).toBe(false)
      }
    })

    test('the SQL editor toolbar fits: status text and the Run buttons do not overlap', async ({ page }) => {
      await openConnection(page)
      await newQueryTab(page)
      await setSql(page, 'SELECT * FROM customers')
      await page.keyboard.press(`${MOD}+Enter`)
      await expect(page.locator('.tab-body:not([hidden]) .toolbar .status')).toHaveText(/finished/)
      const kids = await page.locator('.tab-body:not([hidden]) .query .toolbar > *').evaluateAll((els) =>
        els
          .filter((el) => el.getBoundingClientRect().width > 0)
          .map((el) => {
            const r = el.getBoundingClientRect()
            return { x: r.x, y: r.y, width: r.width, height: r.height }
          }),
      )
      const bar = await box(page.locator('.tab-body:not([hidden]) .query .toolbar'))
      for (let i = 0; i < kids.length; i++) {
        expect(kids[i].x + kids[i].width).toBeLessThanOrEqual(bar.x + bar.width + 1)
        for (let j = i + 1; j < kids.length; j++) expect(overlaps(kids[i], kids[j])).toBe(false)
      }
    })
  })
}

test.describe('toasts and the bottom bar', () => {
  test('a toast does not cover the table bottom bar', async ({ page }) => {
    // BUG: .toasts is positioned at bottom: 34px, but the table bottom bar is 38px tall, so every toast
    // overlaps the bar's right-hand buttons by about 4px (Toasts.svelte vs TableView.svelte .bottombar).
    await page.setViewportSize({ width: 1280, height: 720 })
    await openConnection(page)
    await openTableLoaded(page, 'customers')
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await dlg.getByRole('button', { name: 'Remove' }).click()
    await expect(dlg.getByLabel('Anthropic API key')).toHaveAttribute('placeholder', 'sk-ant-…')
    await dlg.getByLabel('Anthropic API key').fill('sk-ant-x')
    await dlg.getByRole('button', { name: 'Save', exact: true }).click()
    const toast = page.locator('.toast', { hasText: 'API key saved in Keychain' })
    await expect(toast).toBeVisible()
    const t = await box(toast)
    const bar = await box(bottomBar(page))
    expect(overlaps(t, bar)).toBe(false)
  })
})
