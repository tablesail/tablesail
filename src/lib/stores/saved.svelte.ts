// Named queries the user saved from the editor, kept on this machine.

import { load, save } from './persist'

export interface SavedQuery {
  id: string
  name: string
  sql: string
  /** Connection it was saved from; opens there by default. */
  connectionId: string | null
  database: string | null
  savedAt: number
}

const KEY = 'tablesoft.savedQueries'

class SavedStore {
  list = $state<SavedQuery[]>(load<SavedQuery[]>(KEY, []))
  private seq = 0

  add(q: Omit<SavedQuery, 'id' | 'savedAt'>): SavedQuery {
    // Time alone is not unique: two saves in the same millisecond must not share an id.
    const id = `s${Date.now().toString(36)}${(this.seq++).toString(36)}${Math.random().toString(36).slice(2, 6)}`
    const item: SavedQuery = { ...q, id, savedAt: Date.now() }
    this.list = [item, ...this.list]
    save(KEY, this.list)
    return item
  }

  rename(id: string, name: string) {
    this.list = this.list.map((q) => (q.id === id ? { ...q, name } : q))
    save(KEY, this.list)
  }

  remove(id: string) {
    this.list = this.list.filter((q) => q.id !== id)
    save(KEY, this.list)
  }
}

export const saved = new SavedStore()
