import { test, expect } from './fixtures'
import { MOD, card, gotoApp, setDialog, showConnections } from './helpers'
import type { Page } from '@playwright/test'

const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })

async function openForm(page: Page) {
  await page.getByRole('button', { name: 'New connection' }).click()
  await expect(dialog(page, 'New connection')).toBeVisible()
  return dialog(page, 'New connection')
}

async function pickEngine(form: ReturnType<typeof dialog>, label: string) {
  await form.getByRole('radio', { name: label }).click()
}

async function cardAction(page: Page, name: string, item: string) {
  await card(page, name).getByRole('button', { name: 'Actions' }).click()
  await page.getByRole('menuitem', { name: item }).click()
}

test.describe('connection list', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('lists every saved connection with its engine, address and environment', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Connections 7' })).toBeVisible()
    await expect(page.getByRole('option')).toHaveCount(7)

    const local = card(page, 'Local Postgres')
    await expect(local).toContainText('PostgreSQL')
    await expect(local).toContainText('tablesail@127.0.0.1:25432/tablesail')
    await expect(local).toContainText('local')

    await expect(card(page, 'MariaDB')).toContainText('dev')
    await expect(card(page, 'Mongo')).toContainText('127.0.0.1:27117/tablesail')
    await expect(card(page, 'nasty.db')).toContainText('~/data/nasty.db')
    await expect(card(page, 'Staging API')).toContainText('staging')
  })

  test('groups by environment with production first and flags read-only connections', async ({ page }) => {
    const names = await page.getByRole('option').evaluateAll((els) => els.map((e) => e.querySelector('.name')?.textContent))
    expect(names[0]).toBe('Production')
    expect(names[1]).toBe('Staging API')
    expect(names.at(-1)).toBe('nasty.db')
    await expect(card(page, 'Production')).toContainText('read-only')
    await expect(card(page, 'Local Postgres')).not.toContainText('read-only')
  })

  test('search filters by name, host, engine and database, and says when nothing matches', async ({ page }) => {
    const search = page.getByPlaceholder('Search connections…')
    await expect(search).toBeFocused()

    await search.fill('maria')
    await expect(page.getByRole('option')).toHaveCount(1)
    await expect(card(page, 'MariaDB')).toBeVisible()

    await search.fill('staging.internal') // matches a host
    await expect(page.getByRole('option')).toHaveCount(1)
    await expect(card(page, 'Staging API')).toBeVisible()

    await search.fill('postgresql') // matches an engine label
    await expect(page.getByRole('option')).toHaveCount(3)

    await search.fill('zzz-nothing')
    await expect(page.getByRole('option')).toHaveCount(0)
    await expect(page.getByText('No connections match “zzz-nothing”.')).toBeVisible()

    await search.fill('')
    await expect(page.getByRole('option')).toHaveCount(7)
  })

  test('arrow keys move the selection and Enter opens the selected connection', async ({ page }) => {
    await expect(card(page, 'Production')).toHaveAttribute('aria-selected', 'true')
    await page.keyboard.press('ArrowDown')
    await expect(card(page, 'Staging API')).toHaveAttribute('aria-selected', 'true')
    await expect(card(page, 'Production')).toHaveAttribute('aria-selected', 'false')
    await page.keyboard.press('ArrowUp')
    await expect(card(page, 'Production')).toHaveAttribute('aria-selected', 'true')

    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('button', { name: /PostgreSQL 17\.6 : Staging API/ })).toBeVisible()
  })

  test('double-click opens a connection and the card then shows Connected', async ({ page }) => {
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('button', { name: /MySQL 8\.4\.6 : Local MySQL : tablesail/ })).toBeVisible()
    await showConnections(page)
    await expect(card(page, 'Local MySQL')).toContainText('Connected')
    await expect(card(page, 'Local MySQL').getByRole('button', { name: /^Open / })).toHaveCount(0)
    // Opening an already-open connection just switches to it.
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('button', { name: /Local MySQL/ })).toBeVisible()
  })

  test('card menu: Open, Disconnect and Switch to', async ({ page }) => {
    await cardAction(page, 'Local Postgres', 'Open')
    await expect(page.getByRole('button', { name: /Local Postgres/ })).toBeVisible()
    await showConnections(page)

    await card(page, 'Local Postgres').getByRole('button', { name: 'Actions' }).click()
    await expect(page.getByRole('menuitem', { name: 'Switch to' })).toBeVisible()
    await page.getByRole('menuitem', { name: 'Disconnect' }).click()
    await expect(card(page, 'Local Postgres')).not.toContainText('Connected')
    await expect(card(page, 'Local Postgres').getByRole('button', { name: /^Open / })).toBeVisible()
  })

  test('right-click opens the same card menu', async ({ page }) => {
    await card(page, 'MariaDB').click({ button: 'right' })
    const items = await page.getByRole('menuitem').allTextContents()
    expect(items.map((t) => t.trim())).toEqual(['Open', 'Edit…', 'Duplicate', 'Copy as URL', 'Delete…'])
    await page.keyboard.press('Escape')
    await expect(page.getByRole('menu')).toHaveCount(0)
  })

  test('duplicate adds a copy', async ({ page }) => {
    await cardAction(page, 'MariaDB', 'Duplicate')
    await expect(card(page, 'MariaDB copy')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Connections 8' })).toBeVisible()
  })

  test('a connection that is down shows the error on its card', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Down DB')
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('down.internal')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)

    await card(page, 'Down DB').getByRole('button', { name: /^Open / }).click()
    await expect(card(page, 'Down DB')).toContainText('connection refused')
    await expect(page.getByRole('button', { name: 'Toggle sidebar' })).toHaveCount(0)
  })

  test('a wrong password asks again, the right one opens the workspace', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Locked DB')
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('locked.internal')
    await form.getByRole('button', { name: 'Save' }).click()

    await card(page, 'Locked DB').getByRole('button', { name: /^Open / }).click()
    const prompt = dialog(page, 'Password required')
    await expect(prompt).toBeVisible()
    await expect(prompt).toContainText('Locked DB')
    await expect(prompt.getByLabel(/Password for/)).toBeFocused()

    await prompt.getByLabel(/Password for/).fill('wrong')
    await page.keyboard.press('Enter')
    await expect(prompt.getByText('password authentication failed')).toBeVisible()

    await prompt.getByLabel(/Password for/).fill('secret')
    await page.keyboard.press('Enter')
    await expect(prompt).toHaveCount(0)
    await expect(page.getByRole('button', { name: /Locked DB/ })).toBeVisible()
  })
})

test.describe('new connection form', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('opens with PostgreSQL defaults and an engine tile per supported engine', async ({ page }) => {
    const form = await openForm(page)
    const engines = form.getByRole('radiogroup', { name: 'Engine' }).getByRole('radio')
    await expect(engines).toHaveText([/PostgreSQL$/, /MySQL$/, /MariaDB$/, /SQLite$/, /MongoDB$/])
    await expect(form.getByRole('radio', { name: 'PostgreSQL' })).toHaveAttribute('aria-checked', 'true')
    await expect(form.getByRole('textbox', { name: 'Host', exact: true })).toHaveValue('127.0.0.1')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('5432')
    await expect(form.getByRole('textbox', { name: 'User', exact: true })).toHaveValue('postgres')
    await expect(form.getByRole('radio', { name: 'Local' })).toHaveAttribute('aria-checked', 'true')
  })

  test('switching engine updates default port and user, and fields that do not apply', async ({ page }) => {
    const form = await openForm(page)

    await pickEngine(form, 'MySQL')
    await expect(form.getByRole('radio', { name: 'MySQL' })).toHaveAttribute('aria-checked', 'true')
    await expect(form.getByRole('radio', { name: 'PostgreSQL' })).toHaveAttribute('aria-checked', 'false')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('3306')
    await expect(form.getByRole('textbox', { name: 'User', exact: true })).toHaveValue('root')

    await pickEngine(form, 'MariaDB')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('3306')
    await expect(form.getByRole('textbox', { name: 'User', exact: true })).toHaveValue('root')

    await pickEngine(form, 'MongoDB')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('27017')
    await expect(form.getByRole('textbox', { name: 'User', exact: true })).toHaveValue('')
    await expect(form.getByRole('textbox', { name: /Connection URL/ })).toBeVisible()

    await pickEngine(form, 'SQLite')
    await expect(form.getByText('Database file')).toBeVisible()
    await expect(form.getByRole('textbox', { name: 'Host', exact: true })).toHaveCount(0)
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveCount(0)
    await expect(form.getByText('SSH tunnel')).toHaveCount(0)

    await pickEngine(form, 'PostgreSQL')
    await expect(form.getByRole('textbox', { name: 'Host', exact: true })).toHaveValue('127.0.0.1')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('5432')
  })

  test('a port or user the person typed survives an engine switch', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('spinbutton', { name: 'Port' }).fill('6543')
    await form.getByRole('textbox', { name: 'User', exact: true }).fill('app')
    await pickEngine(form, 'MySQL')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('6543')
    await expect(form.getByRole('textbox', { name: 'User', exact: true })).toHaveValue('app')
  })

  test('environment picker is a single-choice radio group', async ({ page }) => {
    const form = await openForm(page)
    const env = form.getByRole('radiogroup', { name: 'Environment' })
    await expect(env.getByRole('radio')).toHaveText(['Local', 'Dev', 'Staging', 'Production'])
    await env.getByRole('radio', { name: 'Production' }).click()
    await expect(env.getByRole('radio', { name: 'Production' })).toHaveAttribute('aria-checked', 'true')
    await expect(env.getByRole('radio', { name: 'Local' })).toHaveAttribute('aria-checked', 'false')
    await env.getByRole('radio', { name: 'Staging' }).click()
    await expect(env.getByRole('radio', { name: 'Staging' })).toHaveAttribute('aria-checked', 'true')
    await expect(env.getByRole('radio', { name: 'Production' })).toHaveAttribute('aria-checked', 'false')
  })

  test('validation: host is required for network engines', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('Host is required.')).toBeVisible()
    await expect(dialog(page, 'New connection')).toBeVisible()

    // Test shows the same problem instead of connecting.
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('Host is required.').first()).toBeVisible()
  })

  test('a validation problem is shown once, not once per button pressed', async ({ page }) => {
    // BUG: Save and then Test each set their own notice and neither clears the other, so the
    // same "Host is required." message appears twice (ConnectionForm.svelte: saveError / testError).
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('')
    await form.getByRole('button', { name: 'Save' }).click()
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('Host is required.')).toHaveCount(1)
  })

  test('validation: SQLite needs a file, MongoDB accepts a URL instead of a host', async ({ page }) => {
    const form = await openForm(page)
    await pickEngine(form, 'SQLite')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('Choose a SQLite database file.')).toBeVisible()

    await pickEngine(form, 'MongoDB')
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('Host is required.')).toBeVisible()
    await form.getByRole('textbox', { name: /Connection URL/ }).fill('mongodb://example.com:27017/app')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)
  })

  test('validation: SSH tunnel and jump host hosts are required once enabled', async ({ page }) => {
    const form = await openForm(page)
    await form.getByText('SSH tunnel', { exact: true }).click()
    await form.getByLabel('Connect through an SSH tunnel').check()
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('SSH host is required.')).toBeVisible()

    await form.getByPlaceholder('bastion.example.com').first().fill('bastion.example.com')
    await form.getByLabel('Use a jump host').check()
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('Jump host is required.')).toBeVisible()
  })

  test('Test reports the server version, or the failure', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('Connected. Server PostgreSQL 17.6 · 4 ms')).toBeVisible()

    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('down.internal')
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('connection refused')).toBeVisible()
    await expect(form.getByText('Connected. Server')).toHaveCount(0)
  })

  test('saving adds the card, selects it and files it under its environment', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Reporting replica')
    await form.getByRole('textbox', { name: 'Host', exact: true }).fill('replica.internal')
    await form.getByRole('textbox', { name: /^Database/ }).fill('reports')
    await form.getByRole('radiogroup', { name: 'Environment' }).getByRole('radio', { name: 'Staging' }).click()
    await form.getByRole('checkbox', { name: /Read-only connection/ }).check()
    await form.getByRole('button', { name: 'Save' }).click()

    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(page.getByRole('heading', { name: 'Connections 8' })).toBeVisible()
    const created = card(page, 'Reporting replica')
    await expect(created).toHaveAttribute('aria-selected', 'true')
    await expect(created).toContainText('postgres@replica.internal:5432/reports')
    await expect(created).toContainText('staging')
    await expect(created).toContainText('read-only')
  })

  test('an empty name falls back to the database name', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: /^Database/ }).fill('analytics')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(card(page, 'analytics')).toBeVisible()
  })

  test('Escape, Cancel, the backdrop and ⌘N all behave as dialogs should', async ({ page }) => {
    let form = await openForm(page)
    await page.keyboard.press('Escape')
    await expect(form).toHaveCount(0)

    await page.keyboard.press(`${MOD}+n`)
    form = dialog(page, 'New connection')
    await expect(form).toBeVisible()
    await form.getByRole('button', { name: 'Cancel' }).click()
    await expect(form).toHaveCount(0)

    await openForm(page)
    await page.mouse.click(5, 5) // backdrop
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(page.getByRole('option')).toHaveCount(7)
  })

  test('⌘↩ saves from anywhere in the form', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Keyboard saved')
    await page.keyboard.press(`${MOD}+Enter`)
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(card(page, 'Keyboard saved')).toBeVisible()
  })

  test('the first field is focused when the dialog opens', async ({ page }) => {
    // BUG: Modal.svelte never moves focus into the dialog, so typing right after opening goes
    // nowhere (expected: the Name field has focus and receives the keystrokes).
    const form = await openForm(page)
    // Typing right away should land in the form, not behind it.
    await page.keyboard.type('typed without clicking')
    await expect(form.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue('typed without clicking')
  })
})

test.describe('edit and delete', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('edit opens the form prefilled and saves changes in place', async ({ page }) => {
    await cardAction(page, 'Staging API', 'Edit…')
    const form = dialog(page, 'Edit connection')
    await expect(form).toBeVisible()
    await expect(form.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue('Staging API')
    await expect(form.getByRole('textbox', { name: 'Host', exact: true })).toHaveValue('db.staging.internal')
    await expect(form.getByRole('spinbutton', { name: 'Port' })).toHaveValue('5432')
    await expect(form.getByRole('radio', { name: 'PostgreSQL' })).toHaveAttribute('aria-checked', 'true')
    await expect(form.getByRole('radio', { name: 'Staging' })).toHaveAttribute('aria-checked', 'true')
    await expect(form.getByLabel('Password', { exact: true })).toHaveAttribute('placeholder', 'Leave blank to keep the saved one')

    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Staging renamed')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form).toHaveCount(0)
    await expect(card(page, 'Staging renamed')).toBeVisible()
    await expect(card(page, 'Staging API')).toHaveCount(0)
    await expect(page.getByRole('heading', { name: 'Connections 7' })).toBeVisible()
  })

  test('edit can switch the environment, which moves the card to another group', async ({ page }) => {
    await cardAction(page, 'MariaDB', 'Edit…')
    const form = dialog(page, 'Edit connection')
    await form.getByRole('radio', { name: 'Production' }).click()
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form).toHaveCount(0)
    const names = await page.getByRole('option').evaluateAll((els) => els.map((e) => e.querySelector('.name')?.textContent))
    expect(names.indexOf('MariaDB')).toBeLessThan(names.indexOf('Staging API'))
    await expect(card(page, 'MariaDB')).toContainText('production')
  })

  test('delete asks first; Cancel keeps the connection', async ({ page }) => {
    await cardAction(page, 'MariaDB', 'Delete…')
    const confirm = dialog(page, 'Delete connection')
    await expect(confirm).toContainText('Delete MariaDB?')
    await expect(confirm).toContainText('saved password is removed from the keychain')
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    await expect(confirm).toHaveCount(0)
    await expect(card(page, 'MariaDB')).toBeVisible()
  })

  test('delete removes the card and updates the count', async ({ page }) => {
    await cardAction(page, 'MariaDB', 'Delete…')
    await dialog(page, 'Delete connection').getByRole('button', { name: 'Delete', exact: true }).click()
    await expect(card(page, 'MariaDB')).toHaveCount(0)
    await expect(page.getByRole('heading', { name: 'Connections 6' })).toBeVisible()
  })

  test('deleting an open connection disconnects it and closes its workspace', async ({ page }) => {
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('button', { name: 'Toggle sidebar' })).toBeVisible()
    await showConnections(page)
    await cardAction(page, 'Local MySQL', 'Delete…')
    await dialog(page, 'Delete connection').getByRole('button', { name: 'Delete', exact: true }).click()
    await expect(card(page, 'Local MySQL')).toHaveCount(0)
    // Back on the welcome screen with nothing live.
    await expect(page.getByText('Connected', { exact: true })).toHaveCount(0)
  })

  test('deleting every connection shows the empty state', async ({ page }) => {
    for (const name of ['Production', 'Staging API', 'MariaDB', 'Mongo', 'Local MySQL', 'Local Postgres', 'nasty.db']) {
      await cardAction(page, name, 'Delete…')
      await dialog(page, 'Delete connection').getByRole('button', { name: 'Delete', exact: true }).click()
      await expect(card(page, name)).toHaveCount(0)
    }
    await expect(page.getByText('No connections yet. Create one to get started.')).toBeVisible()
  })
})

test.describe('SQLite file flow', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('"Open SQLite file" starts a SQLite form; Browse fills the path and the name defaults to the file name', async ({ page }) => {
    await setDialog(page, { open: '/Users/me/data/orders.sqlite' })
    await page.getByRole('button', { name: 'Open SQLite file' }).click()
    const form = dialog(page, 'New connection')
    await expect(form).toBeVisible()
    await expect(form.getByRole('radio', { name: 'SQLite' })).toHaveAttribute('aria-checked', 'true')
    await expect(form.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue('')

    await form.getByRole('button', { name: 'Browse…' }).click()
    await expect(form.getByPlaceholder('/path/to/database.db')).toHaveValue('/Users/me/data/orders.sqlite')

    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form).toHaveCount(0)
    const created = card(page, 'orders.sqlite')
    await expect(created).toBeVisible()
    await expect(created).toContainText('SQLite')
    await expect(created).toContainText('/Users/me/data/orders.sqlite')
  })

  test('a cancelled file dialog leaves the path empty and Save explains why', async ({ page }) => {
    await setDialog(page, { open: null })
    await page.getByRole('button', { name: 'Open SQLite file' }).click()
    const form = dialog(page, 'New connection')
    await form.getByRole('button', { name: 'Browse…' }).click()
    await expect(form.getByPlaceholder('/path/to/database.db')).toHaveValue('')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form.getByText('Choose a SQLite database file.')).toBeVisible()
  })

  test('a typed path works without the dialog, and the saved SQLite connection opens', async ({ page }) => {
    await page.getByRole('button', { name: 'Open SQLite file' }).click()
    const form = dialog(page, 'New connection')
    await form.getByPlaceholder('/path/to/database.db').fill('/tmp/typed.db')
    await form.getByRole('textbox', { name: 'Name', exact: true }).fill('Typed SQLite')
    await form.getByRole('button', { name: 'Save' }).click()
    await card(page, 'Typed SQLite').getByRole('button', { name: /^Open / }).click()
    await expect(page.getByRole('button', { name: /SQLite 3\.50\.4 : Typed SQLite/ })).toBeVisible()
  })

  test('the seeded SQLite connection opens straight away (no password prompt)', async ({ page }) => {
    await card(page, 'nasty.db').dblclick()
    await expect(page.getByRole('button', { name: /SQLite 3\.50\.4 : nasty\.db/ })).toBeVisible()
    await expect(dialog(page, 'Password required')).toHaveCount(0)
  })
})
