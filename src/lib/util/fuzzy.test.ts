import { describe, expect, it } from 'vitest'
import { fuzzyFilter, fuzzyScore } from './fuzzy'

describe('fuzzyScore', () => {
  it('an empty query matches everything with score 0', () => {
    expect(fuzzyScore('', 'anything')).toBe(0)
    expect(fuzzyScore('', '')).toBe(0)
  })

  it('returns null when the query is not a subsequence', () => {
    expect(fuzzyScore('xyz', 'users')).toBeNull()
    expect(fuzzyScore('sru', 'users')).toBeNull() // order matters
    expect(fuzzyScore('a', '')).toBeNull()
  })

  it('does not reuse a character twice', () => {
    expect(fuzzyScore('aa', 'abc')).toBeNull()
    expect(fuzzyScore('aa', 'aba')).not.toBeNull()
  })

  it('is case-insensitive on both sides', () => {
    expect(fuzzyScore('USR', 'users')).not.toBeNull()
    expect(fuzzyScore('usr', 'USERS')).not.toBeNull()
    expect(fuzzyScore('USR', 'users')).toBe(fuzzyScore('usr', 'USERS'))
  })

  it('matches a non-contiguous subsequence', () => {
    expect(fuzzyScore('usrs', 'users')).not.toBeNull()
  })

  it('prefers contiguous matches', () => {
    const contiguous = fuzzyScore('use', 'users')!
    const scattered = fuzzyScore('use', 'u_s_e_r')!
    expect(contiguous).toBeGreaterThan(scattered)
  })

  it('prefers matches at the start or after a separator', () => {
    const start = fuzzyScore('u', 'users')!
    const boundary = fuzzyScore('u', 'app_users')!
    const middle = fuzzyScore('u', 'abuse')!
    expect(start).toBeGreaterThan(middle)
    expect(boundary).toBeGreaterThan(middle)
  })

  it('treats any non-alphanumeric as a word boundary', () => {
    expect(fuzzyScore('b', 'a.b')).toBeGreaterThan(fuzzyScore('b', 'ab')!)
    expect(fuzzyScore('b', 'a b')).toBeGreaterThan(fuzzyScore('b', 'ab')!)
    expect(fuzzyScore('b', 'a-b')).toBeGreaterThan(fuzzyScore('b', 'ab')!)
  })

  it('breaks ties in favour of shorter text', () => {
    expect(fuzzyScore('user', 'users')!).toBeGreaterThan(fuzzyScore('user', 'users_archive')!)
  })

  it('an exact match scores at least as high as a prefix match', () => {
    expect(fuzzyScore('users', 'users')!).toBeGreaterThan(fuzzyScore('users', 'users2')!)
  })

  it('handles regex metacharacters literally', () => {
    expect(fuzzyScore('.*', 'a.*b')).not.toBeNull()
    expect(fuzzyScore('.*', 'abc')).toBeNull()
    expect(fuzzyScore('(', 'f(x)')).not.toBeNull()
  })

  it('handles unicode', () => {
    expect(fuzzyScore('é', 'café')).not.toBeNull()
    expect(fuzzyScore('表', '用户表')).not.toBeNull()
  })
})

describe('fuzzyFilter', () => {
  const items = ['users', 'user_roles', 'orders', 'order_items', 'audit_log', 'abuse']

  it('returns the first `limit` items for an empty query, in order', () => {
    expect(fuzzyFilter(items, '', (s) => s)).toEqual(items)
    expect(fuzzyFilter(items, '', (s) => s, 2)).toEqual(['users', 'user_roles'])
  })

  it('does not mutate or alias the input for an empty query', () => {
    const out = fuzzyFilter(items, '', (s) => s)
    expect(out).not.toBe(items)
    expect(items).toHaveLength(6)
  })

  it('filters out non-matches', () => {
    expect(fuzzyFilter(items, 'ord', (s) => s)).toEqual(['orders', 'order_items'])
  })

  it('ranks the better match first', () => {
    const out = fuzzyFilter(items, 'use', (s) => s)
    expect(out[0]).toBe('users')
    expect(out).toContain('user_roles')
    expect(out.indexOf('users')).toBeLessThan(out.indexOf('abuse'))
  })

  it('applies the limit after ranking', () => {
    expect(fuzzyFilter(items, 'o', (s) => s, 1)).toHaveLength(1)
  })

  it('works on objects through the text accessor', () => {
    const rows = [{ n: 'alpha' }, { n: 'beta' }, { n: 'alphabet' }]
    expect(fuzzyFilter(rows, 'alp', (r) => r.n).map((r) => r.n)).toEqual(['alpha', 'alphabet'])
  })

  it('returns an empty array when nothing matches', () => {
    expect(fuzzyFilter(items, 'zzz', (s) => s)).toEqual([])
    expect(fuzzyFilter([], 'a', (s: string) => s)).toEqual([])
  })

  it('keeps the original order for equal scores (stable sort)', () => {
    const out = fuzzyFilter(['ab', 'ab', 'ab'].map((s, i) => ({ s, i })), 'ab', (x) => x.s)
    expect(out.map((x) => x.i)).toEqual([0, 1, 2])
  })
})
