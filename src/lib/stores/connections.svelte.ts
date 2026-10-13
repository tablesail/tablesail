import { api, errorMessage } from '../api'
import type { ConnectionConfig, Secrets } from '../types'

class ConnectionsStore {
  list = $state<ConnectionConfig[]>([])
  loaded = $state(false)
  error = $state<string | null>(null)

  async load() {
    try {
      this.list = await api.connectionsList()
      this.error = null
    } catch (e) {
      this.error = errorMessage(e)
    } finally {
      this.loaded = true
    }
  }

  async save(config: ConnectionConfig, secrets: Secrets | null): Promise<ConnectionConfig> {
    const saved = await api.connectionSave(config, secrets)
    const i = this.list.findIndex((c) => c.id === saved.id)
    if (i >= 0) this.list[i] = saved
    else this.list.push(saved)
    return saved
  }

  async remove(id: string) {
    await api.connectionDelete(id)
    this.list = this.list.filter((c) => c.id !== id)
  }

  /** Copies the settings (not the secrets, which live in the keychain under the old id). */
  async duplicate(config: ConnectionConfig): Promise<ConnectionConfig> {
    const copy: ConnectionConfig = { ...structuredClone($state.snapshot(config)), id: '', name: `${config.name} copy` }
    return this.save(copy, null)
  }
}

export const connections = new ConnectionsStore()
