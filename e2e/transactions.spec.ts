import { test, expect } from './fixtures'
import { MOD, card, invokes, newQueryTab, openConnection, recordInvokes, runStatement, setSql, showConnections, tabNamed, tabTitles } from './helpers'
import type { Page } from '@playwright/test'

// Manual transaction mode in the query tab. The mock backend logs BEGIN / COMMIT / ROLLBACK into the query log.

const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })
const mode = (page: Page) => page.getByRole('group', { name: 'Transaction mode' })
const manualButton = (page: Page) => mode(page).getByRole('button', { name: 'Manual' })
const autoButton = (page: Page) => mode(page).getByRole('button', { name: 'Auto-commit' })
const pill = (page: Page) => page.locator('.tab-body:not([hidden]) .txpill')
const commit = (page: Page) => page.locator('.tab-body:not([hidden]) .toolbar').getByRole('button', { name: 'Commit', exact: true })
const rollback = (page: Page) => page.locator('.tab-body:not([hidden]) .toolbar').getByRole('button', { name: 'Rollback', exact: true })
const logLine = (page: Page, text: string | RegExp) => page.locator('.console .sql', { hasText: text })
const status = (page: Page) => page.locator('.tab-body:not([hidden]) .toolbar .status')
const UPDATE = "UPDATE customers SET name = 'x' WHERE id = 1"

/** A fresh query tab in Manual mode with the query log open. */
async function manualTab(page: Page, connection = 'Local Postgres') {
  await openConnection(page, connection)
  await page.getByRole('button', { name: 'Toggle query log' }).click()
  await newQueryTab(page)
  await manualButton(page).click()
  await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'true')
}

/** Runs a write in the tab and waits until the open transaction counts `n` statements (the status line may still show the previous run). */
async function runUpdate(page: Page, sql = UPDATE, n = 1) {
  await setSql(page, sql)
  await runStatement(page)
  await expect(pill(page)).toContainText(`· ${n} statement`)
  await expect(status(page)).toHaveText(/finished/)
}

test.describe('the mode switch', () => {
  test('a query tab starts in Auto-commit and never opens a transaction', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
    await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'false')
    await recordInvokes(page)
    await setSql(page, UPDATE)
    await runStatement(page)
    await expect(status(page)).toHaveText(/finished/)
    expect(await invokes(page, 'tx_begin')).toHaveLength(0)
    expect((await invokes(page, 'query_run'))[0].args).toMatchObject({ txId: null })
    await expect(pill(page)).toHaveCount(0)
  })

  test('is offered for every SQL engine and hidden for MongoDB', async ({ page }) => {
    for (const name of ['Local MySQL', 'nasty.db']) {
      await openConnection(page, name)
      await newQueryTab(page)
      await expect(mode(page)).toBeVisible()
    }
    await openConnection(page, 'Mongo')
    await newQueryTab(page)
    await expect(mode(page)).toHaveCount(0)
  })

  test('the choice is remembered per tab, and across a restart', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await manualButton(page).click()
    await newQueryTab(page)
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
    await page.waitForTimeout(600) // the open tabs are saved shortly after a change
    await page.reload()
    await openConnection(page)
    await tabNamed(page, 'Query 1').click()
    await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'true')
    await tabNamed(page, 'Query 2').click()
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
    await expect(pill(page)).toHaveCount(0) // a transaction is never restored
  })
})

test.describe('manual transactions', () => {
  test('switching to Manual alone begins nothing; the first run begins lazily and every run uses the transaction', async ({ page }) => {
    await manualTab(page)
    await recordInvokes(page)
    await expect(pill(page)).toHaveCount(0)
    expect(await invokes(page, 'tx_begin')).toHaveLength(0)

    await runUpdate(page)
    await expect(pill(page)).toContainText('Transaction open · 1 statement')
    await expect(logLine(page, 'BEGIN')).toHaveCount(1)
    const [begin] = await invokes(page, 'tx_begin')
    expect(begin.args).toMatchObject({ database: 'tablesail' })
    const [first] = await invokes(page, 'query_run')
    expect(first.args.txId).toMatch(/^tx/)

    await runUpdate(page, "UPDATE customers SET name = 'y' WHERE id = 2", 2)
    await expect(pill(page)).toContainText('Transaction open · 2 statements')
    expect(await invokes(page, 'tx_begin')).toHaveLength(1)
    const runs = await invokes(page, 'query_run')
    expect(runs[1].args.txId).toBe(first.args.txId)
    await expect(logLine(page, 'BEGIN')).toHaveCount(1)
  })

  test('Commit ends it: COMMIT is logged, the indicator goes and the next run begins a new transaction', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await commit(page).click()
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)
    await expect(pill(page)).toHaveCount(0)
    await expect(commit(page)).toHaveCount(0)
    await expect(page.locator('.toasts')).toContainText('Transaction committed')
    await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'true')

    await runUpdate(page)
    await expect(logLine(page, 'BEGIN')).toHaveCount(2)
    await expect(pill(page)).toContainText('1 statement')
  })

  test('Rollback ends it with ROLLBACK', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await rollback(page).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    await expect(logLine(page, 'COMMIT')).toHaveCount(0)
    await expect(pill(page)).toHaveCount(0)
    await expect(page.locator('.toasts')).toContainText('Transaction rolled back')
  })

  test('⌘⇧C commits and ⌘⇧R rolls back', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await page.keyboard.press(`${MOD}+Shift+c`)
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)
    await expect(pill(page)).toHaveCount(0)

    await runUpdate(page)
    await page.keyboard.press(`${MOD}+Shift+r`)
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    await expect(pill(page)).toHaveCount(0)
  })

  test('the palette offers Commit / Roll back only while a transaction is open', async ({ page }) => {
    await manualTab(page)
    await page.keyboard.press(`${MOD}+k`)
    await expect(page.getByRole('option', { name: /Commit transaction/ })).toHaveCount(0)
    await page.keyboard.press('Escape')

    await runUpdate(page)
    await page.keyboard.press(`${MOD}+k`)
    await expect(page.getByRole('option', { name: /Roll back transaction/ })).toBeVisible()
    await page.getByRole('option', { name: /Commit transaction/ }).click()
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)

    await runUpdate(page)
    await page.keyboard.press(`${MOD}+k`)
    await page.getByRole('combobox', { name: 'Search commands' }).fill('roll back')
    await page.getByRole('option', { name: /Roll back transaction/ }).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
  })

  test('the palette can switch the mode', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await page.keyboard.press(`${MOD}+k`)
    await page.getByRole('combobox', { name: 'Search commands' }).fill('manual')
    await page.getByRole('option', { name: /Switch to manual transactions/ }).click()
    await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press(`${MOD}+k`)
    await page.getByRole('combobox', { name: 'Search commands' }).fill('auto')
    await page.getByRole('option', { name: /Switch to auto-commit/ }).click()
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
  })

  test('a failing statement leaves the transaction open and says the last run failed', async ({ page }) => {
    await manualTab(page)
    await setSql(page, 'SELECT * FROM nonexistent_table')
    await runStatement(page)
    await expect(status(page)).toHaveText(/failed/)
    await expect(pill(page)).toContainText('Transaction open')
    await expect(pill(page)).toContainText('last run failed')
    await runUpdate(page)
    await expect(pill(page)).not.toContainText('last run failed')
  })

  test('the database cannot be switched while a transaction is open', async ({ page }) => {
    await manualTab(page)
    const picker = page.locator('.tab-body:not([hidden]) .toolbar').getByLabel('Database')
    await expect(picker).toBeEnabled()
    await runUpdate(page)
    await expect(picker).toBeDisabled()
    await commit(page).click()
    await expect(picker).toBeEnabled()
  })

  test('each tab has its own transaction', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await newQueryTab(page)
    await expect(pill(page)).toHaveCount(0)
    await manualButton(page).click()
    await runUpdate(page)
    await expect(logLine(page, 'BEGIN')).toHaveCount(2)
    await rollback(page).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    await tabNamed(page, 'Query 1').click()
    await expect(pill(page)).toContainText('Transaction open')
  })

  test('a tab with an open transaction is marked in the tab strip', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await expect(page.locator('.strip .pending.tx')).toHaveCount(1)
    await expect(tabNamed(page, 'Query 1')).toHaveAttribute('title', 'Query 1 (transaction open)')
    await commit(page).click()
    await expect(page.locator('.strip .pending')).toHaveCount(0)
  })

  test('if the transaction is gone, commit clears the state and says so', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await page.evaluate(() => {
      const t = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__
      const orig = t.invoke
      t.invoke = (cmd, args) =>
        cmd === 'tx_commit' ? Promise.reject({ kind: 'invalidInput', message: 'That transaction is no longer open.', code: null, statement: null }) : orig(cmd, args)
    })
    await commit(page).click()
    await expect(pill(page)).toHaveCount(0)
    await expect(page.locator('.toasts')).toContainText('no longer open')
    await expect(page.locator('.toasts')).toContainText('not saved')
    await expect(logLine(page, 'COMMIT')).toHaveCount(0)
  })

  test('a commit that fails for another reason keeps the transaction so it can be rolled back', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await page.evaluate(() => {
      const t = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__
      const orig = t.invoke
      t.invoke = (cmd, args) =>
        cmd === 'tx_commit' ? Promise.reject({ kind: 'database', message: 'deferred constraint violated', code: null, statement: null }) : orig(cmd, args)
    })
    await commit(page).click()
    await expect(page.locator('.toasts')).toContainText('deferred constraint violated')
    await expect(pill(page)).toContainText('Transaction open')
    await rollback(page).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
  })

  test('if the transaction cannot be started the run is not sent and the error is shown', async ({ page }) => {
    await openConnection(page)
    await newQueryTab(page)
    await manualButton(page).click()
    await page.evaluate(() => {
      const t = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__
      const orig = t.invoke
      t.invoke = (cmd, args) =>
        cmd === 'tx_begin' ? Promise.reject({ kind: 'connection', message: 'too many connections', code: null, statement: null }) : orig(cmd, args)
    })
    await recordInvokes(page)
    await setSql(page, UPDATE)
    await runStatement(page)
    await expect(status(page)).toContainText('Could not start a transaction: too many connections')
    expect(await invokes(page, 'query_run')).toHaveLength(0)
    await expect(pill(page)).toHaveCount(0)
  })
})

test.describe('leaving Manual mode with a transaction open', () => {
  test.beforeEach(async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
  })

  test('asks; Cancel keeps Manual and the transaction', async ({ page }) => {
    await autoButton(page).click()
    const dlg = dialog(page, 'Transaction still open')
    await expect(dlg).toContainText('1 statement')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    await expect(manualButton(page)).toHaveAttribute('aria-pressed', 'true')
    await expect(pill(page)).toBeVisible()
  })

  test('Roll back ends the transaction and switches to Auto-commit', async ({ page }) => {
    await autoButton(page).click()
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Roll back' }).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
    await expect(pill(page)).toHaveCount(0)
  })

  test('Commit ends the transaction and switches to Auto-commit', async ({ page }) => {
    await autoButton(page).click()
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Commit' }).click()
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)
    await expect(autoButton(page)).toHaveAttribute('aria-pressed', 'true')
    await expect(pill(page)).toHaveCount(0)
  })
})

test.describe('closing a tab with a transaction open', () => {
  test.beforeEach(async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
  })

  test('⌘W asks; Cancel keeps the tab and the transaction', async ({ page }) => {
    await page.keyboard.press(`${MOD}+w`)
    const dlg = dialog(page, 'Transaction still open')
    await expect(dlg).toContainText('close this tab')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    expect(await tabTitles(page)).toEqual(['Query 1'])
    await expect(pill(page)).toBeVisible()
    await expect(logLine(page, /^(COMMIT|ROLLBACK)/)).toHaveCount(0)
  })

  test('Commit commits, then closes the tab', async ({ page }) => {
    await page.getByRole('button', { name: 'Close tab Query 1' }).click()
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Commit' }).click()
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)
    await expect(tabNamed(page, 'Query 1')).toHaveCount(0)
  })

  test('Roll back rolls back, then closes the tab', async ({ page }) => {
    await page.keyboard.press(`${MOD}+w`)
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Roll back' }).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    await expect(tabNamed(page, 'Query 1')).toHaveCount(0)
  })

  test('a file tab with both an open transaction and unsaved edits asks about the transaction first, then the file', async ({ page }) => {
    // Give the tab a file by saving it, then edit it again.
    await page.evaluate(() => {
      ;(window as unknown as { __TABLESAIL_MOCK_DIALOG__: unknown }).__TABLESAIL_MOCK_DIALOG__ = { save: '/q/both.sql' }
    })
    await page.keyboard.press(`${MOD}+Shift+s`)
    await expect(tabNamed(page, 'both.sql')).toBeVisible()
    await setSql(page, UPDATE + ' -- edited')
    await expect(page.locator('.strip .pending')).toHaveCount(1)
    await page.keyboard.press(`${MOD}+w`)
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Roll back' }).click()
    await expect(logLine(page, 'ROLLBACK')).toHaveCount(1)
    const unsaved = dialog(page, 'Unsaved changes')
    await expect(unsaved).toBeVisible()
    await unsaved.getByRole('button', { name: 'Don’t save' }).click()
    await expect(tabNamed(page, 'both.sql')).toHaveCount(0)
  })
})

test.describe('disconnecting', () => {
  const disconnectMenu = async (page: Page) => {
    await page.getByRole('button', { name: /^PostgreSQL 17\.6 : Local Postgres/ }).click()
    await page.getByRole('menuitem', { name: 'Disconnect' }).click()
  }

  test('without a transaction it disconnects at once', async ({ page }) => {
    await manualTab(page)
    await disconnectMenu(page)
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    await expect(dialog(page, 'Transaction still open')).toHaveCount(0)
  })

  test('with a transaction open it asks first; Cancel stays connected with the transaction intact', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await disconnectMenu(page)
    const dlg = dialog(page, 'Transaction still open')
    await expect(dlg).toContainText('disconnect')
    await dlg.getByRole('button', { name: 'Cancel' }).click()
    await expect(dlg).toHaveCount(0)
    await expect(pill(page)).toBeVisible()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toHaveCount(0)
  })

  test('Roll back rolls the transaction back, then disconnects', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await recordInvokes(page)
    await disconnectMenu(page)
    await dialog(page, 'Transaction still open').getByRole('button', { name: 'Roll back' }).click()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    // the query log goes away with the workspace, so look at what was sent
    expect(await invokes(page, 'tx_rollback')).toHaveLength(1)
    expect(await invokes(page, 'connection_close')).toHaveLength(1)
  })

  test('the Disconnect in a connection card menu asks too', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await recordInvokes(page)
    await showConnections(page)
    await card(page, 'Local Postgres').getByRole('button', { name: 'Actions' }).click()
    await page.getByRole('menuitem', { name: 'Disconnect' }).click()
    const dlg = dialog(page, 'Transaction still open')
    await expect(dlg).toBeVisible()
    expect(await invokes(page, 'connection_close')).toHaveLength(0)
    await dlg.getByRole('button', { name: 'Roll back' }).click()
    await expect(card(page, 'Local Postgres')).not.toContainText('Connected')
    expect(await invokes(page, 'tx_rollback')).toHaveLength(1)
    expect(await invokes(page, 'connection_close')).toHaveLength(1)
  })

  test('asks about each tab with a transaction, one after the other', async ({ page }) => {
    await manualTab(page)
    await runUpdate(page)
    await newQueryTab(page)
    await manualButton(page).click()
    await runUpdate(page)
    await recordInvokes(page)
    await disconnectMenu(page)
    const dlg = dialog(page, 'Transaction still open')
    await dlg.getByRole('button', { name: 'Commit' }).click()
    await expect(logLine(page, 'COMMIT')).toHaveCount(1)
    await expect(dlg).toBeVisible()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toHaveCount(0)
    expect(await invokes(page, 'connection_close')).toHaveLength(0)
    await dlg.getByRole('button', { name: 'Roll back' }).click()
    await expect(page.getByRole('heading', { name: /^Connections/ })).toBeVisible()
    expect(await invokes(page, 'tx_commit')).toHaveLength(1)
    expect(await invokes(page, 'tx_rollback')).toHaveLength(1)
    expect(await invokes(page, 'connection_close')).toHaveLength(1)
  })
})
