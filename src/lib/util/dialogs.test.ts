import { beforeEach, describe, expect, it, vi } from 'vitest'

const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => dialog)

import { pickFile, pickSavePath } from './dialogs'

beforeEach(() => {
  dialog.open.mockReset()
  dialog.save.mockReset()
})

describe('pickFile', () => {
  it('returns the chosen path', async () => {
    dialog.open.mockResolvedValue('/tmp/data.csv')
    expect(await pickFile()).toBe('/tmp/data.csv')
  })

  it('opens a single-file (not directory) dialog and forwards the options', async () => {
    dialog.open.mockResolvedValue('/x')
    const filters = [{ name: 'CSV', extensions: ['csv'] }]
    await pickFile({ title: 'Pick', filters })
    expect(dialog.open).toHaveBeenCalledWith({ multiple: false, directory: false, title: 'Pick', filters })
  })

  it('returns null when cancelled', async () => {
    dialog.open.mockResolvedValue(null)
    expect(await pickFile()).toBeNull()
  })

  it('returns null if the plugin returns something that is not a single path', async () => {
    dialog.open.mockResolvedValue(['/a', '/b'])
    expect(await pickFile()).toBeNull()
  })

  it('returns null when the dialog is unavailable (rejects)', async () => {
    dialog.open.mockRejectedValue(new Error('not in tauri'))
    expect(await pickFile()).toBeNull()
  })

  it('returns null if the plugin throws synchronously', async () => {
    dialog.open.mockImplementation(() => {
      throw new Error('boom')
    })
    expect(await pickFile()).toBeNull()
  })
})

describe('pickSavePath', () => {
  it('returns the chosen path and forwards the options', async () => {
    dialog.save.mockResolvedValue('/tmp/out.json')
    const opts = { title: 'Save', defaultPath: 'users.json', filters: [{ name: 'JSON', extensions: ['json'] }] }
    expect(await pickSavePath(opts)).toBe('/tmp/out.json')
    expect(dialog.save).toHaveBeenCalledWith(opts)
  })

  it('returns null when cancelled (null or undefined)', async () => {
    dialog.save.mockResolvedValue(null)
    expect(await pickSavePath({})).toBeNull()
    dialog.save.mockResolvedValue(undefined)
    expect(await pickSavePath({})).toBeNull()
  })

  it('returns null when the dialog is unavailable', async () => {
    dialog.save.mockRejectedValue(new Error('nope'))
    expect(await pickSavePath({})).toBeNull()
  })
})
