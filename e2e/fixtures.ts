import { test as base, expect } from '@playwright/test'

/**
 * Same as Playwright's `test`, but every test also fails on an uncaught page error
 * (a thrown exception in app code), which a UI assertion alone would not notice.
 */
export const test = base.extend({
  page: async ({ page }, use) => {
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    await use(page)
    expect(errors, 'uncaught errors in the page').toEqual([])
  },
})

export { expect }
