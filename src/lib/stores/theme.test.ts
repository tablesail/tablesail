import { afterEach, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'

const KEY = 'tablesoft.theme'

interface Env {
  theme: (typeof import('./theme.svelte'))['theme']
  /** Value of the data-theme attribute on <html>, or null when never set. */
  attr: () => string | null
  setAttribute: ReturnType<typeof vi.fn>
  /** Simulates the OS switching between light and dark. */
  osChange: (dark: boolean) => void
  storage: ReturnType<typeof installStorage>
}

async function setup(opts: { stored?: string; osDark?: boolean; window?: boolean } = {}): Promise<Env> {
  vi.resetModules()
  const storage = installStorage(opts.stored === undefined ? {} : { [KEY]: opts.stored })
  let attr: string | null = null
  const setAttribute = vi.fn((name: string, value: string) => {
    if (name === 'data-theme') attr = value
  })
  vi.stubGlobal('document', { documentElement: { setAttribute } })
  const listeners: ((e: { matches: boolean }) => void)[] = []
  if (opts.window !== false) {
    vi.stubGlobal('window', {
      matchMedia: (query: string) => {
        expect(query).toBe('(prefers-color-scheme: dark)')
        return {
          matches: opts.osDark ?? false,
          addEventListener: (_type: string, cb: (e: { matches: boolean }) => void) => listeners.push(cb),
        }
      },
    })
  } else {
    vi.stubGlobal('window', undefined)
  }
  const { theme } = await import('./theme.svelte')
  return {
    theme,
    attr: () => attr,
    setAttribute,
    osChange: (dark) => listeners.forEach((cb) => cb({ matches: dark })),
    storage,
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('theme: initial mode', () => {
  it('defaults to system', async () => {
    expect((await setup()).theme.mode).toBe('system')
  })

  it.each(['light', 'dark', 'system'] as const)('restores the stored mode %s', async (mode) => {
    expect((await setup({ stored: mode })).theme.mode).toBe(mode)
  })

  it('ignores an unknown stored value', async () => {
    expect((await setup({ stored: 'solarized' })).theme.mode).toBe('system')
    expect((await setup({ stored: '' })).theme.mode).toBe('system')
  })

  it('falls back to system when storage throws', async () => {
    vi.resetModules()
    vi.stubGlobal('localStorage', {
      getItem() {
        throw new Error('blocked')
      },
    })
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false, addEventListener() {} }) })
    const { theme } = await import('./theme.svelte')
    expect(theme.mode).toBe('system')
  })

  it('does not touch the document until init()', async () => {
    const { setAttribute } = await setup()
    expect(setAttribute).not.toHaveBeenCalled()
  })
})

describe('theme.init', () => {
  it('system mode follows a dark OS', async () => {
    const { theme, attr } = await setup({ osDark: true })
    theme.init()
    expect(attr()).toBe('dark')
    expect(theme.isDark).toBe(true)
  })

  it('system mode follows a light OS', async () => {
    const { theme, attr } = await setup({ osDark: false })
    theme.init()
    expect(attr()).toBe('light')
    expect(theme.isDark).toBe(false)
  })

  it('a stored explicit mode beats the OS', async () => {
    const dark = await setup({ stored: 'dark', osDark: false })
    dark.theme.init()
    expect(dark.attr()).toBe('dark')
    const light = await setup({ stored: 'light', osDark: true })
    light.theme.init()
    expect(light.attr()).toBe('light')
  })

  it('always writes an explicit light/dark attribute, never "system"', async () => {
    const { theme, setAttribute } = await setup({ stored: 'system', osDark: true })
    theme.init()
    expect(setAttribute).toHaveBeenCalledWith('data-theme', 'dark')
    expect(setAttribute.mock.calls.every(([n, v]) => n === 'data-theme' && (v === 'light' || v === 'dark'))).toBe(true)
  })

  it('works without window.matchMedia (treated as a light OS)', async () => {
    const { theme, attr } = await setup({ window: false })
    expect(() => theme.init()).not.toThrow()
    expect(attr()).toBe('light')
  })
})

describe('theme.set', () => {
  it('switches mode, persists it and updates <html>', async () => {
    const { theme, attr, storage } = await setup({ osDark: false })
    theme.init()
    theme.set('dark')
    expect(theme.mode).toBe('dark')
    expect(theme.isDark).toBe(true)
    expect(attr()).toBe('dark')
    expect(storage.getItem(KEY)).toBe('dark')
    theme.set('light')
    expect(attr()).toBe('light')
    expect(storage.getItem(KEY)).toBe('light')
  })

  it('switching back to system picks up the current OS preference', async () => {
    const { theme, attr } = await setup({ osDark: true })
    theme.init()
    theme.set('light')
    expect(attr()).toBe('light')
    theme.set('system')
    expect(attr()).toBe('dark')
  })

  it('still applies the theme when storage is unavailable', async () => {
    const env = await setup()
    vi.stubGlobal('localStorage', {
      setItem() {
        throw new Error('quota')
      },
    })
    expect(() => env.theme.set('dark')).not.toThrow()
    expect(env.attr()).toBe('dark')
  })

  it('a persisted mode survives a restart', async () => {
    const first = await setup()
    first.theme.set('dark')
    vi.resetModules()
    vi.stubGlobal('localStorage', first.storage)
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false, addEventListener() {} }) })
    const { theme } = await import('./theme.svelte')
    expect(theme.mode).toBe('dark')
  })
})

describe('theme.toggle', () => {
  it('dark -> light', async () => {
    const { theme } = await setup({ stored: 'dark' })
    theme.init()
    theme.toggle()
    expect(theme.mode).toBe('light')
  })

  it('light -> dark', async () => {
    const { theme } = await setup({ stored: 'light' })
    theme.init()
    theme.toggle()
    expect(theme.mode).toBe('dark')
  })

  it('leaves system mode, flipping what is currently shown', async () => {
    const dark = await setup({ osDark: true })
    dark.theme.init()
    dark.theme.toggle()
    expect(dark.theme.mode).toBe('light')
    expect(dark.attr()).toBe('light')

    const light = await setup({ osDark: false })
    light.theme.init()
    light.theme.toggle()
    expect(light.theme.mode).toBe('dark')
    expect(light.attr()).toBe('dark')
  })

  it('twice returns to the starting look', async () => {
    const { theme, attr } = await setup({ stored: 'dark' })
    theme.init()
    theme.toggle()
    theme.toggle()
    expect(attr()).toBe('dark')
  })

  it('persists the new mode', async () => {
    const { theme, storage } = await setup({ stored: 'dark' })
    theme.init()
    theme.toggle()
    expect(storage.getItem(KEY)).toBe('light')
  })
})

describe('theme: following the OS', () => {
  it('in system mode a change of OS preference re-themes the page', async () => {
    const { theme, attr, osChange } = await setup({ osDark: false })
    theme.init()
    expect(attr()).toBe('light')
    osChange(true)
    expect(attr()).toBe('dark')
    expect(theme.isDark).toBe(true)
    osChange(false)
    expect(attr()).toBe('light')
  })

  it('in an explicit mode an OS change is ignored', async () => {
    const { theme, attr, osChange } = await setup({ stored: 'light', osDark: false })
    theme.init()
    osChange(true)
    expect(attr()).toBe('light')
    expect(theme.isDark).toBe(false)
  })

  it('a remembered OS change applies once system mode is chosen', async () => {
    const { theme, attr, osChange } = await setup({ stored: 'light', osDark: false })
    theme.init()
    osChange(true)
    theme.set('system')
    expect(attr()).toBe('dark')
  })

  it('does not listen before init()', async () => {
    const { theme, attr, osChange } = await setup({ osDark: false })
    osChange(true)
    expect(attr()).toBeNull()
    expect(theme.isDark).toBe(false)
  })
})
