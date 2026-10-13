import type { BrowseRequest, ExportSource, SessionInfo, TableRef } from '../types'

export interface Toast {
  id: number
  kind: 'info' | 'error' | 'success'
  text: string
}

export interface ExportRequest {
  session: SessionInfo
  source: ExportSource
  /** used for the default file name */
  name: string
}

export interface ImportRequest {
  session: SessionInfo
  table: TableRef
}

class UiStore {
  toasts = $state<Toast[]>([])
  exportRequest = $state<ExportRequest | null>(null)
  importRequest = $state<ImportRequest | null>(null)
  quickOpen = $state(false)
  shortcutsHelp = $state(false)
  about = $state(false)

  private nextToast = 1

  toast(text: string, kind: Toast['kind'] = 'info', ms = 4000) {
    const id = this.nextToast++
    this.toasts = [...this.toasts, { id, kind, text }]
    setTimeout(() => this.dismiss(id), kind === 'error' ? ms * 2 : ms)
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id)
  }

  exportTable(session: SessionInfo, request: BrowseRequest) {
    this.exportRequest = { session, source: { kind: 'table', request }, name: request.table.name }
  }

  exportQuery(session: SessionInfo, database: string | null, sql: string) {
    this.exportRequest = { session, source: { kind: 'query', database, sql }, name: 'query' }
  }

  importInto(session: SessionInfo, table: TableRef) {
    this.importRequest = { session, table }
  }
}

export const ui = new UiStore()
