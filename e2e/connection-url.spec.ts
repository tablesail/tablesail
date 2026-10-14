import { test, expect } from './fixtures'
import { MOD, card, gotoApp, invokes, recordInvokes } from './helpers'
import type { Locator, Page } from '@playwright/test'

const dialog = (page: Page, name: string) => page.getByRole('dialog', { name })

async function openForm(page: Page) {
  await page.getByRole('button', { name: 'New connection' }).click()
  await expect(dialog(page, 'New connection')).toBeVisible()
  return dialog(page, 'New connection')
}

type Form = ReturnType<typeof dialog>

const field = {
  name: (f: Form) => f.getByRole('textbox', { name: 'Name', exact: true }),
  host: (f: Form) => f.getByRole('textbox', { name: 'Host', exact: true }),
  port: (f: Form) => f.getByRole('spinbutton', { name: 'Port' }),
  user: (f: Form) => f.getByRole('textbox', { name: 'User', exact: true }),
  password: (f: Form) => f.getByLabel('Password', { exact: true }),
  database: (f: Form) => f.getByRole('textbox', { name: /^Database/ }),
  mongoUrl: (f: Form) => f.getByRole('textbox', { name: /Connection URL/ }),
  file: (f: Form) => f.getByPlaceholder('/path/to/database.db'),
  importUrl: (f: Form) => f.getByRole('textbox', { name: 'Import from URL' }),
  importToggle: (f: Form) => f.getByRole('button', { name: 'Import from URL' }),
  sslMode: (f: Form) => f.getByLabel('Mode'),
}

/** Imports through the row at the top of the form (open it, type, Enter). */
async function importUrl(f: Form, url: string) {
  await field.importToggle(f).click()
  await field.importUrl(f).fill(url)
  await field.importUrl(f).press('Enter')
}

/** A real paste: puts the text on the clipboard and presses the paste shortcut in the focused field. */
async function pasteInto(page: Page, target: Locator, text: string) {
  await page.evaluate((t) => navigator.clipboard.writeText(t), text)
  await target.focus()
  await page.keyboard.press(`${MOD}+v`)
}

const engineOn = (f: Form, label: string) => expect(f.getByRole('radio', { name: label })).toHaveAttribute('aria-checked', 'true')

test.describe('Import from URL', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('a button at the top opens a URL box that takes the focus', async ({ page }) => {
    const form = await openForm(page)
    await expect(field.importUrl(form)).toHaveCount(0)
    await field.importToggle(form).click()
    await expect(field.importUrl(form)).toBeFocused()
    await expect(field.importUrl(form)).toHaveAttribute('placeholder', /^postgresql:\/\//)
    await expect(form.getByRole('button', { name: 'Import', exact: true })).toBeDisabled()
    await field.importUrl(form).fill('postgres://h/d')
    await expect(form.getByRole('button', { name: 'Import', exact: true })).toBeEnabled()
  })

  test('the URL button comes before the engine tiles, and the Name field still gets the focus on open', async ({ page }) => {
    const form = await openForm(page)
    const toggleY = (await field.importToggle(form).boundingBox())!.y
    const enginesY = (await form.getByRole('radiogroup', { name: 'Engine' }).boundingBox())!.y
    expect(toggleY).toBeLessThan(enginesY)
    await page.keyboard.type('typed without clicking')
    await expect(field.name(form)).toHaveValue('typed without clicking')
  })

  test('the Mongo "Connection URL" field is still the only field with that name', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'MongoDB' }).click()
    await field.importToggle(form).click()
    await expect(field.mongoUrl(form)).toHaveCount(1)
    await expect(field.importUrl(form)).toHaveCount(1)
  })

  test('PostgreSQL: fills host, port, user, password, database and SSL mode, and names the connection', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://app:s3cret@db.example.com:5433/orders?sslmode=verify-full&application_name=x')

    await expect(field.host(form)).toHaveValue('db.example.com')
    await expect(field.port(form)).toHaveValue('5433')
    await expect(field.user(form)).toHaveValue('app')
    await expect(field.password(form)).toHaveValue('s3cret')
    await expect(field.database(form)).toHaveValue('orders')
    await expect(field.name(form)).toHaveValue('orders @ db.example.com')
    await engineOn(form, 'PostgreSQL')
    await expect(field.sslMode(form)).toHaveValue('verifyFull')
    await expect(form.getByText('Filled from URL: PostgreSQL · app@db.example.com:5433/orders · password set')).toBeVisible()

    // The box closes and the toggle is back, with the focus.
    await expect(field.importUrl(form)).toHaveCount(0)
    await expect(field.importToggle(form)).toBeFocused()
  })

  test('the Import button works too, and the box can be closed without importing', async ({ page }) => {
    const form = await openForm(page)
    await field.importToggle(form).click()
    await field.importUrl(form).fill('postgresql://u@click.example.com/db')
    await form.getByRole('button', { name: 'Import', exact: true }).click()
    await expect(field.host(form)).toHaveValue('click.example.com')

    await field.importToggle(form).click()
    await field.importUrl(form).fill('postgres://ignored.example.com/x')
    await form.getByRole('button', { name: 'Close URL import' }).click()
    await expect(field.importUrl(form)).toHaveCount(0)
    await expect(field.host(form)).toHaveValue('click.example.com')
  })

  test('the default port applies when the URL names none, replacing a typed one', async ({ page }) => {
    const form = await openForm(page)
    await field.port(form).fill('6543')
    await importUrl(form, 'postgres://h.example.com/d')
    await expect(field.port(form)).toHaveValue('5432')
  })

  test('what the URL does not say is left as it was', async ({ page }) => {
    const form = await openForm(page)
    await field.user(form).fill('custom')
    await field.database(form).fill('keepme')
    await field.name(form).fill('My name')
    await form.getByRole('radio', { name: 'Staging' }).click()
    await importUrl(form, 'postgres://only.host.example.com')
    await expect(field.host(form)).toHaveValue('only.host.example.com')
    await expect(field.user(form)).toHaveValue('custom')
    await expect(field.database(form)).toHaveValue('keepme')
    await expect(field.name(form)).toHaveValue('My name')
    await expect(field.password(form)).toHaveValue('')
    await expect(form.getByRole('radio', { name: 'Staging' })).toHaveAttribute('aria-checked', 'true')
    await expect(field.sslMode(form)).toHaveValue('prefer')
  })

  test('percent-encoded user, password and database are decoded; IPv6 hosts lose their brackets', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://me%40corp:p%40ss%3Aw%2Frd@[2001:db8::7]:5433/my%20db')
    await expect(field.user(form)).toHaveValue('me@corp')
    await expect(field.password(form)).toHaveValue('p@ss:w/rd')
    await expect(field.database(form)).toHaveValue('my db')
    await expect(field.host(form)).toHaveValue('2001:db8::7')
    await expect(field.port(form)).toHaveValue('5433')
  })

  test('MySQL: switches the engine, with its default port and the SSL mode', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'mysql://shop:pw@mysql.example.com/shop?ssl-mode=REQUIRED&charset=utf8mb4')
    await engineOn(form, 'MySQL')
    await expect(field.host(form)).toHaveValue('mysql.example.com')
    await expect(field.port(form)).toHaveValue('3306')
    await expect(field.user(form)).toHaveValue('shop')
    await expect(field.database(form)).toHaveValue('shop')
    await expect(field.sslMode(form)).toHaveValue('require')
  })

  test('MySQL URL without a user swaps in the MySQL default user', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'mysql://mysql.example.com:3307/shop')
    await expect(field.user(form)).toHaveValue('root')
    await expect(field.port(form)).toHaveValue('3307')
  })

  test('MariaDB: mariadb:// picks the MariaDB tile, and mysql:// keeps a MariaDB form on MariaDB', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'mariadb://app@maria.example.com:3307/d')
    await engineOn(form, 'MariaDB')
    await expect(field.port(form)).toHaveValue('3307')

    await importUrl(form, 'mysql://app@other.example.com/d')
    await engineOn(form, 'MariaDB')
    await expect(field.host(form)).toHaveValue('other.example.com')
  })

  test('JDBC, SQLAlchemy and shell-wrapped URLs', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'jdbc:mysql://jdbc.example.com:3307/shop?user=u&password=p%40ss&useSSL=false')
    await engineOn(form, 'MySQL')
    await expect(field.host(form)).toHaveValue('jdbc.example.com')
    await expect(field.user(form)).toHaveValue('u')
    await expect(field.password(form)).toHaveValue('p@ss')
    await expect(field.sslMode(form)).toHaveValue('disable')

    await importUrl(form, 'postgresql+psycopg2://alchemy:pw@alchemy.example.com/d')
    await engineOn(form, 'PostgreSQL')
    await expect(field.host(form)).toHaveValue('alchemy.example.com')

    await importUrl(form, 'psql "postgresql://shell:pw@shell.example.com:6432/d?sslmode=require"')
    await expect(field.host(form)).toHaveValue('shell.example.com')
    await expect(field.port(form)).toHaveValue('6432')
    await expect(field.sslMode(form)).toHaveValue('require')

    await importUrl(form, 'DATABASE_URL=postgres://env:pw@env.example.com/d')
    await expect(field.host(form)).toHaveValue('env.example.com')
  })

  test('SQLite: sqlite:// URLs, file: URLs and bare paths switch to SQLite with the file filled in', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'sqlite:///Users/me/data/orders.db')
    await engineOn(form, 'SQLite')
    await expect(field.file(form)).toHaveValue('/Users/me/data/orders.db')
    await expect(field.name(form)).toHaveValue('orders.db')
    await expect(field.host(form)).toHaveCount(0)

    await importUrl(form, 'file:///Users/me/My%20Data/other.sqlite3')
    await expect(field.file(form)).toHaveValue('/Users/me/My Data/other.sqlite3')
    await expect(field.name(form)).toHaveValue('other.sqlite3')

    await importUrl(form, '~/data/bare.sqlite')
    await expect(field.file(form)).toHaveValue('~/data/bare.sqlite')
    await expect(form.getByText('Filled from URL: SQLite · ~/data/bare.sqlite')).toBeVisible()
  })

  test('SQLite: the imported file saves and shows on the list', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'sqlite:///Users/me/data/orders.db?mode=ro')
    await expect(form.getByRole('checkbox', { name: /Read-only connection/ })).toBeChecked()
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    const created = card(page, 'orders.db')
    await expect(created).toContainText('SQLite')
    await expect(created).toContainText('/Users/me/data/orders.db')
    await expect(created).toContainText('read-only')
  })

  test('SQLite: an in-memory URL is refused', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'sqlite://:memory:')
    await expect(form.getByRole('alert').filter({ hasText: 'In-memory SQLite databases cannot be opened' })).toBeVisible()
    await engineOn(form, 'PostgreSQL')
  })

  test('MongoDB: the URL stays in the URL field without its password; host, port and database are prefilled', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'mongodb://app:p%40ss@mongo.example.com:27018/shop?replicaSet=rs0&authSource=admin')
    await engineOn(form, 'MongoDB')
    await expect(field.mongoUrl(form)).toHaveValue('mongodb://app@mongo.example.com:27018/shop?replicaSet=rs0&authSource=admin')
    await expect(field.host(form)).toHaveValue('mongo.example.com')
    await expect(field.port(form)).toHaveValue('27018')
    await expect(field.user(form)).toHaveValue('app')
    await expect(field.database(form)).toHaveValue('shop')
    await expect(field.password(form)).toHaveValue('p@ss')
    await expect(field.name(form)).toHaveValue('shop @ mongo.example.com')
  })

  test('MongoDB: mongodb+srv keeps the whole URL and leaves the port at its default', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'mongodb+srv://atlas:pw@cluster0.abc12.mongodb.net/?retryWrites=true&w=majority')
    await expect(field.mongoUrl(form)).toHaveValue('mongodb+srv://atlas@cluster0.abc12.mongodb.net/?retryWrites=true&w=majority')
    await expect(field.host(form)).toHaveValue('cluster0.abc12.mongodb.net')
    await expect(field.port(form)).toHaveValue('27017')
    await expect(field.name(form)).toHaveValue('cluster0.abc12.mongodb.net')
  })

  test('switching away from MongoDB drops its URL from what is saved', async ({ page }) => {
    await recordInvokes(page)
    const form = await openForm(page)
    await importUrl(form, 'mongodb://m.example.com/db')
    await importUrl(form, 'postgres://p.example.com/db')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    const [save] = await invokes(page, 'connection_save')
    expect((save.args.config as { url: unknown }).url).toBeNull()
  })

  test('the password goes to the secrets, never into the saved config (SQL engines and MongoDB)', async ({ page }) => {
    await recordInvokes(page)
    for (const [url, engine] of [
      ['postgres://u:hunter2@pg.example.com/d', 'postgres'],
      ['mysql://u:hunter2@my.example.com/d', 'mysql'],
      ['mariadb://u:hunter2@ma.example.com/d', 'mariadb'],
      ['mongodb://u:hunter2@mo.example.com/d', 'mongodb'],
    ] as const) {
      const form = await openForm(page)
      await importUrl(form, url)
      await form.getByRole('button', { name: 'Save' }).click()
      await expect(dialog(page, 'New connection')).toHaveCount(0)
      const saves = await invokes(page, 'connection_save')
      const save = saves.at(-1)!
      expect((save.args.config as { engine: string }).engine).toBe(engine)
      expect(JSON.stringify(save.args.config)).not.toContain('hunter2')
      expect((save.args.secrets as { dbPassword: string }).dbPassword).toBe('hunter2')
    }
  })

  test('Test connects with the imported fields and password', async ({ page }) => {
    await recordInvokes(page)
    const form = await openForm(page)
    await importUrl(form, 'postgres://tester:pw1@test.example.com:6000/probe')
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('Connected. Server PostgreSQL 17.6')).toBeVisible()
    const [test] = await invokes(page, 'connection_test')
    expect(test.args.config).toMatchObject({ engine: 'postgres', host: 'test.example.com', port: 6000, user: 'tester', database: 'probe' })
    expect(test.args.secrets).toMatchObject({ dbPassword: 'pw1' })
  })

  test('an import clears an earlier Test result', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('button', { name: 'Test' }).click()
    await expect(form.getByText('Connected. Server')).toBeVisible()
    await importUrl(form, 'postgres://other.example.com/d')
    await expect(form.getByText('Connected. Server')).toHaveCount(0)
  })

  test('saving the imported connection puts a card on the list', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgresql://report:pw@replica.example.com:5433/reports?sslmode=require')
    await form.getByRole('radio', { name: 'Staging' }).click()
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    const created = card(page, 'reports @ replica.example.com')
    await expect(created).toContainText('report@replica.example.com:5433/reports')
    await expect(created).toContainText('staging')
  })

  test('the name is filled while empty or still auto-filled, never over a typed one', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://u@first.example.com/db1')
    await expect(field.name(form)).toHaveValue('db1 @ first.example.com')
    await importUrl(form, 'postgres://u@second.example.com/db2')
    await expect(field.name(form)).toHaveValue('db2 @ second.example.com')

    await field.name(form).fill('Billing primary')
    await importUrl(form, 'postgres://u@third.example.com/db3')
    await expect(field.name(form)).toHaveValue('Billing primary')
    await expect(field.host(form)).toHaveValue('third.example.com')
  })

  test('editing a saved connection: an import replaces its fields and the new password is sent on Save', async ({ page }) => {
    await recordInvokes(page)
    await card(page, 'Staging API').getByRole('button', { name: 'Actions' }).click()
    await page.getByRole('menuitem', { name: 'Edit…' }).click()
    const form = dialog(page, 'Edit connection')
    await expect(field.password(form)).toHaveAttribute('placeholder', 'Leave blank to keep the saved one')
    await importUrl(form, 'postgres://newuser:newpw@moved.staging.internal:5433/api')
    await expect(field.name(form)).toHaveValue('Staging API')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form).toHaveCount(0)
    await expect(card(page, 'Staging API')).toContainText('newuser@moved.staging.internal:5433/api')
    const [save] = await invokes(page, 'connection_save')
    expect(save.args.secrets).toMatchObject({ dbPassword: 'newpw' })
    expect((save.args.config as { id: string }).id).toBe('c-stg')
  })

  test('a URL without a password leaves the saved password alone on edit', async ({ page }) => {
    await recordInvokes(page)
    await card(page, 'Staging API').getByRole('button', { name: 'Actions' }).click()
    await page.getByRole('menuitem', { name: 'Edit…' }).click()
    const form = dialog(page, 'Edit connection')
    await importUrl(form, 'postgres://u@no-password.staging.internal/api')
    await expect(field.password(form)).toHaveValue('')
    await form.getByRole('button', { name: 'Save' }).click()
    const [save] = await invokes(page, 'connection_save')
    expect(save.args.secrets).toBeNull()
  })
})

test.describe('Import from URL: errors', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('an unknown scheme is reported inline and nothing changes', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'redis://cache.example.com:6379/0')
    const alert = form.getByRole('alert').filter({ hasText: 'Unsupported URL scheme "redis"' })
    await expect(alert).toBeVisible()
    await expect(alert).toContainText('postgresql://')
    await expect(field.importUrl(form)).toBeVisible() // still open to fix
    await expect(field.importUrl(form)).toHaveValue('redis://cache.example.com:6379/0')
    await expect(field.host(form)).toHaveValue('127.0.0.1')
    await engineOn(form, 'PostgreSQL')
  })

  test('text that is not a URL is explained, and typing clears the message', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'db.example.com')
    await expect(form.getByRole('alert').filter({ hasText: 'does not look like a connection URL' })).toBeVisible()
    await field.importUrl(form).fill('postgres://h/d')
    await expect(form.getByRole('alert').filter({ hasText: 'does not look like a connection URL' })).toHaveCount(0)
  })

  test('a bad port is reported', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://h.example.com:99999/d')
    await expect(form.getByRole('alert').filter({ hasText: 'Invalid port "99999"' })).toBeVisible()
    await expect(field.host(form)).toHaveValue('127.0.0.1')
  })

  test('a mistyped SSL mode is reported, not quietly weakened', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://h.example.com/d?sslmode=verify-fulll')
    await expect(form.getByRole('alert').filter({ hasText: 'Unknown SSL mode "verify-fulll"' })).toBeVisible()
    await expect(field.host(form)).toHaveValue('127.0.0.1')
  })

  test('a failed import does not change the form even after a good one', async ({ page }) => {
    const form = await openForm(page)
    await importUrl(form, 'postgres://good.example.com/d')
    await field.importToggle(form).click()
    await field.importUrl(form).fill('nonsense://x')
    await field.importUrl(form).press('Enter')
    await expect(form.getByRole('alert').first()).toContainText('Unsupported URL scheme')
    await expect(field.host(form)).toHaveValue('good.example.com')
  })

  test('Enter with nothing typed does nothing, and ⌘↩ in the box still saves the form', async ({ page }) => {
    const form = await openForm(page)
    await field.importToggle(form).click()
    await field.importUrl(form).press('Enter')
    await expect(form.getByRole('alert')).toHaveCount(0)
    await expect(field.importUrl(form)).toBeVisible()
    await field.name(form).fill('Saved from the box')
    await field.importUrl(form).focus()
    await page.keyboard.press(`${MOD}+Enter`)
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(card(page, 'Saved from the box')).toBeVisible()
  })

  test('Escape closes the URL box first and the dialog second', async ({ page }) => {
    const form = await openForm(page)
    await field.importToggle(form).click()
    await expect(field.importUrl(form)).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(field.importUrl(form)).toHaveCount(0)
    await expect(form).toBeVisible()
    await expect(field.importToggle(form)).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(form).toHaveCount(0)
  })
})

test.describe('Pasting a URL', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  test('into the import box applies it at once', async ({ page }) => {
    const form = await openForm(page)
    await field.importToggle(form).click()
    await pasteInto(page, field.importUrl(form), 'postgres://pasted:pw@paste.example.com:5433/pasted_db')
    await expect(field.host(form)).toHaveValue('paste.example.com')
    await expect(field.user(form)).toHaveValue('pasted')
    await expect(field.importUrl(form)).toHaveCount(0)
  })

  test('into the import box, text that is not a URL stays in the box with the reason', async ({ page }) => {
    const form = await openForm(page)
    await field.importToggle(form).click()
    await pasteInto(page, field.importUrl(form), 'redis://cache.example.com')
    await expect(field.importUrl(form)).toHaveValue('redis://cache.example.com')
    await expect(form.getByRole('alert').filter({ hasText: 'Unsupported URL scheme "redis"' })).toBeVisible()
  })

  test('into the Host field fills the whole form', async ({ page }) => {
    const form = await openForm(page)
    await pasteInto(page, field.host(form), 'mysql://pasted:pw@mysql.paste.example.com:3307/shop')
    await engineOn(form, 'MySQL')
    await expect(field.host(form)).toHaveValue('mysql.paste.example.com')
    await expect(field.port(form)).toHaveValue('3307')
    await expect(field.user(form)).toHaveValue('pasted')
    await expect(field.password(form)).toHaveValue('pw')
    await expect(field.database(form)).toHaveValue('shop')
    await expect(form.getByText('Filled from URL: MySQL')).toBeVisible()
  })

  test('into the Host field: a wrong scheme is reported and the Host stays as it was', async ({ page }) => {
    const form = await openForm(page)
    await pasteInto(page, field.host(form), 'https://example.com/db')
    await expect(form.getByRole('alert').filter({ hasText: 'Unsupported URL scheme "https"' })).toBeVisible()
    await expect(field.host(form)).toHaveValue('127.0.0.1')
  })

  test('into the Host field: a plain host name is pasted as it is', async ({ page }) => {
    const form = await openForm(page)
    await field.host(form).fill('')
    await pasteInto(page, field.host(form), 'db.plain.example.com')
    await expect(field.host(form)).toHaveValue('db.plain.example.com')
    await expect(form.getByText('Filled from URL')).toHaveCount(0)
  })

  test('into the MongoDB URL field fills the form around it', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'MongoDB' }).click()
    await pasteInto(page, field.mongoUrl(form), 'mongodb://m:pw@paste.mongo.example.com:27019/app?replicaSet=rs0')
    await expect(field.mongoUrl(form)).toHaveValue('mongodb://m@paste.mongo.example.com:27019/app?replicaSet=rs0')
    await expect(field.host(form)).toHaveValue('paste.mongo.example.com')
    await expect(field.port(form)).toHaveValue('27019')
    await expect(field.password(form)).toHaveValue('pw')
  })

  test('into the SQLite file field turns a sqlite:// URL into the path; a plain path is pasted as it is', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'SQLite' }).click()
    await pasteInto(page, field.file(form), 'sqlite:///Users/me/pasted.db')
    await expect(field.file(form)).toHaveValue('/Users/me/pasted.db')
    await expect(field.name(form)).toHaveValue('pasted.db')

    await field.file(form).fill('')
    await pasteInto(page, field.file(form), '/Users/me/plain.db')
    await expect(field.file(form)).toHaveValue('/Users/me/plain.db')
  })

  test('a MongoDB password typed into the URL field is saved to the keychain, not into the config', async ({ page }) => {
    await recordInvokes(page)
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'MongoDB' }).click()
    await field.mongoUrl(form).fill('mongodb://typed:p%40ss$&1@typed.mongo.example.com/app')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    const [save] = await invokes(page, 'connection_save')
    expect((save.args.config as { url: string }).url).toBe('mongodb://typed@typed.mongo.example.com/app')
    expect(JSON.stringify(save.args.config)).not.toContain('ss$&1')
    expect((save.args.secrets as { dbPassword: string }).dbPassword).toBe('p@ss$&1')
  })

  test('the Password field wins over a password typed into the Mongo URL', async ({ page }) => {
    await recordInvokes(page)
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'MongoDB' }).click()
    await field.mongoUrl(form).fill('mongodb://typed:inurl@typed.mongo.example.com/app')
    await field.password(form).fill('infield')
    await form.getByRole('button', { name: 'Test' }).click()
    const [test] = await invokes(page, 'connection_test')
    expect((test.args.config as { url: string }).url).toBe('mongodb://typed@typed.mongo.example.com/app')
    expect((test.args.secrets as { dbPassword: string }).dbPassword).toBe('infield')
  })

  test('a MongoDB URL that needs TLS does not keep an SSL mode of "Disable"', async ({ page }) => {
    const form = await openForm(page)
    await form.getByRole('radio', { name: 'MongoDB' }).click()
    await form.locator('summary', { hasText: 'SSL' }).click()
    await field.sslMode(form).selectOption('disable')
    await importUrl(form, 'mongodb+srv://u:pw@cluster0.example.mongodb.net/app')
    await expect(field.sslMode(form)).toHaveValue('prefer')
  })

  test('a multi-line paste imports its first line only', async ({ page }) => {
    const form = await openForm(page)
    await pasteInto(page, field.host(form), 'postgres://env:pw@env.example.com:5433/first\nREDIS_URL=redis://cache:6379\n')
    await expect(field.host(form)).toHaveValue('env.example.com')
    await expect(field.database(form)).toHaveValue('first')
  })

  test('into the Password field is never taken for a URL', async ({ page }) => {
    const form = await openForm(page)
    await pasteInto(page, field.password(form), 'postgres://looks:like@a.url/but-is-a-password')
    await expect(field.password(form)).toHaveValue('postgres://looks:like@a.url/but-is-a-password')
    await expect(field.host(form)).toHaveValue('127.0.0.1')
  })
})

test.describe('Pasting a URL on the connection list', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  const search = (page: Page) => page.getByRole('searchbox', { name: /Search connections/ }).or(page.getByPlaceholder('Search connections…'))

  test('into the search box opens a new connection filled from it', async ({ page }) => {
    await pasteInto(page, search(page), 'postgres://listed:pw@list.example.com:5433/orders')
    const form = dialog(page, 'New connection')
    await expect(form).toBeVisible()
    await engineOn(form, 'PostgreSQL')
    await expect(field.host(form)).toHaveValue('list.example.com')
    await expect(field.port(form)).toHaveValue('5433')
    await expect(field.user(form)).toHaveValue('listed')
    await expect(field.password(form)).toHaveValue('pw')
    await expect(field.name(form)).toHaveValue('orders @ list.example.com')
    await expect(form.getByText('Filled from URL: PostgreSQL')).toBeVisible()
    await expect(search(page)).toHaveValue('')
  })

  test('the connection saves like any other and lands on the list', async ({ page }) => {
    await recordInvokes(page)
    await pasteInto(page, search(page), 'mysql://u:hunter2@saved.example.com/shop')
    const form = dialog(page, 'New connection')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(form).toHaveCount(0)
    await expect(card(page, 'shop @ saved.example.com')).toBeVisible()
    const [save] = await invokes(page, 'connection_save')
    expect(save.args.secrets).toMatchObject({ dbPassword: 'hunter2' })
    expect(JSON.stringify(save.args.config)).not.toContain('hunter2')
  })

  test('a SQLite URL opens the SQLite form', async ({ page }) => {
    await pasteInto(page, search(page), 'sqlite:///Users/me/listed.db')
    const form = dialog(page, 'New connection')
    await engineOn(form, 'SQLite')
    await expect(field.file(form)).toHaveValue('/Users/me/listed.db')
  })

  test('a URL the form cannot read is pasted into the search box as text', async ({ page }) => {
    await pasteInto(page, search(page), 'redis://cache.example.com')
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(search(page)).toHaveValue('redis://cache.example.com')
  })

  test('a pasted file name or ordinary search text filters the list', async ({ page }) => {
    await pasteInto(page, search(page), 'nasty.db')
    await expect(dialog(page, 'New connection')).toHaveCount(0)
    await expect(search(page)).toHaveValue('nasty.db')
    await expect(card(page, 'nasty.db')).toBeVisible()
  })
})

test.describe('Copy as URL', () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page)
  })

  const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText())

  async function copyUrl(page: Page, name: string) {
    await card(page, name).getByRole('button', { name: 'Actions' }).click()
    await page.getByRole('menuitem', { name: 'Copy as URL' }).click()
  }

  test('copies a Postgres connection without a password and says so', async ({ page }) => {
    await copyUrl(page, 'Local Postgres')
    await expect.poll(() => clipboard(page)).toBe('postgresql://tablesail@127.0.0.1:25432/tablesail')
    await expect(page.locator('.toast', { hasText: 'Connection URL copied' })).toContainText('password is not included')
  })

  test('copies MySQL, MariaDB, MongoDB and SQLite connections', async ({ page }) => {
    await copyUrl(page, 'Local MySQL')
    await expect.poll(() => clipboard(page)).toBe('mysql://tablesail@127.0.0.1:23306/tablesail')
    await copyUrl(page, 'MariaDB')
    await expect.poll(() => clipboard(page)).toBe('mariadb://tablesail@127.0.0.1:23307/tablesail')
    await copyUrl(page, 'Mongo')
    await expect.poll(() => clipboard(page)).toBe('mongodb://127.0.0.1:27117/tablesail')
    await copyUrl(page, 'nasty.db')
    await expect.poll(() => clipboard(page)).toBe('sqlite://~/data/nasty.db')
  })

  test('the menu item is next to Duplicate, before Delete', async ({ page }) => {
    await card(page, 'Local Postgres').click({ button: 'right' })
    const items = (await page.getByRole('menuitem').allTextContents()).map((t) => t.trim())
    expect(items).toEqual(['Open', 'Edit…', 'Duplicate', 'Copy as URL', 'Delete…'])
  })

  test('includes a non-default SSL mode, and never the password that was saved with it', async ({ page }) => {
    const form = await openForm(page)
    await field.name(form).fill('Secure one')
    await field.host(form).fill('secure.example.com')
    await field.password(form).fill('hunter2')
    await form.getByText('SSL', { exact: false }).first().click()
    await field.sslMode(form).selectOption('verifyFull')
    await form.getByRole('button', { name: 'Save' }).click()
    await expect(dialog(page, 'New connection')).toHaveCount(0)

    await copyUrl(page, 'Secure one')
    await expect.poll(() => clipboard(page)).toBe('postgresql://postgres@secure.example.com:5432/?sslmode=verify-full')
    expect(await clipboard(page)).not.toContain('hunter2')
  })

  test('what was copied imports back into a new connection form', async ({ page }) => {
    await copyUrl(page, 'Local MySQL')
    const url = await clipboard(page)
    const form = await openForm(page)
    await importUrl(form, url)
    await engineOn(form, 'MySQL')
    await expect(field.host(form)).toHaveValue('127.0.0.1')
    await expect(field.port(form)).toHaveValue('23306')
    await expect(field.user(form)).toHaveValue('tablesail')
    await expect(field.database(form)).toHaveValue('tablesail')
  })

  test('SQLite copy imports back to the same file', async ({ page }) => {
    await copyUrl(page, 'nasty.db')
    const url = await clipboard(page)
    const form = await openForm(page)
    await importUrl(form, url)
    await engineOn(form, 'SQLite')
    await expect(field.file(form)).toHaveValue('~/data/nasty.db')
  })
})
