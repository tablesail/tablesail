export type ThemeMode = 'light' | 'dark' | 'system'

const KEY = 'tablesoft.theme'

function read(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY)
    if (v === 'light' || v === 'dark' || v === 'system') return v
  } catch {
    // storage unavailable: fall back to system
  }
  return 'system'
}

const media = typeof window !== 'undefined' ? window.matchMedia('(prefers-color-scheme: dark)') : null

class ThemeStore {
  mode = $state<ThemeMode>(read())
  /** The system's preference, kept live. */
  private systemDark = $state(media?.matches ?? false)

  /** Applies the stored mode to <html>. Call once at startup. */
  init() {
    media?.addEventListener('change', (e) => {
      this.systemDark = e.matches
      this.apply()
    })
    this.apply()
  }

  set(mode: ThemeMode) {
    this.mode = mode
    try {
      localStorage.setItem(KEY, mode)
    } catch {
      // ignore
    }
    this.apply()
  }

  /** Whether the dark palette is currently in effect. */
  get isDark(): boolean {
    return this.mode === 'system' ? this.systemDark : this.mode === 'dark'
  }

  /** Shortcut action: flip between light and dark, leaving system mode. */
  toggle() {
    this.set(this.isDark ? 'light' : 'dark')
  }

  // Always resolve to an explicit light/dark attribute so theme-specific styles apply
  // in system mode too (the CSS media query only covers the first paint).
  private apply() {
    document.documentElement.setAttribute('data-theme', this.isDark ? 'dark' : 'light')
  }
}

export const theme = new ThemeStore()
