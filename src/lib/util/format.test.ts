import { afterEach, describe, expect, it, vi } from 'vitest'
import { basename, compactCount, formatBytes, formatCount, formatDuration } from './format'

describe('formatCount', () => {
  it('groups digits', () => {
    expect(formatCount(0)).toBe('0')
    expect(formatCount(12)).toBe('12')
    expect(formatCount(1000)).toBe('1,000')
    expect(formatCount(1_234_567_890)).toBe('1,234,567,890')
    expect(formatCount(-9876)).toBe('-9,876')
  })
})

describe('compactCount', () => {
  it.each([
    [0, '0'],
    [7, '7'],
    [999, '999'],
    [1000, '1k'],
    [1049, '1k'],
    [1050, '1.1k'],
    [1500, '1.5k'],
    [9999, '10k'],
    [10_000, '10k'],
    [12_345, '12k'],
    [99_999, '100k'],
    [500_000, '500k'],
    [1_000_000, '1M'],
    [1_500_000, '1.5M'],
    [9_999_999, '10M'],
    [10_000_000, '10M'],
    [123_456_789, '123M'],
    [1_000_000_000, '1B'],
    [2_500_000_000, '2.5B'],
    [123_000_000_000, '123B'],
  ])('%d -> %s', (n, expected) => {
    expect(compactCount(n)).toBe(expected)
  })

  it('never prints 1000 of a unit: values just under a boundary roll over', () => {
    expect(compactCount(999_500)).toBe('1M')
    expect(compactCount(999_999)).toBe('1M')
    expect(compactCount(999_999_999)).toBe('1B')
  })

  it('never renders a leading number of 1000 or more, across a sweep', () => {
    for (let n = 0; n < 2_000_000; n += 997) {
      const text = compactCount(n)
      const num = parseFloat(text)
      expect(num).toBeLessThan(1000)
    }
  })
})

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [512, '512 B'],
    [1023, '1023 B'],
    [1024, '1.0 KB'],
    [1536, '1.5 KB'],
    [10 * 1024, '10 KB'],
    [500 * 1024, '500 KB'],
    [1024 ** 2, '1.0 MB'],
    [3.5 * 1024 ** 2, '3.5 MB'],
    [1024 ** 3, '1.0 GB'],
    [1024 ** 4, '1.0 TB'],
  ])('%d -> %s', (n, expected) => {
    expect(formatBytes(n)).toBe(expected)
  })

  it('rolls over instead of printing 1024 of the previous unit', () => {
    expect(formatBytes(1024 ** 2 - 1)).toBe('1.0 MB')
    expect(formatBytes(1024 ** 3 - 1)).toBe('1.0 GB')
    expect(formatBytes(1024 ** 4 - 1)).toBe('1.0 TB')
  })
})

describe('formatDuration', () => {
  it.each([
    [0, '0 ms'],
    [0.4, '0 ms'],
    [12, '12 ms'],
    [999, '999 ms'],
    [1000, '1.00 s'],
    [1234, '1.23 s'],
    [9999, '10.00 s'],
    [10_000, '10.0 s'],
    [12_345, '12.3 s'],
    [59_000, '59.0 s'],
    [60_000, '1m 0s'],
    [61_500, '1m 1s'],
    [125_000, '2m 5s'],
    [3_600_000, '60m 0s'],
  ])('%d ms -> %s', (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected)
  })

  it('does not print 1000 ms or 60.0 s at the unit boundaries', () => {
    expect(formatDuration(999.6)).toBe('1.00 s')
    expect(formatDuration(59_999)).toBe('1m 0s')
  })
})

describe('basename', () => {
  it('handles posix paths', () => {
    expect(basename('/a/b/c.csv')).toBe('c.csv')
  })
  it('handles windows paths', () => {
    expect(basename('C:\\Users\\me\\data.db')).toBe('data.db')
  })
  it('handles mixed separators', () => {
    expect(basename('C:\\x/y\\z.txt')).toBe('z.txt')
  })
  it('returns a bare name unchanged', () => {
    expect(basename('file.sql')).toBe('file.sql')
  })
  it('returns the empty string for a trailing slash', () => {
    expect(basename('/a/b/')).toBe('')
  })
  it('empty input', () => {
    expect(basename('')).toBe('')
  })
})

describe('isMac / modLabel', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.resetModules()
  })

  async function loadWith(platform: string | undefined) {
    vi.resetModules()
    if (platform === undefined) vi.stubGlobal('navigator', undefined)
    else vi.stubGlobal('navigator', { platform })
    return import('./format')
  }

  it('is Command on macOS', async () => {
    const m = await loadWith('MacIntel')
    expect(m.isMac).toBe(true)
    expect(m.modLabel).toBe('⌘')
  })

  it('is Command on iOS', async () => {
    expect((await loadWith('iPhone')).isMac).toBe(true)
    expect((await loadWith('iPad')).isMac).toBe(true)
  })

  it('is Ctrl elsewhere', async () => {
    for (const p of ['Win32', 'Linux x86_64']) {
      const m = await loadWith(p)
      expect(m.isMac).toBe(false)
      expect(m.modLabel).toBe('Ctrl')
    }
  })

  it('falls back to Ctrl without a navigator', async () => {
    const m = await loadWith(undefined)
    expect(m.isMac).toBe(false)
    expect(m.modLabel).toBe('Ctrl')
  })
})
