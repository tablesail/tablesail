// Shared helpers for the unit tests (not part of the app bundle).

import { vi } from 'vitest'

/** An in-memory `Storage`, with `data` exposed for assertions. */
export class MemoryStorage implements Storage {
  data = new Map<string, string>()

  get length(): number {
    return this.data.size
  }
  clear(): void {
    this.data.clear()
  }
  getItem(key: string): string | null {
    return this.data.has(key) ? (this.data.get(key) as string) : null
  }
  key(index: number): string | null {
    return [...this.data.keys()][index] ?? null
  }
  removeItem(key: string): void {
    this.data.delete(key)
  }
  setItem(key: string, value: string): void {
    this.data.set(key, String(value))
  }
  /** Parsed JSON stored under `key`, or undefined. */
  json<T = unknown>(key: string): T | undefined {
    const raw = this.data.get(key)
    return raw === undefined ? undefined : (JSON.parse(raw) as T)
  }
}

/** Installs a fresh MemoryStorage as the global `localStorage`. */
export function installStorage(initial: Record<string, unknown> = {}): MemoryStorage {
  const storage = new MemoryStorage()
  for (const [k, v] of Object.entries(initial)) storage.setItem(k, typeof v === 'string' ? v : JSON.stringify(v))
  vi.stubGlobal('localStorage', storage)
  return storage
}

/**
 * Routes `invoke(command, args)` to handlers keyed by command name. Unhandled commands reject,
 * so a test never silently talks to a missing backend.
 */
export type InvokeHandlers = Record<string, (args: Record<string, unknown>) => unknown>

export function routeInvoke(handlers: InvokeHandlers) {
  return async (command: string, args: Record<string, unknown> = {}) => {
    const handler = handlers[command]
    if (!handler) throw new Error(`unmocked command: ${command}`)
    return handler(args)
  }
}

/** Lets pending promise callbacks run. */
export async function settle(times = 5) {
  for (let i = 0; i < times; i++) await Promise.resolve()
}
