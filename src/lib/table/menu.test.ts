import { describe, expect, it, vi } from 'vitest'
import { copyMenuItems } from './menu'

const build = (engine: Parameters<typeof copyMenuItems>[0]['engine'], disabled = false) => {
  const copy = vi.fn()
  const copyAs = vi.fn()
  return { items: copyMenuItems({ engine, disabled, copy, copyAs }), copy, copyAs }
}

describe('copyMenuItems', () => {
  it('offers Copy, Copy with headers and a Copy as flyout', () => {
    const { items } = build('postgres')
    expect(items.map((i) => i.label)).toEqual(['Copy', 'Copy with headers', 'Copy as'])
    expect(items[2].submenu?.map((i) => i.label)).toEqual(['INSERT statements', 'JSON', 'Markdown table', 'Tab-separated'])
  })

  it('shows the keyboard shortcuts of the grid next to Copy and Copy with headers', () => {
    const { items } = build('postgres')
    expect(items.map((i) => i.hint)).toEqual(['⌘C', '⌘⇧C', undefined])
  })

  it('offers JSON and Tab-separated only for MongoDB', () => {
    const { items } = build('mongodb')
    expect(items[2].submenu?.map((i) => i.label)).toEqual(['JSON', 'Tab-separated'])
  })

  it('wires the entries to the callbacks', () => {
    const { items, copy, copyAs } = build('mysql')
    items[0].onclick?.()
    items[1].onclick?.()
    expect(copy.mock.calls).toEqual([[false], [true]])
    items[2].submenu?.[1].onclick?.()
    items[2].submenu?.[0].onclick?.()
    expect(copyAs.mock.calls).toEqual([['json'], ['insert']])
  })

  it('disables everything together', () => {
    const { items } = build('sqlite', true)
    expect(items.every((i) => i.disabled)).toBe(true)
  })
})
