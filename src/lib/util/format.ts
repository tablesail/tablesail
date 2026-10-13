export function formatCount(n: number): string {
  return n.toLocaleString('en-US')
}

export function compactCount(n: number): string {
  if (n < 1000) return String(n)
  const tiers = [
    [1e3, 'k'],
    [1e6, 'M'],
    [1e9, 'B'],
  ] as const
  const text = (t: number) => {
    const v = n / tiers[t][0]
    return v.toFixed(t === 2 || v < 10 ? 1 : 0)
  }
  let t = n < 1e6 ? 0 : n < 1e9 ? 1 : 2
  // 999,999 rounds to "1000" of its unit; show it as 1M instead.
  if (t < 2 && Number(text(t)) >= 1000) t++
  return text(t).replace(/\.0$/, '') + tiers[t][1]
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  const digits = (x: number) => (x < 10 ? 1 : 0)
  // Rounding can land on 1024 of this unit (1 MB - 1 byte): show it as 1.0 of the next one.
  if (Number(v.toFixed(digits(v))) >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(digits(v))} ${units[i]}`
}

export function formatDuration(ms: number): string {
  if (Math.round(ms) < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) {
    const s = (ms / 1000).toFixed(ms < 10_000 ? 2 : 1)
    // 59.99 s would round up to "60.0 s"
    return Number(s) < 60 ? `${s} s` : '1m 0s'
  }
  const m = Math.floor(ms / 60_000)
  const s = Math.floor((ms % 60_000) / 1000)
  return `${m}m ${s}s`
}

export function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path
}

export const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
export const modLabel = isMac ? '⌘' : 'Ctrl'
