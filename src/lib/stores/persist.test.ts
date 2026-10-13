import { afterEach, describe, expect, it, vi } from 'vitest'
import { installStorage } from '../../test/helpers'
import { load, save } from './persist'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('load', () => {
  it('returns the fallback when the key is missing', () => {
    installStorage()
    expect(load('k', 'fallback')).toBe('fallback')
    expect(load('k', [1])).toEqual([1])
  })

  it('parses stored JSON', () => {
    installStorage({ k: { a: [1, 2, { b: null }] } })
    expect(load('k', null)).toEqual({ a: [1, 2, { b: null }] })
  })

  it('returns stored falsy JSON values instead of the fallback', () => {
    installStorage({ zero: 0, no: false, nul: null, emptyArr: [] })
    expect(load('zero', 9)).toBe(0)
    expect(load('no', true)).toBe(false)
    expect(load('nul', 'x')).toBeNull()
    expect(load('emptyArr', [1])).toEqual([])
  })

  it('returns the fallback for corrupt JSON', () => {
    installStorage({ k: '{not json' })
    expect(load('k', 'fb')).toBe('fb')
  })

  it('returns the fallback for an empty string', () => {
    installStorage({ k: '' })
    expect(load('k', 'fb')).toBe('fb')
  })

  it('returns the fallback when reading throws (blocked storage)', () => {
    vi.stubGlobal('localStorage', {
      getItem() {
        throw new Error('SecurityError')
      },
    })
    expect(load('k', 'fb')).toBe('fb')
  })

  it('returns the fallback when there is no localStorage at all', () => {
    vi.stubGlobal('localStorage', undefined)
    expect(load('k', 'fb')).toBe('fb')
  })
})

describe('save', () => {
  it('stores JSON that load can read back', () => {
    const s = installStorage()
    save('k', { a: 1, list: [true, null, 'x'] })
    expect(s.getItem('k')).toBe('{"a":1,"list":[true,null,"x"]}')
    expect(load('k', null)).toEqual({ a: 1, list: [true, null, 'x'] })
  })

  it('overwrites an existing value', () => {
    const s = installStorage({ k: 1 })
    save('k', 2)
    expect(s.getItem('k')).toBe('2')
  })

  it('does not throw when the quota is exceeded', () => {
    vi.stubGlobal('localStorage', {
      setItem() {
        throw new DOMException('full', 'QuotaExceededError')
      },
    })
    expect(() => save('k', 'v')).not.toThrow()
  })

  it('does not throw for values JSON cannot serialise', () => {
    installStorage()
    const circular: Record<string, unknown> = {}
    circular.self = circular
    expect(() => save('k', circular)).not.toThrow()
    expect(() => save('k', 10n)).not.toThrow()
  })

  it('does not throw when there is no localStorage', () => {
    vi.stubGlobal('localStorage', undefined)
    expect(() => save('k', 'v')).not.toThrow()
  })
})
