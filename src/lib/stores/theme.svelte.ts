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

class ThemeStore {
  mode = $state<ThemeMode>(read())

  /** Applies the stored mode to <html>. Call once at startup. */
  init() {
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
    if (this.mode === 'system') return window.matchMedia('(prefers-color-scheme: dark)').matches
    return this.mode === 'dark'
  }

  /** Shortcut action: flip between light and dark, leaving system mode. */
  toggle() {
    this.set(this.isDark ? 'light' : 'dark')
  }

  private apply() {
    const root = document.documentElement
    if (this.mode === 'system') root.removeAttribute('data-theme')
    else root.setAttribute('data-theme', this.mode)
  }
}

export const theme = new ThemeStore()
