import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

// Separate from vite.config.ts so the Tauri dev-server settings don't leak into tests.
//
// The svelte plugin compiles runes in `*.svelte.ts` stores. vitest runs modules through Vite's
// SSR pipeline, which would make Svelte emit server code (no reactivity, `$effect` removed), so
// we force client output and the `browser` export condition: stores then behave exactly as in
// the app, and `flushSync()` from 'svelte' runs pending effects.
export default defineConfig({
  plugins: [svelte({ dynamicCompileOptions: () => ({ generate: 'client' }) })],
  ssr: { resolve: { conditions: ['browser'] } },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    restoreMocks: true,
  },
})
