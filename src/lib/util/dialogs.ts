import { open, save } from '@tauri-apps/plugin-dialog'

export interface FileFilter {
  name: string
  extensions: string[]
}

/** Native open dialog; null when cancelled or unavailable. */
export async function pickFile(options: { title?: string; filters?: FileFilter[] } = {}): Promise<string | null> {
  try {
    const picked = await open({ multiple: false, directory: false, ...options })
    return typeof picked === 'string' ? picked : null
  } catch {
    return null
  }
}

export async function pickSavePath(options: {
  title?: string
  defaultPath?: string
  filters?: FileFilter[]
}): Promise<string | null> {
  try {
    return (await save(options)) ?? null
  } catch {
    return null
  }
}
