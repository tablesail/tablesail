import { beforeEach, describe, expect, it, vi } from 'vitest'
import { routeInvoke } from '../../test/helpers'
import type { ConnectionConfig } from '../types'
import { newConfig } from '../util/env'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

async function fresh() {
  vi.resetModules()
  return (await import('./connections.svelte')).connections
}

const conn = (id: string, name = id): ConnectionConfig => ({ ...newConfig('postgres'), id, name })

let connections: Awaited<ReturnType<typeof fresh>>
beforeEach(async () => {
  core.invoke.mockReset()
  connections = await fresh()
})

describe('connections.load', () => {
  it('starts unloaded and empty', () => {
    expect(connections.list).toEqual([])
    expect(connections.loaded).toBe(false)
    expect(connections.error).toBeNull()
  })

  it('loads the saved connections', async () => {
    core.invoke.mockImplementation(routeInvoke({ connections_list: () => [conn('a'), conn('b')] }))
    await connections.load()
    expect(connections.list.map((c) => c.id)).toEqual(['a', 'b'])
    expect(connections.loaded).toBe(true)
    expect(connections.error).toBeNull()
  })

  it('records the error message and still marks the store loaded', async () => {
    core.invoke.mockRejectedValue({ kind: 'io', message: 'cannot read store', code: null, statement: null })
    await connections.load()
    expect(connections.error).toBe('cannot read store')
    expect(connections.loaded).toBe(true)
    expect(connections.list).toEqual([])
  })

  it('a later successful load clears the error', async () => {
    core.invoke.mockRejectedValueOnce(new Error('first failure'))
    await connections.load()
    expect(connections.error).toBe('first failure')
    core.invoke.mockResolvedValueOnce([conn('a')])
    await connections.load()
    expect(connections.error).toBeNull()
    expect(connections.list).toHaveLength(1)
  })

  it('a failed reload keeps the previous list', async () => {
    core.invoke.mockResolvedValueOnce([conn('a')])
    await connections.load()
    core.invoke.mockRejectedValueOnce(new Error('later failure'))
    await connections.load()
    expect(connections.list.map((c) => c.id)).toEqual(['a'])
  })
})

describe('connections.save', () => {
  it('appends a new connection with the id the backend assigned', async () => {
    core.invoke.mockImplementation(routeInvoke({ connection_save: ({ config }) => ({ ...(config as ConnectionConfig), id: 'new-id' }) }))
    const saved = await connections.save(conn('', 'Fresh'), { dbPassword: 'pw' })
    expect(saved.id).toBe('new-id')
    expect(connections.list.map((c) => c.id)).toEqual(['new-id'])
    expect(core.invoke).toHaveBeenCalledWith('connection_save', expect.objectContaining({ secrets: { dbPassword: 'pw' } }))
  })

  it('replaces an existing connection in place', async () => {
    core.invoke.mockImplementation(routeInvoke({ connections_list: () => [conn('a', 'A'), conn('b', 'B'), conn('c', 'C')], connection_save: ({ config }) => config }))
    await connections.load()
    await connections.save(conn('b', 'Renamed'), null)
    expect(connections.list.map((c) => c.name)).toEqual(['A', 'Renamed', 'C'])
  })

  it('does not add to the list when the backend rejects', async () => {
    core.invoke.mockRejectedValue({ kind: 'invalidInput', message: 'name required', code: null, statement: null })
    await expect(connections.save(conn('x'), null)).rejects.toMatchObject({ message: 'name required' })
    expect(connections.list).toEqual([])
  })
})

describe('connections.remove', () => {
  it('deletes on the backend and drops it from the list', async () => {
    core.invoke.mockImplementation(routeInvoke({ connections_list: () => [conn('a'), conn('b')], connection_delete: () => undefined }))
    await connections.load()
    await connections.remove('a')
    expect(connections.list.map((c) => c.id)).toEqual(['b'])
    expect(core.invoke).toHaveBeenCalledWith('connection_delete', { id: 'a' })
  })

  it('keeps the connection when the backend fails', async () => {
    core.invoke.mockImplementation(
      routeInvoke({
        connections_list: () => [conn('a')],
        connection_delete: () => {
          throw new Error('locked')
        },
      }),
    )
    await connections.load()
    await expect(connections.remove('a')).rejects.toThrow('locked')
    expect(connections.list).toHaveLength(1)
  })
})

describe('connections.duplicate', () => {
  it('saves a copy with an empty id, a "copy" name and no secrets', async () => {
    let sent: { config: ConnectionConfig; secrets: unknown } | undefined
    core.invoke.mockImplementation(
      routeInvoke({
        connection_save: (args) => {
          sent = args as typeof sent
          return { ...(args.config as ConnectionConfig), id: 'copy-id' }
        },
      }),
    )
    const original = { ...conn('orig', 'Prod DB'), host: 'db.example.com', options: { sslmode: 'require' } }
    const copy = await connections.duplicate(original)
    expect(sent?.config.id).toBe('')
    expect(sent?.config.name).toBe('Prod DB copy')
    expect(sent?.config.host).toBe('db.example.com')
    expect(sent?.secrets).toBeNull()
    expect(copy.id).toBe('copy-id')
    expect(connections.list.map((c) => c.id)).toEqual(['copy-id'])
  })

  it('does not share nested objects with the original', async () => {
    let sent: ConnectionConfig | undefined
    core.invoke.mockImplementation(
      routeInvoke({
        connection_save: (args) => {
          sent = args.config as ConnectionConfig
          return { ...sent, id: 'x' }
        },
      }),
    )
    const original = conn('orig')
    original.options.a = '1'
    await connections.duplicate(original)
    sent!.options.a = 'changed'
    expect(original.options.a).toBe('1')
    expect(sent!.ssl).not.toBe(original.ssl)
  })

  it('leaves the original untouched', async () => {
    core.invoke.mockImplementation(routeInvoke({ connection_save: ({ config }) => ({ ...(config as ConnectionConfig), id: 'x' }) }))
    const original = conn('orig', 'Name')
    await connections.duplicate(original)
    expect(original.id).toBe('orig')
    expect(original.name).toBe('Name')
  })
})
