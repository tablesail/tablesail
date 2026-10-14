import { describe, expect, it } from 'vitest'
import type { Engine, TableKind, TableRef } from '../types'
import {
  actionFor,
  adminActions,
  carryTableState,
  confirmLabel,
  dialogTitle,
  doneVerb,
  dropConfirmed,
  holdsRows,
  itemsNoun,
  menuLabel,
  noun,
  renameProblem,
  tableStateKeys,
  treeKey,
} from './tableAdmin'

describe('adminActions', () => {
  it('offers rename, truncate and drop for tables, virtual tables and collections', () => {
    for (const kind of ['table', 'virtual', 'collection'] as TableKind[]) {
      expect(adminActions(kind, 'postgres')).toEqual(['rename', 'truncate', 'drop'])
    }
  })

  it('offers views and materialized views rename and drop only', () => {
    expect(adminActions('view', 'postgres')).toEqual(['rename', 'drop'])
    expect(adminActions('view', 'mysql')).toEqual(['rename', 'drop'])
    expect(adminActions('materializedView', 'postgres')).toEqual(['rename', 'drop'])
  })

  it('cannot rename a view in SQLite (there is no ALTER VIEW)', () => {
    expect(adminActions('view', 'sqlite')).toEqual(['drop'])
  })

  it('cannot rename a time-series collection', () => {
    expect(adminActions('timeSeries', 'mongodb')).toEqual(['truncate', 'drop'])
  })

  it('offers foreign tables rename and drop', () => {
    expect(adminActions('foreignTable', 'postgres')).toEqual(['rename', 'drop'])
  })

  it('cannot rename a MongoDB view either, only drop it', () => {
    expect(adminActions('view', 'mongodb')).toEqual(['drop'])
  })

  it('offers sequences rename and drop where they are objects of their own', () => {
    for (const engine of ['postgres', 'mysql', 'mariadb'] as Engine[]) expect(adminActions('sequence', engine)).toEqual(['rename', 'drop'])
    expect(adminActions('sequence', 'sqlite')).toEqual([])
    expect(adminActions('sequence', 'mongodb')).toEqual([])
  })

  it('leaves system objects alone', () => {
    for (const engine of ['postgres', 'mysql', 'sqlite', 'mongodb'] as Engine[]) expect(adminActions('system', engine)).toEqual([])
  })
})

describe('wording', () => {
  it('names the object after its kind', () => {
    expect(noun('table', 'postgres')).toBe('table')
    expect(noun('view', 'mysql')).toBe('view')
    expect(noun('materializedView', 'postgres')).toBe('materialized view')
    expect(noun('foreignTable', 'postgres')).toBe('foreign table')
    expect(noun('sequence', 'postgres')).toBe('sequence')
  })

  it('knows which objects hold rows of their own', () => {
    for (const kind of ['table', 'virtual', 'collection', 'timeSeries'] as TableKind[]) expect(holdsRows(kind)).toBe(true)
    for (const kind of ['view', 'materializedView', 'foreignTable', 'sequence', 'system'] as TableKind[]) expect(holdsRows(kind)).toBe(false)
  })

  it('calls everything a collection on MongoDB', () => {
    expect(noun('collection', 'mongodb')).toBe('collection')
    expect(noun('table', 'mongodb')).toBe('collection')
    expect(noun('timeSeries', 'mongodb')).toBe('collection')
    expect(noun('view', 'mongodb')).toBe('view')
    expect(itemsNoun('mongodb')).toBe('documents')
    expect(itemsNoun('postgres')).toBe('rows')
  })

  it('labels the menu items, with "Empty collection" for MongoDB', () => {
    expect(menuLabel('rename', 'postgres')).toBe('Rename…')
    expect(menuLabel('truncate', 'postgres')).toBe('Truncate…')
    expect(menuLabel('drop', 'postgres')).toBe('Drop…')
    expect(menuLabel('rename', 'mongodb')).toBe('Rename…')
    expect(menuLabel('truncate', 'mongodb')).toBe('Empty collection…')
    expect(menuLabel('drop', 'mongodb')).toBe('Drop…')
  })

  it('titles the dialog after the action and the kind', () => {
    expect(dialogTitle('drop', 'table', 'postgres')).toBe('Drop table')
    expect(dialogTitle('truncate', 'table', 'mysql')).toBe('Truncate table')
    expect(dialogTitle('rename', 'view', 'postgres')).toBe('Rename view')
    expect(dialogTitle('drop', 'materializedView', 'postgres')).toBe('Drop materialized view')
    expect(dialogTitle('truncate', 'collection', 'mongodb')).toBe('Empty collection')
    expect(dialogTitle('drop', 'collection', 'mongodb')).toBe('Drop collection')
  })

  it('labels the primary button and the toast verb', () => {
    expect(confirmLabel('rename', 'sqlite')).toBe('Rename')
    expect(confirmLabel('drop', 'sqlite')).toBe('Drop')
    expect(confirmLabel('truncate', 'sqlite')).toBe('Truncate')
    expect(confirmLabel('truncate', 'mongodb')).toBe('Empty')
    expect(doneVerb('rename', 'postgres')).toBe('Renamed')
    expect(doneVerb('drop', 'postgres')).toBe('Dropped')
    expect(doneVerb('truncate', 'postgres')).toBe('Truncated')
    expect(doneVerb('truncate', 'mongodb')).toBe('Emptied')
  })
})

describe('renameProblem', () => {
  it('rejects an empty or blank name', () => {
    expect(renameProblem('users', '')).toBe('Enter a new name.')
    expect(renameProblem('users', '   ')).toBe('Enter a new name.')
  })

  it('rejects the unchanged name, ignoring surrounding spaces', () => {
    expect(renameProblem('users', 'users')).toBe('Choose a different name.')
    expect(renameProblem('users', '  users ')).toBe('Choose a different name.')
  })

  it('accepts a different name; case counts as different', () => {
    expect(renameProblem('users', 'people')).toBeNull()
    expect(renameProblem('users', 'Users')).toBeNull()
    expect(renameProblem('users', 'my table')).toBeNull()
  })
})

describe('dropConfirmed', () => {
  it('needs the exact name (case-sensitive), spaces around a paste aside', () => {
    expect(dropConfirmed('MixedCase', 'MixedCase')).toBe(true)
    expect(dropConfirmed('MixedCase', '  MixedCase\n')).toBe(true)
    expect(dropConfirmed('MixedCase', 'mixedcase')).toBe(false)
    expect(dropConfirmed('users', 'user')).toBe(false)
    expect(dropConfirmed('users', '')).toBe(false)
  })
})

describe('actionFor', () => {
  it('maps drop and truncate straight through', () => {
    expect(actionFor('drop', 'users', 'ignored')).toEqual({ kind: 'drop' })
    expect(actionFor('truncate', 'users', 'ignored')).toEqual({ kind: 'truncate' })
  })

  it('sends the trimmed new name for a rename, and nothing while it is unusable', () => {
    expect(actionFor('rename', 'users', '  people ')).toEqual({ kind: 'rename', newName: 'people' })
    expect(actionFor('rename', 'users', 'users')).toBeNull()
    expect(actionFor('rename', 'users', ' ')).toBeNull()
  })
})

describe('treeKey', () => {
  it('qualifies with the schema when there is one', () => {
    expect(treeKey('public', 'users')).toBe('public.users')
    expect(treeKey(null, 'users')).toBe('users')
    expect(treeKey(undefined, 'users')).toBe('users')
    expect(treeKey('', 'users')).toBe('users')
  })
})

describe('carryTableState', () => {
  const users: TableRef = { database: 'shop', schema: 'public', name: 'users' }
  const store = (initial: Record<string, string>) => {
    const map = new Map(Object.entries(initial))
    return {
      map,
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
      removeItem: (k: string) => void map.delete(k),
    }
  }

  it('names the keys the table view uses (draft per session and per connection, hidden columns)', () => {
    expect(tableStateKeys('s1', 'c1', users)).toEqual([
      'tablesail:draft:s1:shop.public.users',
      'tablesail:draft:c1:shop.public.users',
      'tablesail.hiddenColumns.c1.shop.public.users',
    ])
    // No schema: an empty segment, like the table view's.
    expect(tableStateKeys('s1', 'c1', { database: 'd', schema: null, name: 't' })[2]).toBe('tablesail.hiddenColumns.c1.d..t')
  })

  it('moves the draft and the hidden columns to the new name on a rename', () => {
    const [d1, d2, h] = tableStateKeys('s1', 'c1', users)
    const s = store({ [d1]: '{"d":1}', [d2]: '{"d":2}', [h]: '["a"]', other: 'x' })
    carryTableState('s1', 'c1', users, 'people', s)
    const [n1, n2, nh] = tableStateKeys('s1', 'c1', { ...users, name: 'people' })
    expect(Object.fromEntries(s.map)).toEqual({ [n1]: '{"d":1}', [n2]: '{"d":2}', [nh]: '["a"]', other: 'x' })
  })

  it('forgets them on a drop', () => {
    const keys = tableStateKeys('s1', 'c1', users)
    const s = store({ [keys[0]]: '1', [keys[1]]: '2', [keys[2]]: '3', keep: 'x' })
    carryTableState('s1', 'c1', users, null, s)
    expect(Object.fromEntries(s.map)).toEqual({ keep: 'x' })
  })

  it('does nothing when there is nothing stored, and does not throw when storage does', () => {
    const s = store({})
    carryTableState('s1', 'c1', users, 'people', s)
    expect(s.map.size).toBe(0)
    const broken = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {},
      removeItem: () => {},
    }
    expect(() => carryTableState('s1', 'c1', users, 'people', broken)).not.toThrow()
  })
})
