import { test, expect } from './fixtures'
import { MOD, card, gridCell, inspector, openConnection, sqlText, tabNamed, tabTitles } from './helpers'
import type { Page } from '@playwright/test'

const panel = (page: Page) => inspector(page)
const composer = (page: Page) => panel(page).getByRole('textbox', { name: 'Message the assistant' })
const send = (page: Page) => panel(page).getByRole('button', { name: 'Send' })
const stop = (page: Page) => panel(page).getByRole('button', { name: 'Stop' })
const thinking = (page: Page) => panel(page).getByRole('status').filter({ hasText: 'Thinking…' })

async function openAssistant(page: Page) {
  await page.getByRole('button', { name: 'AI assistant' }).click()
  await expect(panel(page).getByRole('tab', { name: 'Assistant' })).toHaveAttribute('aria-selected', 'true')
}

async function ask(page: Page, text: string) {
  await composer(page).fill(text)
  await composer(page).press('Enter')
}

const REPLY_SQL = 'SELECT id, email, name\nFROM public.customers\nORDER BY id DESC\nLIMIT 10;'

test.describe('assistant: empty state', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openAssistant(page)
  })

  test('explains itself and offers three suggestion chips built from the schema', async ({ page }) => {
    await expect(panel(page).getByText('Ask about your data')).toBeVisible()
    await expect(panel(page).getByText('Describe what you need and get SQL for this connection.')).toBeVisible()
    const chips = panel(page).locator('.chip')
    await expect(chips).toHaveText([
      'Show the 10 most recent rows of all_types',
      'Which tables reference all_types?',
      'Write a query to count rows per day',
    ])
  })

  test('the composer, send button and model label are there; Send is disabled until there is text', async ({ page }) => {
    await expect(composer(page)).toHaveAttribute('placeholder', 'Ask about your data…')
    await expect(send(page)).toBeDisabled()
    await composer(page).fill('hello')
    await expect(send(page)).toBeEnabled()
    await composer(page).fill('   ')
    await expect(send(page)).toBeDisabled()
    await expect(panel(page).locator('.model')).toHaveText('claude-sonnet-5-5')
    await expect(panel(page).getByRole('button', { name: 'New chat' })).toBeDisabled()
  })

  test('MongoDB talks about collections and documents', async ({ page }) => {
    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await card(page, 'Mongo').getByRole('button', { name: 'Open', exact: true }).click()
    await expect(page.getByRole('button', { name: /MongoDB 8\.2\.1/ })).toBeVisible()
    await expect(panel(page).locator('.chip').nth(0)).toContainText('documents of all_types')
    await expect(panel(page).locator('.chip').nth(1)).toContainText('Which collections reference')
  })
})

test.describe('assistant: conversation', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openAssistant(page)
  })

  test('clicking a chip sends it and shows the question, a thinking indicator and then the canned reply', async ({ page }) => {
    await panel(page).locator('.chip').first().click()
    await expect(panel(page).locator('.user .bubble')).toHaveText('Show the 10 most recent rows of all_types')
    await expect(thinking(page)).toBeVisible()
    await expect(panel(page).getByText('Ask about your data')).toHaveCount(0)

    await expect(thinking(page)).toHaveCount(0)
    const reply = panel(page).locator('.reply')
    await expect(reply).toBeVisible()
    await expect(reply).toContainText('Here is a query for that.')
    await expect(reply.locator('code').first()).toHaveText('customers')
    await expect(reply.locator('.codebox pre')).toContainText('FROM public.customers')
    await expect(reply.locator('.codebox pre')).toContainText('ORDER BY id DESC')
    await expect(panel(page).getByText('Change LIMIT to see more rows', { exact: false })).toBeVisible()
  })

  test('the SQL block is syntax highlighted', async ({ page }) => {
    await ask(page, 'recent customers')
    await expect(panel(page).locator('.codebox')).toBeVisible()
    await expect(panel(page).locator('.codebox code .k').first()).toBeVisible() // keywords get a class
  })

  test('typing a question and pressing Enter sends it and clears the box', async ({ page }) => {
    await composer(page).fill('who are my best customers?')
    await composer(page).press('Enter')
    await expect(composer(page)).toHaveValue('')
    await expect(panel(page).locator('.user .bubble')).toHaveText('who are my best customers?')
    await expect(panel(page).locator('.reply')).toBeVisible()
  })

  test('Shift+Enter adds a new line instead of sending', async ({ page }) => {
    await composer(page).fill('line one')
    await composer(page).press('Shift+Enter')
    await composer(page).pressSequentially('line two')
    await expect(composer(page)).toHaveValue('line one\nline two')
    await expect(panel(page).locator('.user')).toHaveCount(0)
  })

  test('the Send button sends too', async ({ page }) => {
    await composer(page).fill('count orders')
    await send(page).click()
    await expect(panel(page).locator('.user .bubble')).toHaveText('count orders')
  })

  test('the composer grows with its text up to a limit', async ({ page }) => {
    const before = (await composer(page).boundingBox())!.height
    await composer(page).fill(Array.from({ length: 12 }, (_, i) => `line ${i}`).join('\n'))
    await expect.poll(async () => (await composer(page).boundingBox())!.height).toBeGreaterThan(before + 20)
    expect((await composer(page).boundingBox())!.height).toBeLessThan(140)
  })

  test('while a reply is pending Send is disabled and Enter does not send a second question', async ({ page }) => {
    await ask(page, 'first')
    await expect(thinking(page)).toBeVisible()
    await composer(page).fill('second')
    await expect(send(page)).toBeDisabled()
    await composer(page).press('Enter')
    await expect(panel(page).locator('.user')).toHaveCount(1)
    await expect(thinking(page)).toHaveCount(0)
    await expect(composer(page)).toHaveValue('second') // the draft is kept
  })

  test('Stop cancels the pending reply; the question stays and a new one can be asked', async ({ page }) => {
    await ask(page, 'something slow')
    await expect(thinking(page)).toBeVisible()
    await stop(page).click()
    await expect(thinking(page)).toHaveCount(0)
    await expect(panel(page).locator('.user .bubble')).toHaveText('something slow')
    // The cancelled request must not produce a reply later.
    await expect(panel(page).locator('.reply')).toHaveCount(0)
    await expect(panel(page).getByRole('alert')).toHaveCount(0)

    await ask(page, 'try again')
    await expect(panel(page).locator('.reply')).toBeVisible()
    await expect(panel(page).locator('.user')).toHaveCount(2)
  })

  test('New chat forgets the conversation and returns to the suggestions', async ({ page }) => {
    await ask(page, 'hello')
    await expect(panel(page).locator('.reply')).toBeVisible()
    await panel(page).getByRole('button', { name: 'New chat' }).click()
    await expect(panel(page).locator('.user')).toHaveCount(0)
    await expect(panel(page).getByText('Ask about your data')).toBeVisible()
    await expect(composer(page)).toBeFocused()
  })

  test('the chat keeps its messages when the panel is closed and reopened', async ({ page }) => {
    await ask(page, 'remember me')
    await expect(panel(page).locator('.reply')).toBeVisible()
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expect(panel(page)).toHaveCount(0)
    await page.getByRole('button', { name: 'AI assistant' }).click()
    await expect(panel(page).locator('.user .bubble')).toHaveText('remember me')
    await expect(panel(page).locator('.reply')).toBeVisible()
  })

  test('an unsent draft is kept while switching between Details and Assistant', async ({ page }) => {
    await composer(page).fill('half written')
    await panel(page).getByRole('tab', { name: 'Details' }).click()
    await panel(page).getByRole('tab', { name: 'Assistant' }).click()
    await expect(composer(page)).toHaveValue('half written')
  })

  test('each connection has its own conversation', async ({ page }) => {
    await ask(page, 'postgres question')
    await expect(panel(page).locator('.reply')).toBeVisible()
    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await card(page, 'Local MySQL').dblclick()
    await expect(page.getByRole('button', { name: /^MySQL 8\.4\.6/ })).toBeVisible()
    await expect(panel(page).locator('.user')).toHaveCount(0)
    await expect(panel(page).getByText('Ask about your data')).toBeVisible()

    await page.getByRole('button', { name: 'Connections', exact: true }).click()
    await card(page, 'Local Postgres').dblclick()
    await expect(panel(page).locator('.user .bubble')).toHaveText('postgres question')
  })

  test('the Assistant tab shows no row details and the Details tab no chat', async ({ page }) => {
    await expect(panel(page).getByPlaceholder('Search for field…')).toHaveCount(0)
    await panel(page).getByRole('tab', { name: 'Details' }).click()
    await expect(composer(page)).toHaveCount(0)
  })
})

test.describe('assistant: using the SQL', () => {
  test.beforeEach(async ({ page }) => {
    await openConnection(page)
    await openAssistant(page)
    await ask(page, 'recent customers')
    await expect(panel(page).locator('.codebox')).toBeVisible()
  })

  test('Open in editor creates a query tab holding exactly that SQL, without running it', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Open in editor' }).click()
    await expect(tabNamed(page, 'Query 1')).toHaveAttribute('aria-selected', 'true')
    expect(await sqlText(page)).toBe(REPLY_SQL)
    await expect(page.getByText('Run a query to see results here.')).toBeVisible()
  })

  test('Run opens a query tab and runs the SQL straight away', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Run', exact: true }).click()
    await expect(tabNamed(page, 'Query 1')).toBeVisible()
    expect(await sqlText(page)).toBe(REPLY_SQL)
    await expect(page.getByRole('tab', { name: 'Result 1' })).toBeVisible()
    await expect(gridCell(page, 0, 2)).toHaveText('Alice')
  })

  test('opening the same reply twice creates two tabs', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Open in editor' }).click()
    await panel(page).getByRole('button', { name: 'Open in editor' }).click()
    expect(await tabTitles(page)).toEqual(['Query 1', 'Query 2'])
  })

  test('Copy puts the SQL on the clipboard and says so', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Copy', exact: true }).click()
    await expect(page.getByText('Copied', { exact: true })).toBeVisible()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(REPLY_SQL)
  })

  test('the panel stays open after using a reply so the conversation can continue', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Open in editor' }).click()
    await expect(panel(page)).toBeVisible()
    await expect(composer(page)).toBeVisible()
  })

  test('a result selected in the Run tab shows up in Details', async ({ page }) => {
    await panel(page).getByRole('button', { name: 'Run', exact: true }).click()
    await gridCell(page, 0, 1).click()
    await panel(page).getByRole('tab', { name: 'Details' }).click()
    await expect(panel(page).locator('.field').filter({ hasText: 'email' })).toContainText('a@example.com')
  })
})

test.describe('assistant: no API key', () => {
  async function removeKey(page: Page) {
    await page.getByRole('button', { name: 'Menu' }).click()
    await page.getByRole('menuitem', { name: /Settings…/ }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await dlg.getByRole('button', { name: 'Remove' }).click()
    await expect(dlg.getByLabel('Anthropic API key')).toHaveAttribute('placeholder', 'sk-ant-…')
    await dlg.getByRole('button', { name: 'Done' }).click()
  }

  test('without a key the panel says so and links to Settings', async ({ page }) => {
    await openConnection(page)
    await removeKey(page)
    await openAssistant(page)
    await expect(panel(page).getByText('Add your Anthropic API key in Settings to use the assistant.')).toBeVisible()
    await expect(composer(page)).toHaveCount(0)
    await panel(page).getByRole('button', { name: 'Open Settings' }).click()
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible()
  })

  test('saving a key from that Settings dialog brings the chat back without reopening the panel', async ({ page }) => {
    await openConnection(page)
    await removeKey(page)
    await openAssistant(page)
    await panel(page).getByRole('button', { name: 'Open Settings' }).click()
    const dlg = page.getByRole('dialog', { name: 'Settings' })
    await dlg.getByLabel('Anthropic API key').fill('sk-ant-new')
    await dlg.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(page.getByText('API key saved in Keychain')).toBeVisible()
    await dlg.getByRole('button', { name: 'Done' }).click()
    await expect(composer(page)).toBeVisible()
    await expect(panel(page).getByText('Add your Anthropic API key')).toHaveCount(0)
  })

  test('removing the key while a chat is open turns the panel into the no-key state', async ({ page }) => {
    await openConnection(page)
    await openAssistant(page)
    await expect(composer(page)).toBeVisible()
    await removeKey(page)
    await expect(panel(page).getByText('Add your Anthropic API key in Settings to use the assistant.')).toBeVisible()
  })
})

test.describe('assistant: keyboard', () => {
  test('⌘↩ is not hijacked by the app while typing in the composer', async ({ page }) => {
    await openConnection(page)
    await openAssistant(page)
    await composer(page).fill('x')
    await composer(page).press(`${MOD}+Enter`)
    // No query tab is created and nothing breaks.
    expect(await tabTitles(page)).toEqual([])
  })
})
