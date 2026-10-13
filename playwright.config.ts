import { fileURLToPath } from 'node:url'
import { defineConfig } from '@playwright/test'

// E2E suite: drives the whole UI in a normal browser against the dev-only Tauri mock
// (src/dev/mockTauri.ts). Uses the installed Google Chrome, so no browsers are downloaded.
// Port 5181 is reserved for this suite (5173 and 5180 belong to other dev servers).
const PORT = 5181

export default defineConfig({
  testDir: 'e2e',
  outputDir: 'test-results',
  reporter: [['list']],
  fullyParallel: true,
  workers: 3,
  retries: 0,
  forbidOnly: !!process.env.CI,
  timeout: 30_000,
  expect: { timeout: 7_000 },
  use: {
    baseURL: `http://localhost:${PORT}`,
    channel: 'chrome',
    headless: true,
    launchOptions: {
      args: ['--disable-breakpad', '--disable-crash-reporter'],
      // macOS: see e2e/chrome-quiet.sh (without it the run hangs after the last test).
      ...(process.platform === 'darwin' ? { executablePath: fileURLToPath(new URL('./e2e/chrome-quiet.sh', import.meta.url)) } : {}),
    },
    viewport: { width: 1440, height: 900 },
    // Keep artifacts small: only failure screenshots, no traces or videos.
    trace: 'off',
    video: 'off',
    screenshot: 'only-on-failure',
    permissions: ['clipboard-read', 'clipboard-write'],
    colorScheme: 'light',
    locale: 'en-US',
  },
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: true,
    timeout: 60_000,
  },
})
