// AI assistant state: one conversation per open connection, the saved-key flag and the
// chosen model. Conversations live in memory only; they survive tab switches, not restarts.

import { api, errorMessage, isCoreError } from '../api'
import type { SchemaSnapshot, SessionInfo, TableInfo } from '../types'

export const DEFAULT_MODEL = 'claude-sonnet-5-5'

export const MODELS: { id: string; label: string }[] = [
  { id: 'claude-sonnet-5-5', label: 'Claude Sonnet 5.5' },
  { id: 'claude-opus-5-5', label: 'Claude Opus 5.5' },
  { id: 'claude-haiku-5-5', label: 'Claude Haiku 5.5' },
]

const MODEL_KEY = 'tablesoft.assistant.model'

function readModel(): string {
  try {
    const v = localStorage.getItem(MODEL_KEY)
    if (v) return v
  } catch {
    // storage unavailable: use the default
  }
  return DEFAULT_MODEL
}

export interface ChatMessage {
  id: number
  role: 'user' | 'assistant'
  content: string
}

export interface Chat {
  messages: ChatMessage[]
  /** Set while a reply is pending; also the handle for cancelling it. */
  requestId: string | null
  error: string | null
}

const EMPTY_CHAT: Chat = { messages: [], requestId: null, error: null }

/** What the model is told about the schema is capped at about this many characters. */
const CONTEXT_CAP = 30_000

const SKIPPED_KINDS = new Set<TableInfo['kind']>(['sequence', 'system'])

function qualified(t: Pick<TableInfo, 'schema' | 'name'>): string {
  return t.schema ? `${t.schema}.${t.name}` : t.name
}

function usableTables(snapshot: SchemaSnapshot): TableInfo[] {
  return Object.values(snapshot.schemas)
    .flat()
    .filter((t) => !SKIPPED_KINDS.has(t.kind))
}

/** `public.users(id, email, created_at)`, one table per line, capped with a note. */
export function buildSchemaContext(snapshot: SchemaSnapshot | null): string {
  if (!snapshot) return ''
  const tables = usableTables(snapshot)
  const lines: string[] = []
  let size = 0
  for (const t of tables) {
    const name = qualified(t)
    const cols = snapshot.columns[name] ?? snapshot.columns[t.name]
    let line = cols ? `${name}(${cols.join(', ')})` : name
    if (t.kind === 'view') line += ' -- view'
    else if (t.kind === 'materializedView') line += ' -- materialized view'
    if (size + line.length + 1 > CONTEXT_CAP) {
      lines.push(`-- ${tables.length - lines.length} more tables not listed (schema too large)`)
      break
    }
    lines.push(line)
    size += line.length + 1
  }
  return lines.join('\n')
}

/** The table the suggestion chips talk about: the first real table, public schema first. */
export function firstTableName(snapshot: SchemaSnapshot | null): string | null {
  if (!snapshot) return null
  const tables = usableTables(snapshot).filter((t) => t.kind === 'table' || t.kind === 'collection')
  const pick = tables.find((t) => t.schema === 'public') ?? tables[0]
  if (!pick) return null
  return pick.schema && pick.schema !== 'public' ? `${pick.schema}.${pick.name}` : pick.name
}

function newRequestId(): string {
  try {
    return crypto.randomUUID()
  } catch {
    return `req-${Date.now()}-${Math.random().toString(36).slice(2)}`
  }
}

class AssistantStore {
  chats = $state<Record<string, Chat>>({})
  /** Unsent composer text per connection. */
  drafts = $state<Record<string, string>>({})
  /** Whether an API key is available; null until first checked. */
  hasKey = $state<boolean | null>(null)
  model = $state(readModel())

  private nextId = 1

  chat(sessionId: string): Chat {
    return this.chats[sessionId] ?? EMPTY_CHAT
  }

  draft(sessionId: string): string {
    return this.drafts[sessionId] ?? ''
  }

  setDraft(sessionId: string, text: string) {
    this.drafts[sessionId] = text
  }

  setModel(id: string) {
    this.model = id
    try {
      localStorage.setItem(MODEL_KEY, id)
    } catch {
      // ignore: persistence is a convenience
    }
  }

  async refreshKey() {
    try {
      this.hasKey = await api.assistantKeyStatus()
    } catch {
      this.hasKey = false
    }
  }

  /** Stores the key in the OS keychain (an empty string removes it). Throws on failure. */
  async setKey(key: string) {
    await api.assistantKeySet(key)
    await this.refreshKey()
  }

  /** Adds the question and asks for a reply. `schemaContext` is read by the caller at send time. */
  send(session: SessionInfo, database: string, schemaContext: string, text: string) {
    const content = text.trim()
    const id = session.sessionId
    if (!content || this.chat(id).requestId) return
    const messages = [...this.chat(id).messages, { id: this.nextId++, role: 'user' as const, content }]
    this.chats[id] = { messages, requestId: null, error: null }
    this.setDraft(id, '')
    void this.run(session, database, schemaContext)
  }

  /** Asks again after a failure, without adding a message. */
  retry(session: SessionInfo, database: string, schemaContext: string) {
    const chat = this.chat(session.sessionId)
    if (chat.requestId || chat.messages.at(-1)?.role !== 'user') return
    chat.error = null
    void this.run(session, database, schemaContext)
  }

  private async run(session: SessionInfo, database: string, schemaContext: string) {
    const id = session.sessionId
    const requestId = newRequestId()
    const chat = this.chats[id]
    chat.requestId = requestId
    chat.error = null
    const stillCurrent = () => this.chats[id]?.requestId === requestId
    try {
      const reply = await api.assistantAsk({
        requestId,
        engine: session.config.engine,
        serverVersion: session.serverVersion,
        database,
        schemaContext,
        messages: chat.messages.map(({ role, content }) => ({ role, content })),
        model: this.model,
      })
      if (!stillCurrent()) return
      this.chats[id].messages.push({ id: this.nextId++, role: 'assistant', content: reply.text })
    } catch (e) {
      if (!stillCurrent()) return
      if (!(isCoreError(e) && e.kind === 'cancelled')) {
        this.chats[id].error = errorMessage(e)
        // A missing key shows the "add your key" state instead of an error line.
        void this.refreshKey()
      }
    } finally {
      if (stillCurrent()) this.chats[id].requestId = null
    }
  }

  /** Stops the pending reply; the question stays in the conversation. */
  cancel(sessionId: string) {
    const chat = this.chats[sessionId]
    const requestId = chat?.requestId
    if (!requestId) return
    chat.requestId = null
    void api.assistantCancel(requestId).catch(() => {})
  }

  /** Forgets the conversation (and stops a pending reply). */
  clear(sessionId: string) {
    this.cancel(sessionId)
    delete this.chats[sessionId]
  }
}

export const assistant = new AssistantStore()
