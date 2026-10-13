import { api, errorMessage, isCoreError } from '../api'
import type { Secrets, SchemaSnapshot, SessionInfo } from '../types'
import { tabs } from './tabs.svelte'

export interface SchemaEntry {
  snapshot: SchemaSnapshot | null
  loading: boolean
  error: string | null
}

const EMPTY: SchemaEntry = { snapshot: null, loading: false, error: null }

/** True when a failed open looks like a wrong or missing password. */
export function isAuthError(e: unknown): boolean {
  const msg = errorMessage(e).toLowerCase()
  if (isCoreError(e) && (e.kind === 'ssh' || e.kind === 'connection' || e.kind === 'database' || e.kind === 'other')) {
    return /password|authenticat|access denied|permission denied|passphrase|credentials/.test(msg)
  }
  return /password|authenticat|access denied/.test(msg)
}

class SessionsStore {
  list = $state<SessionInfo[]>([])
  activeId = $state<string | null>(null)
  /** 'connections' shows the welcome screen; sessions stay open behind it. */
  view = $state<'connections' | 'workspace'>('connections')
  currentDb = $state<Record<string, string>>({})
  schemas = $state.raw<Record<string, SchemaEntry>>({})

  get active(): SessionInfo | null {
    return this.list.find((s) => s.sessionId === this.activeId) ?? null
  }

  byConnection(connectionId: string): SessionInfo | undefined {
    return this.list.find((s) => s.config.id === connectionId)
  }

  database(sessionId: string): string {
    return this.currentDb[sessionId] ?? ''
  }

  schemaEntry(sessionId: string | null, database: string): SchemaEntry {
    if (!sessionId) return EMPTY
    return this.schemas[`${sessionId}\u0000${database}`] ?? EMPTY
  }

  async open(connectionId: string, secrets: Secrets | null = null): Promise<SessionInfo> {
    const info = await api.connectionOpen(connectionId, secrets)
    const existing = this.list.findIndex((s) => s.sessionId === info.sessionId)
    if (existing >= 0) this.list[existing] = info
    else this.list.push(info)
    if (!this.currentDb[info.sessionId]) {
      const wanted = info.config.database
      this.currentDb[info.sessionId] = info.databases.includes(wanted) ? wanted : (info.databases[0] ?? wanted)
    }
    this.activate(info.sessionId)
    void this.ensureSchema(info.sessionId, this.currentDb[info.sessionId])
    return info
  }

  activate(sessionId: string) {
    this.activeId = sessionId
    this.view = 'workspace'
  }

  async close(sessionId: string) {
    tabs.removeSession(sessionId)
    this.list = this.list.filter((s) => s.sessionId !== sessionId)
    delete this.currentDb[sessionId]
    if (this.activeId === sessionId) {
      this.activeId = this.list[0]?.sessionId ?? null
      if (!this.activeId) this.view = 'connections'
    }
    try {
      await api.connectionClose(sessionId)
    } catch {
      // the session is gone from the UI either way
    }
  }

  setDatabase(sessionId: string, database: string) {
    this.currentDb[sessionId] = database
    const entry = this.schemaEntry(sessionId, database)
    if (!entry.snapshot) void this.ensureSchema(sessionId, database)
  }

  private setEntry(sessionId: string, database: string, entry: SchemaEntry) {
    this.schemas = { ...this.schemas, [`${sessionId}\u0000${database}`]: entry }
  }

  /** Cached snapshot first for an instant sidebar, then a background refresh. */
  async ensureSchema(sessionId: string, database: string) {
    const prev = this.schemaEntry(sessionId, database)
    if (prev.loading) return
    this.setEntry(sessionId, database, { snapshot: prev.snapshot, loading: true, error: null })
    try {
      const cached = await api.schemaSnapshot(sessionId, database, false)
      if (!this.list.some((s) => s.sessionId === sessionId)) return
      this.setEntry(sessionId, database, { snapshot: cached, loading: true, error: null })
    } catch {
      // no cache yet; the refresh below reports real errors
    }
    await this.refreshSchema(sessionId, database)
  }

  async refreshSchema(sessionId: string, database: string) {
    const prev = this.schemaEntry(sessionId, database)
    this.setEntry(sessionId, database, { snapshot: prev.snapshot, loading: true, error: null })
    try {
      const fresh = await api.schemaSnapshot(sessionId, database, true)
      if (!this.list.some((s) => s.sessionId === sessionId)) return
      this.setEntry(sessionId, database, { snapshot: fresh, loading: false, error: null })
    } catch (e) {
      const cur = this.schemaEntry(sessionId, database)
      this.setEntry(sessionId, database, { snapshot: cur.snapshot, loading: false, error: errorMessage(e) })
    }
  }
}

export const sessions = new SessionsStore()
