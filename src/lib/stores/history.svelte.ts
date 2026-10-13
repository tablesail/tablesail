// Every statement run from a query tab, newest first, kept on this machine.

import { load, save } from './persist'

export interface HistoryEntry {
  id: number
  sql: string
  connectionId: string
  connectionName: string
  database: string
  /** Unix ms */
  at: number
  elapsedMs: number | null
  ok: boolean
}

const KEY = 'tablesoft.history'
const MAX = 500

class HistoryStore {
  entries = $state<HistoryEntry[]>(load<HistoryEntry[]>(KEY, []))

  add(entry: Omit<HistoryEntry, 'id' | 'at'>) {
    const sql = entry.sql.trim()
    if (!sql) return
    const next: HistoryEntry = { ...entry, sql, id: Date.now() + Math.random(), at: Date.now() }
    this.entries = [next, ...this.entries].slice(0, MAX)
    save(KEY, this.entries)
  }

  remove(id: number) {
    this.entries = this.entries.filter((e) => e.id !== id)
    save(KEY, this.entries)
  }

  clear() {
    this.entries = []
    save(KEY, this.entries)
  }
}

export const history = new HistoryStore()
