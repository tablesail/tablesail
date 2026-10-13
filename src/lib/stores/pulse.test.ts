import { beforeEach, describe, expect, it, vi } from 'vitest'

async function fresh() {
  vi.resetModules()
  return (await import('./pulse.svelte')).pulse
}

let pulse: Awaited<ReturnType<typeof fresh>>
beforeEach(async () => {
  pulse = await fresh()
})

describe('pulse', () => {
  it('has no samples for an unknown session', () => {
    expect(pulse.for('nope')).toEqual([])
  })

  it('records samples in order', () => {
    pulse.record('s1', 10)
    pulse.record('s1', 20)
    pulse.record('s1', 5)
    expect(pulse.for('s1')).toEqual([10, 20, 5])
  })

  it('keeps sessions apart', () => {
    pulse.record('a', 1)
    pulse.record('b', 2)
    expect(pulse.for('a')).toEqual([1])
    expect(pulse.for('b')).toEqual([2])
  })

  it('accepts zero', () => {
    pulse.record('s', 0)
    expect(pulse.for('s')).toEqual([0])
  })

  it('accepts fractional milliseconds', () => {
    pulse.record('s', 0.25)
    expect(pulse.for('s')).toEqual([0.25])
  })

  it('ignores negative, NaN and infinite samples', () => {
    pulse.record('s', -1)
    pulse.record('s', NaN)
    pulse.record('s', Infinity)
    pulse.record('s', -Infinity)
    expect(pulse.for('s')).toEqual([])
  })

  it('keeps only the latest 40 samples', () => {
    for (let i = 1; i <= 100; i++) pulse.record('s', i)
    const list = pulse.for('s')
    expect(list).toHaveLength(40)
    expect(list[0]).toBe(61)
    expect(list[39]).toBe(100)
  })

  it('exactly 40 samples are all kept', () => {
    for (let i = 1; i <= 40; i++) pulse.record('s', i)
    expect(pulse.for('s')).toHaveLength(40)
    expect(pulse.for('s')[0]).toBe(1)
  })

  it('replaces the array on each record (so reactive readers see a change)', () => {
    pulse.record('s', 1)
    const before = pulse.for('s')
    pulse.record('s', 2)
    expect(pulse.for('s')).not.toBe(before)
    expect(before).toEqual([1])
  })
})
