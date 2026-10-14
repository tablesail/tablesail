// Prompts the SQL editor features raise for the workspace to show (like ui.exportRequest, but local to the editor).

export interface SqlImportRequest {
  sessionId: string
  /** the .sql dump the user picked */
  path: string
}

class EditorRequests {
  /** A query tab asked to leave manual mode while its transaction is open: commit, roll back or stay. */
  leaveManual = $state<string | null>(null)
  /** A disconnect that waits for the session's open transactions to be committed or rolled back. */
  disconnect = $state<string | null>(null)
  /** A picked .sql dump waiting for its import options. */
  sqlImport = $state<SqlImportRequest | null>(null)
}

export const editorRequests = new EditorRequests()
