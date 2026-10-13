<!--
  Table view: browse a table (Data) with lazy paging, filters and sorting pushed down to the
  database, edit cells with pending changes (preview, commit, discard, drafts), and show the
  Structure. Props: { session, table, active, onExport?, onImport? }. Keyboard shortcuts are
  only handled while `active` is true.
-->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import { onDestroy, tick, untrack } from 'svelte'
  import { api, errorMessage, isCoreError } from '../api'
  import type {
    BrowseRequest,
    CellInput,
    ColumnMeta,
    ColumnValue,
    Filter,
    Row,
    SessionInfo,
    Sort,
    TableRef,
    TableStructure,
    Value,
  } from '../types'
  import DataGrid from '../grid/DataGrid.svelte'
  import type { EditorContext, GridSelection } from '../grid/types'
  import Modal from '../ui/Modal.svelte'
  import { formatCount, isTruncatedValue, valueToEditText } from '../format'
  import {
    ChangeStore,
    DEFAULT_MARKER,
    inputToValue,
    isChangeDraft,
    keyString,
    valueToInput,
    type InsertedRow,
  } from './changes.svelte'
  import { Pager } from './pager'
  import { toFilters, type FilterRow } from './filters'
  import FilterBar from './FilterBar.svelte'
  import StructureView from './StructureView.svelte'
  import RelationsView from './RelationsView.svelte'
  import { inspector } from '../stores/inspector.svelte'
  import { pulse } from '../stores/pulse.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import RowMenu from './RowMenu.svelte'
  import type { RowMenuItem } from './menu'
  import CellEditor from './editors/CellEditor.svelte'
  import JsonModal from './editors/JsonModal.svelte'
  import TextModal from './editors/TextModal.svelte'
  import BinaryModal from './editors/BinaryModal.svelte'

  interface Props {
    session: SessionInfo
    table: TableRef
    active: boolean
    /** Tab id, so the inspector knows which view published the row. */
    tabId?: string
    /** Filters applied on first load (e.g. "View related" from the inspector). */
    initialFilters?: Filter[]
    onExport?: (request: BrowseRequest) => void
    onImport?: (table: TableRef) => void
  }

  let { session, table, active, tabId = '', initialFilters, onExport, onImport }: Props = $props()

  const PAGE_SIZE = 200
  const MAX_PAGES = 100
  const FETCH_THROTTLE_MS = 80
  const MAX_BULK_CELLS = 5000
  const LONG_TEXT_CHARS = 160

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  type CountState = 'pending' | 'exact' | 'estimate' | 'unknown'

  let tab = $state<'data' | 'structure' | 'relations'>('data')
  let quick = $state('')
  let structure = $state.raw<TableStructure | null>(null)
  let structureError = $state<string | null>(null)
  let columns = $state.raw<ColumnMeta[]>([])
  let total = $state(0)
  let countState = $state<CountState>('pending')
  let loadedEnd = $state(0)
  let loading = $state(true)
  let loadError = $state<string | null>(null)
  let lastMs = $state<number | null>(null)
  let lastQuery = $state('')
  let rev = $state(0)
  let sort = $state.raw<Sort[]>([])
  let committing = $state(false)
  let selection = $state.raw<GridSelection | null>(null)

  // filters: the editable rows, and what was last applied
  let showFilters = $state(false)
  let filterRows = $state<FilterRow[]>([])
  let matchAll = $state(true)
  let rawMode = $state(false)
  let rawWhere = $state('')
  let applied = $state.raw<{ filters: Filter[]; matchAll: boolean; rawWhere: string | null }>({
    filters: [],
    matchAll: true,
    rawWhere: null,
  })
  let filterBar: ReturnType<typeof FilterBar> | undefined = $state()

  const store = new ChangeStore()
  let grid: ReturnType<typeof DataGrid> | undefined = $state()

  // dialogs
  interface ErrorInfo {
    title: string
    message: string
    statement?: string | null
    code?: string | null
  }
  let previewStatements = $state.raw<string[] | null>(null)
  let confirmCommit = $state(false)
  let errorInfo = $state.raw<ErrorInfo | null>(null)
  let draftPrompt = $state.raw<{ draft: ReturnType<ChangeStore['toDraft']>; count: number } | null>(null)
  let viewer = $state.raw<{
    kind: 'json' | 'text' | 'binary'
    row: number
    col: number
    value: Value | undefined
    readOnly: boolean
    fetchFull?: () => Promise<Value>
  } | null>(null)
  let menu = $state.raw<{ x: number; y: number; items: RowMenuItem[] } | null>(null)
  let flash = $state.raw<{ text: string; error: boolean } | null>(null)
  let copiedSql = $state(false)

  let pager: Pager = new Pager({ pageSize: PAGE_SIZE, maxPages: MAX_PAGES, load: () => Promise.reject(new Error('not ready')) })
  let dataGen = 0
  let lastNeed: [number, number] | null = null
  let lastFetchAt = 0
  let fetchTimer: ReturnType<typeof setTimeout> | undefined
  let flashTimer: ReturnType<typeof setTimeout> | undefined
  let copyTimer: ReturnType<typeof setTimeout> | undefined
  let draftReady = false

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  const engine = $derived(session.config.engine)
  const tableKey = $derived(
    [session.sessionId, table.database, table.schema ?? '', table.name].join('\u0000'),
  )
  const colIndex = $derived(new Map(columns.map((c, i) => [c.name, i])))
  const colInfo = $derived(new Map((structure?.columns ?? []).map((c) => [c.name, c])))
  const pkIdx = $derived(
    structure && columns.length > 0 ? structure.primaryKey.map((n) => colIndex.get(n) ?? -1) : [],
  )
  const pkColumns = $derived(new Set(structure?.primaryKey ?? []))

  const readOnlyReason = $derived.by<string | null>(() => {
    if (session.config.readOnly) return 'This connection is read-only.'
    const s = structure
    if (!s) return null
    if (s.editability.kind === 'readOnly') return s.editability.reason
    if (s.primaryKey.length === 0) return 'This table has no primary key, so rows cannot be identified for editing.'
    if (columns.length > 0 && pkIdx.includes(-1)) return 'The primary key columns are not part of the result.'
    return null
  })
  const editable = $derived(structure !== null && readOnlyReason === null && !committing)

  const request = $derived<BrowseRequest>({
    table,
    filters: applied.filters,
    matchAll: applied.matchAll,
    rawWhere: applied.rawWhere,
    sort,
    columns: [],
  })
  const appliedCount = $derived(applied.filters.length + (applied.rawWhere ? 1 : 0))
  const rowCount = $derived(store.insertCount + total)

  const countText = $derived.by(() => {
    switch (countState) {
      case 'pending':
        return 'Counting…'
      case 'exact':
        return `${formatCount(total)} rows`
      case 'estimate':
        return `≈${formatCount(total)} rows`
      default:
        return `${formatCount(loadedEnd)}+ rows`
    }
  })

  const selectionText = $derived.by(() => {
    const s = selection
    if (!s) return ''
    const rows = s.row1 - s.row0 + 1
    const cols = s.col1 - s.col0 + 1
    if (rows === 1 && cols === 1) return ''
    return `${formatCount(rows)} × ${cols} selected`
  })

  const summary = $derived(store.summary)
  const pendingText = $derived.by(() => {
    const parts: string[] = []
    if (summary.updates) parts.push(`${summary.updates} edited`)
    if (summary.inserts) parts.push(`${summary.inserts} new`)
    if (summary.deletes) parts.push(`${summary.deletes} deleted`)
    return parts.join(', ')
  })

  const importDisabled = $derived(
    !onImport || session.config.readOnly || structure?.editability.kind === 'readOnly',
  )

  // ---------------------------------------------------------------------------
  // Loading
  // ---------------------------------------------------------------------------

  function toErrorInfo(title: string, e: unknown): ErrorInfo {
    if (isCoreError(e)) return { title, message: e.message, statement: e.statement, code: e.code }
    return { title, message: errorMessage(e) }
  }

  function showFlash(text: string, error = false, ms = 4500) {
    flash = { text, error }
    clearTimeout(flashTimer)
    flashTimer = setTimeout(() => (flash = null), ms)
  }

  function buildRequest(): BrowseRequest {
    return {
      table,
      filters: applied.filters,
      matchAll: applied.matchAll,
      rawWhere: applied.rawWhere,
      sort,
      columns: [],
    }
  }

  function setColumns(next: ColumnMeta[]) {
    const sig = (cs: ColumnMeta[]) => cs.map((c) => c.name + '\u0001' + c.typeName).join('\u0002')
    if (sig(next) !== sig(columns)) columns = next
  }

  async function loadStructure() {
    const key = tableKey
    try {
      const s = await api.tableStructure(session.sessionId, table)
      if (key !== tableKey) return
      structure = s
      structureError = null
    } catch (e) {
      if (key === tableKey) structureError = errorMessage(e)
    }
  }

  function resetData(keepPosition: boolean) {
    const req = buildRequest()
    const sid = session.sessionId
    const gen = ++dataGen
    clearTimeout(fetchTimer)
    fetchTimer = undefined
    const p = new Pager({
      pageSize: PAGE_SIZE,
      maxPages: MAX_PAGES,
      load: (offset, limit) => api.browsePage(sid, req, offset, limit),
    })
    p.onPage = (page, idx) => {
      if (gen !== dataGen) return
      lastMs = page.elapsedMs
      lastQuery = page.query
      pulse.record(sid, page.elapsedMs)
      if (idx === 0 || columns.length === 0 || page.columns.length !== columns.length) setColumns(page.columns)
      loadError = null
      loading = false
      const end = page.offset + page.rows.length
      loadedEnd = Math.max(loadedEnd, end)
      if (page.rows.length < PAGE_SIZE) {
        total = end
        countState = 'exact'
      } else if (countState !== 'exact' && end >= total) {
        // The count is unknown or only an estimate: keep room below the last loaded page.
        total = end + PAGE_SIZE
      }
      rev++
    }
    p.onError = (e, idx) => {
      if (gen !== dataGen) return
      loading = false
      loadError = errorMessage(e)
      if (idx === 0 && countState === 'pending') countState = 'unknown'
    }
    pager = p
    total = 0
    loadedEnd = 0
    countState = 'pending'
    loadError = null
    loading = true
    if (!keepPosition) lastNeed = null
    rev++

    void p.ensure(0, PAGE_SIZE)
    if (keepPosition && lastNeed) void p.ensure(lastNeed[0], lastNeed[1])

    api.browseCount(sid, req).then(
      (c) => {
        if (gen !== dataGen) return
        if (c.exact !== null) {
          total = c.exact
          countState = 'exact'
        } else if (countState !== 'exact') {
          if (c.estimate !== null) {
            total = Math.max(c.estimate, loadedEnd)
            countState = 'estimate'
          } else {
            countState = 'unknown'
          }
        }
        rev++
      },
      () => {
        if (gen === dataGen && countState === 'pending') countState = 'unknown'
      },
    )
  }

  function refresh() {
    resetData(true)
    void loadStructure()
  }

  function init() {
    store.clear()
    draftReady = false
    structure = null
    structureError = null
    columns = []
    sort = []
    tab = 'data'
    filterRows = []
    matchAll = true
    rawMode = false
    rawWhere = ''
    applied = { filters: initialFilters ? [...initialFilters] : [], matchAll: true, rawWhere: null }
    quick = ''
    showFilters = false
    selection = null
    viewer = null
    menu = null
    previewStatements = null
    errorInfo = null
    draftPrompt = null
    lastMs = null
    lastQuery = ''
    resetData(false)
    void loadStructure().then(checkDraft)
  }

  $effect(() => {
    void tableKey
    untrack(init)
  })

  onDestroy(() => {
    dataGen++
    clearTimeout(fetchTimer)
    clearTimeout(flashTimer)
    clearTimeout(copyTimer)
  })

  // The grid asks for display rows; inserted rows sit above the loaded data.
  function onNeedRows(start: number, end: number) {
    const ins = store.insertCount
    lastNeed = [Math.max(0, start - ins), Math.max(0, end - ins)]
    if (lastNeed[1] <= lastNeed[0]) return
    if (pager.covers(lastNeed[0], lastNeed[1])) {
      void pager.ensure(lastNeed[0], lastNeed[1])
      return
    }
    const wait = FETCH_THROTTLE_MS - (performance.now() - lastFetchAt)
    if (wait <= 0 || pager.loadedPages === 0) fetchNeeded()
    else if (!fetchTimer) fetchTimer = setTimeout(fetchNeeded, wait)
  }

  function fetchNeeded() {
    fetchTimer = undefined
    lastFetchAt = performance.now()
    if (lastNeed) void pager.ensure(lastNeed[0], lastNeed[1])
  }

  function retryLoad() {
    pager.clearFailures()
    loadError = null
    if (lastNeed) void pager.ensure(lastNeed[0], lastNeed[1])
    void pager.ensure(0, PAGE_SIZE)
  }

  // ---------------------------------------------------------------------------
  // Filters and sorting
  // ---------------------------------------------------------------------------

  function applyFilters() {
    applied = {
      filters: toFilters(filterRows),
      matchAll,
      rawWhere: rawMode && rawWhere.trim() !== '' ? rawWhere.trim() : null,
    }
    resetData(false)
    grid?.scrollToRow(0)
  }

  function clearFilters() {
    filterRows = []
    rawWhere = ''
    rawMode = false
    matchAll = true
    applyFilters()
  }

  // Quick "Filter rows…" box: substring match on every text-like column, pushed down to the server.
  const QUICK_FAMILIES = new Set(['text', 'enum', 'uuid', 'objectId', 'json', 'date', 'dateTime', 'other'])

  function applyQuick() {
    const q = quick.trim()
    const cols = columns.filter((c) => QUICK_FAMILIES.has(c.family)).map((c) => c.name)
    applied = q && cols.length
      ? { filters: cols.map((column) => ({ column, op: 'contains' as const, value: q })), matchAll: false, rawWhere: null }
      : { filters: [], matchAll: true, rawWhere: null }
    filterRows = []
    rawMode = false
    resetData(false)
    grid?.scrollToRow(0)
  }

  function openRelated(ref: TableRef) {
    tabs.openTable(session.sessionId, ref)
  }

  // Publish the focused row to the Row inspector while this tab is visible.
  $effect(() => {
    void rev
    void store.version
    const sel = selection
    if (!active || !tabId || tab !== 'data' || !sel || columns.length === 0) return
    const row = getRow(sel.activeRow)
    if (!row) return
    inspector.show({
      tabId,
      session,
      table,
      columns,
      row,
      rowIndex: sel.activeRow,
      primaryKey: structure?.primaryKey ?? [],
      foreignKeys: structure?.foreignKeys ?? [],
      editable,
    })
  })

  onDestroy(() => {
    if (tabId) inspector.clear(tabId)
  })

  async function toggleFilters() {
    showFilters = !showFilters
    if (showFilters && filterRows.length === 0 && !rawMode) {
      await tick()
      filterBar?.addRow()
    }
  }

  function onSort(column: string, descending: boolean, off?: boolean) {
    sort = off ? [] : [{ column, descending }]
    resetData(false)
    grid?.scrollToRow(0)
  }

  function filterByCell(r: number, c: number) {
    const v = getRow(r)?.[c]
    const column = columns[c].name
    const row: FilterRow =
      v === null
        ? { id: Date.now(), column, op: 'isNull', value: '' }
        : { id: Date.now(), column, op: 'eq', value: valueToEditText(v) }
    filterRows = [...filterRows, row]
    showFilters = true
    applyFilters()
  }

  // ---------------------------------------------------------------------------
  // Rows as the grid sees them (loaded data + pending changes)
  // ---------------------------------------------------------------------------

  let memoRow: Row | undefined
  let memoKey: string | null = null

  function keyOfRow(row: Row): string | null {
    if (row === memoRow) return memoKey
    memoRow = row
    memoKey = pkIdx.length > 0 && !pkIdx.includes(-1) ? JSON.stringify(pkIdx.map((i) => row[i])) : null
    return memoKey
  }

  function keyValues(row: Row): ColumnValue[] {
    const names = structure?.primaryKey ?? []
    return names.map((column, i) => ({ column, value: row[pkIdx[i]] }))
  }

  function insertedValues(ir: InsertedRow | undefined): Row | undefined {
    if (!ir) return undefined
    return columns.map((col) => {
      const input = ir.values.get(col.name)
      return input ? inputToValue(input, col.family) : DEFAULT_MARKER
    })
  }

  function getRow(index: number): Row | undefined {
    const ins = store.insertCount
    if (index < ins) return insertedValues(store.insertedAt(index))
    const row = pager.get(index - ins)
    if (!row || !store.hasRowChanges) return row
    const ks = keyOfRow(row)
    const edits = ks === null ? undefined : store.rowEdit(ks)
    if (!edits || edits.size === 0) return row
    const out = row.slice()
    for (const [name, e] of edits) {
      const i = colIndex.get(name)
      if (i !== undefined) out[i] = inputToValue(e.input, columns[i].family)
    }
    return out
  }

  function loadedRow(index: number): Row | undefined {
    return pager.get(index - store.insertCount)
  }

  function rowClass(r: number): string {
    if (r < store.insertCount) return 'inserted'
    if (!store.hasRowChanges) return ''
    const row = loadedRow(r)
    const ks = row ? keyOfRow(row) : null
    return ks !== null && store.isDeleted(ks) ? 'deleted' : ''
  }

  function cellClass(r: number, c: number): string {
    if (r < store.insertCount || !store.hasRowChanges) return ''
    const row = loadedRow(r)
    const ks = row ? keyOfRow(row) : null
    return ks !== null && store.editFor(ks, columns[c].name) ? 'edited' : ''
  }

  function rowLabel(r: number): string {
    const ins = store.insertCount
    return r < ins ? '+' : String(r - ins + 1)
  }

  function columnHint(c: number): string | undefined {
    return pkColumns.has(columns[c]?.name) ? 'PK' : undefined
  }

  // ---------------------------------------------------------------------------
  // Editing
  // ---------------------------------------------------------------------------

  function canEditCell(r: number, c: number): boolean {
    if (!editable) return false
    const info = colInfo.get(columns[c].name)
    if (info?.isGenerated) return false
    if (r < store.insertCount) return true
    const row = loadedRow(r)
    if (!row) return false
    const ks = keyOfRow(row)
    return ks !== null && !store.isDeleted(ks)
  }

  function commitCell(r: number, c: number, input: CellInput) {
    const column = columns[c].name
    const ins = store.insertCount
    if (r < ins) {
      const ir = store.insertedAt(r)
      if (ir) store.setInsertCell(ir.id, column, input)
      return
    }
    const row = pager.get(r - ins)
    if (!row || keyOfRow(row) === null) return
    store.setCell(keyValues(row), column, row[c], input)
  }

  function fetcher(r: number, column: string): (() => Promise<Value>) | undefined {
    if (r < store.insertCount) return undefined
    const row = loadedRow(r)
    if (!row || keyOfRow(row) === null) return undefined
    const key = keyValues(row)
    return () => api.cellFetch(session.sessionId, table, key, column)
  }

  function isJsonValue(v: Value | undefined): boolean {
    return typeof v === 'object' && v !== null && '$json' in v
  }

  function isBytesValue(v: Value | undefined): boolean {
    return typeof v === 'object' && v !== null && '$bytes' in v
  }

  function openViewer(kind: 'json' | 'text' | 'binary', r: number, c: number, readOnly: boolean) {
    viewer = {
      kind,
      row: r,
      col: c,
      value: getRow(r)?.[c],
      readOnly,
      fetchFull: fetcher(r, columns[c].name),
    }
  }

  // Returns true when the cell is handled by a dialog instead of the inline editor.
  function onEditCell(r: number, c: number, initial?: string): boolean {
    const col = columns[c]
    const v = getRow(r)?.[c]
    if (v === undefined) return true
    const ro = !canEditCell(r, c)
    if (col.family === 'json' || col.family === 'document' || isJsonValue(v)) {
      openViewer('json', r, c, ro)
      return true
    }
    if (col.family === 'binary' || isBytesValue(v)) {
      openViewer('binary', r, c, ro)
      return true
    }
    const longText =
      isTruncatedValue(v) ||
      (typeof v === 'string' && initial === undefined && (v.length > LONG_TEXT_CHARS || v.includes('\n')))
    if (longText) {
      openViewer('text', r, c, ro)
      return true
    }
    return false
  }

  function onViewerSave(input: CellInput) {
    const v = viewer
    viewer = null
    if (v) commitCell(v.row, v.col, input)
    void tick().then(() => grid?.focus())
  }

  function closeViewer() {
    viewer = null
    void tick().then(() => grid?.focus())
  }

  function forEachSelectedCell(fn: (r: number, c: number) => void, limit = MAX_BULK_CELLS) {
    const s = selection
    if (!s) return
    let n = 0
    for (let r = s.row0; r <= s.row1; r++) {
      for (let c = s.col0; c <= s.col1; c++) {
        if (n++ >= limit) return
        fn(r, c)
      }
    }
  }

  function setNullSelected() {
    if (!editable) return
    let skipped = ''
    forEachSelectedCell((r, c) => {
      const name = columns[c].name
      const info = colInfo.get(name)
      if (!canEditCell(r, c)) return
      if (info && !info.nullable) {
        skipped = name
        return
      }
      commitCell(r, c, { kind: 'null' })
    })
    if (skipped) showFlash(`${skipped} is NOT NULL.`, true)
  }

  async function addRow() {
    if (!editable) return
    store.addInsert()
    await tick()
    grid?.scrollToRow(0)
    grid?.selectCell(0, 0)
    grid?.focus()
  }

  function selectedRowIndexes(limit = 50_000): number[] {
    const s = selection
    if (!s) return []
    const out: number[] = []
    for (let r = s.row0; r <= s.row1 && out.length < limit; r++) out.push(r)
    return out
  }

  function deleteSelected() {
    if (!editable) return
    const ins = store.insertCount
    const insertedIds: number[] = []
    const keys: ColumnValue[][] = []
    for (const r of selectedRowIndexes()) {
      if (r < ins) {
        const ir = store.insertedAt(r)
        if (ir) insertedIds.push(ir.id)
      } else {
        const row = pager.get(r - ins)
        if (row && keyOfRow(row) !== null) keys.push(keyValues(row))
      }
    }
    const restoring = keys.length > 0 && keys.every((k) => store.isDeleted(keyString(k)))
    for (const k of keys) store.setDeleted(k, !restoring)
    if (insertedIds.length > 0) {
      for (const id of insertedIds) store.removeInsert(id)
      grid?.clearSelection()
    }
  }

  async function duplicateSelected() {
    if (!editable) return
    const rows = selectedRowIndexes(200)
    let last = -1
    for (const r of rows) {
      const display = getRow(r)
      if (!display) continue
      const values = new Map<string, CellInput>()
      columns.forEach((col, c) => {
        const info = colInfo.get(col.name)
        if (info?.isGenerated || info?.isAutoIncrement) return
        const input = valueToInput(display[c])
        if (input) values.set(col.name, input)
      })
      store.addInsert(values)
      last = r
    }
    if (last >= 0) {
      await tick()
      grid?.scrollToRow(0)
      grid?.selectCell(0, 0)
      grid?.focus()
    }
  }

  function discard() {
    store.clear()
    grid?.clearSelection()
  }

  // ---------------------------------------------------------------------------
  // Preview and commit
  // ---------------------------------------------------------------------------

  async function previewSql() {
    if (store.isEmpty) return
    try {
      previewStatements = await api.changesPreview(session.sessionId, store.buildChangeSet(table))
    } catch (e) {
      errorInfo = toErrorInfo('Could not build the SQL', e)
    }
  }

  const previewText = $derived(
    (previewStatements ?? []).map((s) => (/;\s*$/.test(s) ? s : s + ';')).join('\n\n'),
  )

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      return false
    }
  }

  async function copyPreview() {
    if (await copy(previewText)) {
      copiedSql = true
      clearTimeout(copyTimer)
      copyTimer = setTimeout(() => (copiedSql = false), 1500)
    }
  }

  function requestCommit() {
    if (store.isEmpty || committing || !structure) return
    if (session.config.env === 'production') confirmCommit = true
    else void commit()
  }

  async function commit() {
    confirmCommit = false
    if (store.isEmpty || committing) return
    committing = true
    try {
      const res = await api.changesApply(session.sessionId, store.buildChangeSet(table))
      store.clear()
      grid?.clearSelection()
      showFlash(
        `Committed ${res.statements} statement${res.statements === 1 ? '' : 's'}, ${formatCount(res.rowsAffected)} row${res.rowsAffected === 1 ? '' : 's'} affected.`,
      )
      resetData(true)
    } catch (e) {
      // Everything was rolled back: keep the pending changes so nothing is lost.
      errorInfo = toErrorInfo('Commit failed, nothing was changed', e)
    } finally {
      committing = false
    }
  }

  // ---------------------------------------------------------------------------
  // Drafts
  // ---------------------------------------------------------------------------

  function draftKeys(): string[] {
    const t = `${table.database}.${table.schema ?? ''}.${table.name}`
    // Session ids can change between launches; the connection id lets a draft survive a restart.
    return [...new Set([`tablesoft:draft:${session.sessionId}:${t}`, `tablesoft:draft:${session.config.id}:${t}`])]
  }

  function checkDraft() {
    try {
      for (const key of draftKeys()) {
        const raw = localStorage.getItem(key)
        if (!raw) continue
        const parsed: unknown = JSON.parse(raw)
        if (!isChangeDraft(parsed)) continue
        const count = parsed.inserts.length + parsed.edits.length + parsed.deletes.length
        if (count > 0) {
          draftPrompt = { draft: parsed, count }
          return
        }
      }
    } catch {
      // unreadable draft or storage blocked: start clean
    }
    draftReady = true
  }

  function restoreDraft() {
    if (draftPrompt) store.restore(draftPrompt.draft)
    draftPrompt = null
    draftReady = true
  }

  function dropDraft() {
    draftPrompt = null
    draftReady = true
    clearDraftStorage()
  }

  function clearDraftStorage() {
    try {
      for (const key of draftKeys()) localStorage.removeItem(key)
    } catch {
      // storage blocked
    }
  }

  $effect(() => {
    void store.version
    untrack(() => {
      if (!draftReady) return
      try {
        if (store.isEmpty) {
          clearDraftStorage()
        } else {
          const json = JSON.stringify(store.toDraft())
          for (const key of draftKeys()) localStorage.setItem(key, json)
        }
      } catch {
        // storage full or blocked: the draft is a convenience only
      }
    })
  })

  // ---------------------------------------------------------------------------
  // Keyboard and menus
  // ---------------------------------------------------------------------------

  const modalOpen = $derived(
    previewStatements !== null || confirmCommit || errorInfo !== null || draftPrompt !== null || viewer !== null,
  )

  function onWindowKey(e: KeyboardEvent) {
    if (!active || modalOpen) return
    const mod = e.metaKey || e.ctrlKey
    if (!mod || e.altKey) return
    const key = e.key.toLowerCase()
    if (key === 's' && !e.shiftKey) {
      e.preventDefault()
      requestCommit()
    } else if (key === 'r' && !e.shiftKey) {
      e.preventDefault()
      if (tab === 'data') refresh()
    } else if (key === 'f' && !e.shiftKey && tab === 'data') {
      e.preventDefault()
      void toggleFilters()
    }
  }

  // Keys pressed while the grid itself has focus.
  function onGridKey(e: KeyboardEvent): boolean {
    const mod = e.metaKey || e.ctrlKey
    if (mod && !e.altKey && ((e.shiftKey && e.key.toLowerCase() === 'n') || e.key === 'Backspace')) {
      if (editable) setNullSelected()
      return true
    }
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'd') {
      void duplicateSelected()
      return true
    }
    if (!mod && !e.altKey && (e.key === 'Delete' || e.key === 'Backspace')) {
      if (editable) deleteSelected()
      return true
    }
    return false
  }

  function onContextMenu(e: MouseEvent, r: number, c: number) {
    const v = getRow(r)?.[c]
    const ins = store.insertCount
    const canEdit = canEditCell(r, c)
    const loaded = v !== undefined
    const row = r < ins ? undefined : loadedRow(r)
    const rowKey = row ? keyOfRow(row) : null
    const isDeleted = rowKey !== null && store.isDeleted(rowKey)
    const info = colInfo.get(columns[c].name)
    const items: RowMenuItem[] = [
      { label: 'Edit cell', hint: 'Enter', disabled: !canEdit, onclick: () => grid?.edit(r, c) },
      {
        label: 'Set NULL',
        hint: '⌘⇧N',
        disabled: !canEdit || (info !== undefined && !info.nullable),
        onclick: setNullSelected,
      },
      { separator: true },
      { label: 'Copy', hint: '⌘C', disabled: !loaded, onclick: () => void grid?.copySelection(false) },
      { label: 'Copy with headers', disabled: !loaded, onclick: () => void grid?.copySelection(true) },
      { label: 'Filter by this value', disabled: !loaded || r < ins, onclick: () => filterByCell(r, c) },
      { separator: true },
      { label: 'Duplicate row', hint: '⌘D', disabled: !editable || !loaded, onclick: () => void duplicateSelected() },
      {
        label: r < ins ? 'Remove new row' : isDeleted ? 'Restore row' : 'Delete row',
        hint: '⌫',
        danger: !isDeleted,
        disabled: !editable || !loaded,
        onclick: deleteSelected,
      },
    ]
    menu = { x: e.clientX, y: e.clientY, items }
  }

  function closeMenu() {
    menu = null
    grid?.focus()
  }

  $effect(() => {
    // Focus the grid when this tab becomes the visible one.
    if (active && tab === 'data' && !modalOpen) untrack(() => void tick().then(() => grid?.focus()))
  })
</script>

<svelte:window onkeydown={onWindowKey} />

<div class="tv">
  {#if readOnlyReason && tab === 'data'}
    <div class="banner info"><Icon name="lock" size={12} /> {readOnlyReason}</div>
  {/if}
  {#if session.config.env === 'production' && editable && tab === 'data'}
    <div class="banner warn">Production connection: commits ask for confirmation.</div>
  {/if}
  {#if loadError}
    <div class="banner error">
      <span class="msg">{loadError}</span>
      <button onclick={retryLoad}>Retry</button>
    </div>
  {/if}

  {#if showFilters && tab === 'data'}
    <FilterBar
      bind:this={filterBar}
      columns={columns.map((c) => c.name)}
      bind:rows={filterRows}
      bind:matchAll
      bind:rawMode
      bind:rawWhere
      rawPlaceholder={engine === 'mongodb' ? '{ "status": "active" }' : "status = 'active' AND created_at > '2024-01-01'"}
      onApply={applyFilters}
      onClear={clearFilters}
      onChange={() => {}}
    />
  {/if}

  <div class="main">
    <div class="pane" hidden={tab !== 'data'}>
      {#if columns.length === 0 && (loading || loadError)}
        <div class="center muted">{loadError ? 'Could not load the table.' : 'Loading…'}</div>
      {:else}
        <DataGrid
          bind:this={grid}
          {columns}
          {rowCount}
          {getRow}
          {onNeedRows}
          revision={rev + store.version}
          {sort}
          {onSort}
          {editable}
          {canEditCell}
          {onEditCell}
          onCommitCell={commitCell}
          {rowClass}
          {cellClass}
          {rowLabel}
          {columnHint}
          onSelectionChange={(s) => (selection = s)}
          {onContextMenu}
          onKeyDown={onGridKey}
          emptyMessage={loading ? 'Loading…' : appliedCount > 0 ? 'No rows match the filter' : 'This table is empty'}
        >
          {#snippet editor(ctx: EditorContext)}
            <CellEditor {ctx} info={colInfo.get(ctx.column.name)} {engine} />
          {/snippet}
        </DataGrid>
      {/if}
    </div>

    {#if tab === 'relations'}
      <div class="pane">
        {#if structure}
          <RelationsView {structure} onOpen={openRelated} />
        {:else}
          <div class="center muted">Loading…</div>
        {/if}
      </div>
    {/if}

    {#if tab === 'structure'}
      <div class="pane">
        {#if structure}
          <StructureView {structure} />
        {:else if structureError}
          <div class="center error-text">{structureError}</div>
        {:else}
          <div class="center muted">Loading…</div>
        {/if}
      </div>
    {/if}
  </div>

  <div class="bottombar">
    <div class="seg" role="tablist">
      <button role="tab" aria-selected={tab === 'data'} class:on={tab === 'data'} onclick={() => (tab = 'data')}>Data</button>
      <button role="tab" aria-selected={tab === 'structure'} class:on={tab === 'structure'} onclick={() => (tab = 'structure')}>Structure</button>
      <button role="tab" aria-selected={tab === 'relations'} class:on={tab === 'relations'} onclick={() => (tab = 'relations')}>Relations</button>
    </div>

    {#if tab === 'data'}
      <button class="bb" disabled={!editable} title="Add a row" onclick={addRow}><Icon name="plus" size={12} strokeWidth={1.8} /> Row</button>
      <button class="bb io" disabled={!editable || !selection} title="Delete selected rows (Delete)" aria-label="Delete selected rows" onclick={deleteSelected}><Icon name="trash" size={13} /></button>
    {/if}

    <div class="info" title={tab === 'data' ? lastQuery : ''}>
      {#if tab === 'data'}
        {#if loadError}
          <span class="err">Query failed</span>
        {:else}
          <span>{countText}{store.insertCount > 0 ? ` + ${store.insertCount} new` : ''}</span>
          {#if lastMs !== null}<span class="dim">{lastMs < 1 ? '<1' : Math.round(lastMs)} ms</span>{/if}
        {/if}
        {#if selectionText}<span class="dim">{selectionText}</span>{/if}
      {:else if structure}
        <span>{structure.columns.length} columns · {structure.indexes.length} indexes · {structure.foreignKeys.length} foreign keys</span>
      {/if}
      {#if flash}<span class:err={flash.error} class="flash">{flash.text}</span>{/if}
    </div>

    {#if tab === 'data' && !store.isEmpty}
      <div class="changes">
        <span class="pending-label">{store.pending} change{store.pending === 1 ? '' : 's'}</span>
        <button class="bb" onclick={previewSql}>Preview</button>
        <button class="bb" disabled={committing} onclick={discard}>Discard</button>
        <button class="primary sm" disabled={committing || !structure} title="Commit (⌘S)" onclick={requestCommit}>
          {committing ? 'Committing…' : 'Commit'}
        </button>
      </div>
    {/if}

    {#if tab === 'data'}
      <label class="quick">
        <Icon name="search" size={12} />
        <input
          type="search"
          placeholder="Filter rows…"
          bind:value={quick}
          spellcheck="false"
          onkeydown={(e) => e.key === 'Enter' && applyQuick()}
          oninput={() => quick === '' && applied.filters.length > 0 && applyQuick()}
        />
      </label>
      <button class="bb io" class:on={showFilters || (appliedCount > 0 && !quick)} title="Filters (⌘F)" aria-label="Filters" onclick={toggleFilters}>
        <Icon name="filter" size={13} />{#if appliedCount > 0 && !quick}<span class="count">{appliedCount}</span>{/if}
      </button>
      <button class="bb io" title="Refresh (⌘R)" aria-label="Refresh" onclick={refresh}><Icon name="refresh" size={13} /></button>
    {/if}
    <button class="bb io" disabled={!onExport} title="Export…" aria-label="Export" onclick={() => onExport?.(request)}><Icon name="export" size={13} /></button>
    <button class="bb io" disabled={importDisabled} title="Import CSV…" aria-label="Import CSV" onclick={() => onImport?.(table)}><Icon name="import" size={13} /></button>
  </div>
</div>

{#if menu}
  <RowMenu x={menu.x} y={menu.y} items={menu.items} onClose={closeMenu} />
{/if}

{#if viewer}
  {@const v = viewer}
  {@const name = columns[v.col]?.name ?? ''}
  {@const nullable = colInfo.get(name)?.nullable ?? true}
  {#if v.kind === 'json'}
    <JsonModal
      title={`${name} (${v.readOnly ? 'view' : 'edit'} JSON)`}
      value={v.value}
      readOnly={v.readOnly}
      {nullable}
      fetchFull={v.fetchFull}
      onSave={onViewerSave}
      onClose={closeViewer}
    />
  {:else if v.kind === 'text'}
    <TextModal
      title={`${name} (${v.readOnly ? 'view' : 'edit'} text)`}
      value={v.value}
      readOnly={v.readOnly}
      {nullable}
      fetchFull={v.fetchFull}
      onSave={onViewerSave}
      onClose={closeViewer}
    />
  {:else}
    <BinaryModal
      title={`${name} (binary)`}
      value={v.value}
      readOnly={v.readOnly}
      {nullable}
      fetchFull={v.fetchFull}
      onSetNull={() => onViewerSave({ kind: 'null' })}
      onClose={closeViewer}
    />
  {/if}
{/if}

{#if previewStatements !== null}
  <Modal title="Preview SQL" width="720px" onClose={() => (previewStatements = null)}>
    <pre class="sql-block">{previewText}</pre>
    {#snippet footer()}
      <button class="ghost" onclick={copyPreview}>{copiedSql ? 'Copied' : 'Copy'}</button>
      <span class="grow"></span>
      <button onclick={() => (previewStatements = null)}>Close</button>
      <button
        class="primary"
        onclick={() => {
          previewStatements = null
          requestCommit()
        }}>Commit</button
      >
    {/snippet}
  </Modal>
{/if}

{#if confirmCommit}
  <Modal title="Commit to production?" width="460px" onClose={() => (confirmCommit = false)}>
    <p>
      You are about to change data on <strong>{session.config.name}</strong>, a production connection:
    </p>
    <ul>
      {#if summary.updates}<li>{summary.updates} row{summary.updates === 1 ? '' : 's'} updated</li>{/if}
      {#if summary.inserts}<li>{summary.inserts} row{summary.inserts === 1 ? '' : 's'} inserted</li>{/if}
      {#if summary.deletes}<li>{summary.deletes} row{summary.deletes === 1 ? '' : 's'} deleted</li>{/if}
    </ul>
    <p class="muted">Everything runs in one transaction and is rolled back if a statement fails.</p>
    {#snippet footer()}
      <button class="ghost" onclick={previewSql}>Preview SQL</button>
      <span class="grow"></span>
      <button onclick={() => (confirmCommit = false)}>Cancel</button>
      <button class="danger" onclick={commit}>Commit to production</button>
    {/snippet}
  </Modal>
{/if}

{#if errorInfo}
  <Modal title={errorInfo.title} width="640px" onClose={() => (errorInfo = null)}>
    <p class="error-text">{errorInfo.message}{errorInfo.code ? ` (${errorInfo.code})` : ''}</p>
    {#if errorInfo.statement}
      <p class="muted">Failing statement</p>
      <pre class="sql-block">{errorInfo.statement}</pre>
    {/if}
    {#if errorInfo.title.startsWith('Commit failed')}
      <p class="muted">Your pending changes are still here. Fix the problem and commit again.</p>
    {/if}
    {#snippet footer()}
      {#if errorInfo?.statement}
        <button class="ghost" onclick={() => errorInfo?.statement && copy(errorInfo.statement)}>Copy statement</button>
      {/if}
      <span class="grow"></span>
      <button class="primary" onclick={() => (errorInfo = null)}>Close</button>
    {/snippet}
  </Modal>
{/if}

{#if draftPrompt}
  <Modal title="Unsaved changes found" width="460px" dismissable={false} onClose={dropDraft}>
    <p>
      {draftPrompt.count} pending change{draftPrompt.count === 1 ? '' : 's'} on
      <strong>{table.name}</strong> were never committed. Restore them?
    </p>
    {#snippet footer()}
      <button onclick={dropDraft}>Discard</button>
      <button class="primary" onclick={restoreDraft}>Restore</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .tv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }

  .grow {
    flex: 1;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
  }

  .banner.info {
    background: var(--bg-sidebar);
    color: var(--fg-muted);
  }

  .banner.warn {
    background: rgb(232 130 12 / 0.12);
    color: var(--warning);
  }

  .banner.error {
    background: rgb(217 48 37 / 0.1);
    color: var(--danger);
  }

  .banner .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
    -webkit-user-select: text;
  }

  .main {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
  }

  .pane {
    position: absolute;
    inset: 0;
  }

  .pane[hidden] {
    display: none;
  }

  .center {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }


  .bottombar {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 36px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    background: var(--bg-chrome);
    font-size: 11px;
    flex: 0 0 auto;
    white-space: nowrap;
  }

  .bottombar .seg {
    margin-right: 6px;
    padding: 2px;
    gap: 2px;
  }

  .bottombar .seg button {
    min-height: 22px;
    padding: 0 10px;
    font-size: 11px;
  }

  .bb {
    position: relative;
    gap: 4px;
    min-height: 26px;
    height: 26px;
    padding: 0 8px;
    border: none;
    background: transparent;
    box-shadow: none;
    border-radius: 6px;
    font-size: 11px;
    color: var(--fg-muted);
  }

  .bb.io {
    width: 28px;
    padding: 0;
  }

  .bb:hover:not(:disabled),
  .bb.on {
    background: var(--bg-active);
    color: var(--fg);
  }

  .count {
    position: absolute;
    top: 0;
    right: -1px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: 7px;
    font-size: 9px;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
    background: var(--accent);
    color: var(--accent-fg);
  }

  .info {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: center;
    gap: 12px;
    overflow: hidden;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
  }

  .info .dim {
    color: var(--fg-muted);
  }

  .info .flash {
    color: var(--success);
  }

  .info .err,
  .info .flash.err {
    color: var(--danger);
  }

  .changes {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 26px;
    margin-right: 4px;
    padding: 0 2px 0 9px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--warning) 14%, transparent);
  }

  .pending-label {
    margin-right: 4px;
    font-weight: 650;
    color: var(--warning);
    font-variant-numeric: tabular-nums;
  }

  .primary.sm {
    min-height: 22px;
    height: 22px;
    padding: 0 10px;
    font-size: 11px;
  }

  .quick {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 170px;
    height: 26px;
    margin: 0 2px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--fg-faint);
  }

  .quick:focus-within {
    border-color: var(--accent);
  }

  .quick input {
    flex: 1;
    width: 100%;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
  }

  .quick input:focus {
    box-shadow: none;
  }

  .sql-block {
    margin: 0 0 8px;
    padding: 10px 12px;
    max-height: 50vh;
    overflow: auto;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  p {
    margin: 0 0 8px;
  }

  ul {
    margin: 0 0 8px;
    padding-left: 20px;
  }
</style>
