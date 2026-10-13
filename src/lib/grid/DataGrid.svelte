<!--
  DataGrid: virtualised, read-only or editable result grid.

  One scroll container holds a spacer that gives the scrollbar its range and a sticky
  "window" that never moves; only the visible rows and columns (plus a small overscan)
  are rendered, as plain absolutely positioned elements, and shifted with one transform.
  Beyond ~8M px of total height the scroll range is scaled (scrollTop maps proportionally
  to the row position), so the grid holds any number of rows.

  REQUIRED props
    columns      ColumnMeta[]              header, type family drives alignment
    rowCount     number                    total rows (grows as the parent learns more)
    getRow       (index) => Row|undefined  undefined = not loaded yet (placeholder is drawn)
    onNeedRows   (start, end) => void      visible range + overscan, end exclusive; called whenever
                                           it changes (and after a column-set change)
    revision     number                    bump when rows changed / arrived to repaint

  OPTIONAL props
    sort         Sort[]                    shows the sort arrow of the first matching column
    onSort       (column, descending, off?) => void
                                           header click cycles asc -> desc -> off (off === true)
    editable     boolean                   enables cell editing (double click / Enter / typing)
    canEditCell  (row, col) => boolean     veto editing of single cells
    onEditCell   (row, col, initial?) => boolean|void
                                           called on double click / Enter (also when read-only);
                                           return true when handled (e.g. a modal was opened)
    onCommitCell (row, col, input: CellInput) => void
    editor       Snippet<[EditorContext]>  renders the inline editor inside the cell box
    rowClass     (row) => string           extra classes on a row element (inserted, deleted)
    cellClass    (row, col) => string      extra classes on a cell (edited)
    rowLabel     (row) => string           gutter text (default: row + 1)
    columnHint   (col) => string|undefined small badge in the header (e.g. PK)
    onSelectionChange (sel: GridSelection|null) => void
    onContextMenu (event, row, col) => void
    onKeyDown    (event) => boolean        called first for keys pressed on the focused grid;
                                           return true to swallow the key
    emptyMessage string                    shown when rowCount is 0 (default "No rows")

  Exported methods (bind:this): focus(), edit(row, col), getSelection(), selectCell(row, col), selectRows(from, to),
  scrollToRow(row), copySelection(withHeader?), clearSelection().

  Keyboard: arrows, Shift+arrows (extend), Cmd/Ctrl+arrows (edges), Home/End, PageUp/PageDown,
  Cmd/Ctrl+A, Cmd/Ctrl+C (TSV), Tab / Shift+Tab, Enter / F2 / double click / typing to edit.
-->
<script lang="ts">
  import { untrack, type Snippet } from 'svelte'
  import type { CellInput, ColumnMeta, Row, Sort } from '../types'
  import { formatValue, valueToEditText, statusTone } from '../format'
  import type { EditMove, EditorContext, GridSelection } from './types'
  import {
    HEADER_H,
    ROW_H,
    columnAt,
    prefixSums,
    scrollToVirtual,
    verticalMetrics,
    virtualToScroll,
  } from './layout'

  interface Props {
    columns: ColumnMeta[]
    rowCount: number
    getRow: (index: number) => Row | undefined
    onNeedRows: (start: number, end: number) => void
    revision: number
    sort?: Sort[]
    onSort?: (column: string, descending: boolean, off?: boolean) => void
    editable?: boolean
    canEditCell?: (row: number, col: number) => boolean
    onEditCell?: (row: number, col: number, initial?: string) => boolean | void
    onCommitCell?: (row: number, col: number, input: CellInput) => void
    editor?: Snippet<[EditorContext]>
    rowClass?: (row: number) => string
    cellClass?: (row: number, col: number) => string
    rowLabel?: (row: number) => string
    columnHint?: (col: number) => string | undefined
    onSelectionChange?: (sel: GridSelection | null) => void
    onContextMenu?: (e: MouseEvent, row: number, col: number) => void
    onKeyDown?: (e: KeyboardEvent) => boolean
    emptyMessage?: string
  }

  let {
    columns,
    rowCount,
    getRow,
    onNeedRows,
    revision,
    sort,
    onSort,
    editable = false,
    canEditCell,
    onEditCell,
    onCommitCell,
    editor,
    rowClass,
    cellClass,
    rowLabel,
    columnHint,
    onSelectionChange,
    onContextMenu,
    onKeyDown,
    emptyMessage = 'No rows',
  }: Props = $props()

  const DEFAULT_W = 120
  const MIN_W = 48
  const MAX_INITIAL_W = 400
  const MAX_RESIZE_W = 1600
  const ROW_OVERSCAN = 6
  /** Compact type label for the header: "timestamp with time zone" → "timestamptz". */
  function shortType(t: string): string {
    const x = t.toLowerCase()
    if (x.startsWith('timestamp with time zone')) return 'timestamptz'
    if (x.startsWith('timestamp without time zone')) return 'timestamp'
    if (x.startsWith('character varying')) return 'varchar'
    if (x.startsWith('double precision')) return 'float8'
    return x.replace(/\(.*\)$/, '')
  }

  const COL_OVERSCAN_PX = 240
  const NEED_OVERSCAN = 60
  const MAX_COPY_CELLS = 250_000

  interface Cell {
    r: number
    c: number
  }

  let scroller: HTMLDivElement | undefined = $state()
  let bodyEl: HTMLDivElement | undefined = $state()
  let clientW = $state(0)
  let clientH = $state(0)
  let scrollTop = $state(0)
  let scrollLeft = $state(0)
  let widths = $state.raw<number[]>([])
  let anchor = $state.raw<Cell | null>(null)
  let head = $state.raw<Cell | null>(null)
  let editing = $state.raw<{ id: number; row: number; col: number; initial: string | undefined } | null>(null)
  let editSeq = 0
  let needNonce = $state(0)

  const userSized = new Set<number>()
  let sampled = false
  let rafId = 0
  let resizedAt = 0

  // ---------------------------------------------------------------------------
  // Geometry
  // ---------------------------------------------------------------------------

  const nCols = $derived(columns.length)
  const effWidths = $derived(columns.map((_, i) => widths[i] ?? DEFAULT_W))
  const colLeft = $derived(prefixSums(effWidths))
  const totalW = $derived(colLeft[colLeft.length - 1] ?? 0)
  const gutterW = $derived(Math.max(44, String(rowCount).length * 8 + 24))
  const bodyW = $derived(Math.max(0, clientW - gutterW))
  const bodyH = $derived(Math.max(0, clientH - HEADER_H))
  const vm = $derived(verticalMetrics(rowCount, clientH))
  const vt = $derived(scrollToVirtual(scrollTop, vm))
  const spacerW = $derived(Math.max(gutterW + totalW, clientW))

  const firstVisible = $derived(Math.floor(vt / ROW_H))
  const rowStart = $derived(Math.max(0, firstVisible - ROW_OVERSCAN))
  const rowEnd = $derived(
    Math.min(rowCount - 1, firstVisible + Math.ceil((vt - firstVisible * ROW_H + bodyH) / ROW_H) + ROW_OVERSCAN),
  )
  const rowIdx = $derived.by(() => {
    const out: number[] = []
    for (let r = rowStart; r <= rowEnd; r++) out.push(r)
    return out
  })
  const rowsShift = $derived(vt - rowStart * ROW_H)

  const colStart = $derived.by(() => {
    const c = columnAt(colLeft, Math.max(0, scrollLeft - COL_OVERSCAN_PX))
    return c < 0 ? 0 : c
  })
  const colEnd = $derived.by(() => {
    const c = columnAt(colLeft, scrollLeft + bodyW + COL_OVERSCAN_PX)
    return c < 0 ? nCols - 1 : c
  })
  const colIdx = $derived.by(() => {
    const out: number[] = []
    for (let c = colStart; c <= colEnd; c++) out.push(c)
    return out
  })

  // ---------------------------------------------------------------------------
  // Scrolling
  // ---------------------------------------------------------------------------

  function onScroll() {
    if (rafId) return
    rafId = requestAnimationFrame(() => {
      rafId = 0
      if (!scroller) return
      scrollTop = scroller.scrollTop
      scrollLeft = scroller.scrollLeft
    })
  }

  function setVirtualTop(v: number) {
    if (!scroller) return
    scroller.scrollTop = virtualToScroll(v, vm)
    scrollTop = scroller.scrollTop
  }

  function ensureVisible(r: number, c: number) {
    if (!scroller) return
    if (r * ROW_H < vt) setVirtualTop(r * ROW_H)
    else if ((r + 1) * ROW_H > vt + bodyH) setVirtualTop((r + 1) * ROW_H - bodyH)
    const x0 = colLeft[c] ?? 0
    const x1 = x0 + (effWidths[c] ?? 0)
    if (x0 < scrollLeft) scroller.scrollLeft = x0
    else if (x1 > scrollLeft + bodyW) scroller.scrollLeft = Math.min(x1 - bodyW, x0)
    scrollLeft = scroller.scrollLeft
  }

  // Rows the parent should have loaded.
  const needStart = $derived(Math.max(0, firstVisible - NEED_OVERSCAN))
  const needEnd = $derived(
    Math.min(rowCount, firstVisible + Math.ceil((vt - firstVisible * ROW_H + bodyH) / ROW_H) + NEED_OVERSCAN),
  )
  $effect(() => {
    const s = needStart
    const e = needEnd
    void needNonce
    if (bodyH > 0 && e > s) untrack(() => onNeedRows(s, e))
  })

  // ---------------------------------------------------------------------------
  // Column widths
  // ---------------------------------------------------------------------------

  let measureCtx: CanvasRenderingContext2D | null = null
  let fontFamily = 'system-ui, sans-serif'
  try {
    fontFamily = getComputedStyle(document.documentElement).fontFamily || fontFamily
  } catch {
    // keep the default
  }

  function textWidth(text: string, bold = false): number {
    measureCtx ??= document.createElement('canvas').getContext('2d')
    if (!measureCtx) return text.length * 7
    measureCtx.font = `${bold ? '600 ' : ''}12px ${fontFamily}`
    return measureCtx.measureText(text).width
  }

  const FAMILY_W: Record<string, number> = {
    boolean: 70,
    integer: 80,
    float: 100,
    decimal: 100,
    date: 100,
    time: 90,
    dateTime: 160,
    uuid: 300,
    objectId: 210,
    text: 160,
    json: 160,
    binary: 130,
  }

  function headerWidth(c: number): number {
    const col = columns[c]
    const hint = columnHint?.(c)
    // name + type label (≈10px font) + optional PK/FK badge + padding and sort arrow
    return textWidth(col.name, true) + textWidth(shortType(col.typeName)) * 0.83 + 6 + (hint ? textWidth(hint) + 14 : 0) + 44
  }

  function clampInitial(w: number): number {
    return Math.min(MAX_INITIAL_W, Math.max(MIN_W, Math.round(w)))
  }

  function sampleWidth(c: number, from: number, to: number): number | null {
    let max = -1
    const family = columns[c].family
    for (let r = from; r < to; r++) {
      const row = getRow(r)
      if (!row) continue
      max = Math.max(max, textWidth(formatValue(row[c], family).text))
    }
    return max < 0 ? null : max + 18
  }

  function initWidths() {
    widths = columns.map((col, c) => clampInitial(Math.max(headerWidth(c), FAMILY_W[col.family] ?? 140)))
  }

  function autosizeAll() {
    const to = Math.min(rowCount, 60)
    widths = columns.map((col, c) => {
      if (userSized.has(c)) return widths[c] ?? DEFAULT_W
      const s = sampleWidth(c, 0, to)
      // Typed columns keep their family width as a floor: later rows are often wider than the first ones.
      const floor = ['text', 'json', 'other', 'binary'].includes(col.family) ? 0 : (FAMILY_W[col.family] ?? 0)
      return clampInitial(Math.max(headerWidth(c), floor, s ?? FAMILY_W[col.family] ?? 140))
    })
  }

  function autosize(c: number) {
    const s = sampleWidth(c, Math.max(0, firstVisible - 100), Math.min(rowCount, firstVisible + 300))
    setWidth(c, Math.max(headerWidth(c), s ?? 0))
  }

  function setWidth(c: number, w: number) {
    const next = effWidths.slice()
    next[c] = Math.min(MAX_RESIZE_W, Math.max(MIN_W, Math.round(w)))
    widths = next
    userSized.add(c)
  }

  function startResize(e: PointerEvent, c: number) {
    e.preventDefault()
    e.stopPropagation()
    const startX = e.clientX
    const startW = effWidths[c]
    const move = (ev: PointerEvent) => setWidth(c, startW + ev.clientX - startX)
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      resizedAt = performance.now()
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  // The parent hides inactive tabs (display: none), which resets the browser's scroll offset:
  // put it back when the grid is shown again.
  let wasHidden = false
  $effect(() => {
    const hidden = clientH === 0
    untrack(() => {
      if (hidden) {
        wasHidden = true
      } else if (wasHidden && scroller) {
        wasHidden = false
        scroller.scrollTop = scrollTop
        scroller.scrollLeft = scrollLeft
      }
    })
  })

  // A new column set (new table / new result) resets widths, selection and scroll.
  const signature = $derived(columns.map((c) => c.name + '\u0001' + c.typeName).join('\u0002'))
  let lastSignature: string | null = null
  $effect.pre(() => {
    const sig = signature
    untrack(() => {
      if (sig === lastSignature) return
      const first = lastSignature === null
      lastSignature = sig
      userSized.clear()
      sampled = false
      initWidths()
      anchor = head = null
      editing = null
      if (!first && scroller) {
        scroller.scrollTop = 0
        scroller.scrollLeft = 0
        scrollTop = 0
        scrollLeft = 0
      }
      needNonce++
    })
  })

  // Once the first rows are in, size columns to their content.
  $effect(() => {
    void revision
    void signature
    untrack(() => {
      if (sampled || rowCount === 0 || !getRow(0)) return
      sampled = true
      autosizeAll()
    })
  })

  // ---------------------------------------------------------------------------
  // Selection
  // ---------------------------------------------------------------------------

  const selection = $derived.by<GridSelection | null>(() => {
    if (!anchor || !head || rowCount === 0 || nCols === 0) return null
    const lastR = rowCount - 1
    const lastC = nCols - 1
    const ar = Math.min(anchor.r, lastR)
    const ac = Math.min(anchor.c, lastC)
    const hr = Math.min(head.r, lastR)
    const hc = Math.min(head.c, lastC)
    return {
      row0: Math.min(ar, hr),
      row1: Math.max(ar, hr),
      col0: Math.min(ac, hc),
      col1: Math.max(ac, hc),
      activeRow: ar,
      activeCol: ac,
    }
  })

  $effect(() => {
    const s = selection
    untrack(() => onSelectionChange?.(s))
  })

  function clamp(v: number, lo: number, hi: number): number {
    return Math.min(hi, Math.max(lo, v))
  }

  function setSelection(a: Cell, h: Cell = a) {
    anchor = a
    head = h
  }

  function move(dr: number, dc: number, extend: boolean) {
    if (rowCount === 0 || nCols === 0) return
    const from = head ?? { r: 0, c: 0 }
    const first = !head
    const r = first ? 0 : clamp(from.r + dr, 0, rowCount - 1)
    const c = first ? 0 : clamp(from.c + dc, 0, nCols - 1)
    const next = { r, c }
    if (extend && anchor) head = next
    else setSelection(next)
    ensureVisible(r, c)
  }

  const selRect = $derived.by(() => {
    const s = selection
    if (!s) return null
    const top = Math.max(0, s.row0 * ROW_H - vt)
    const bottom = Math.min(bodyH, (s.row1 + 1) * ROW_H - vt)
    const left = Math.max(0, colLeft[s.col0] - scrollLeft)
    const right = Math.min(bodyW, colLeft[s.col1 + 1] - scrollLeft)
    if (bottom <= top || right <= left) return null
    return { top, left, width: right - left, height: bottom - top }
  })

  const activeRect = $derived.by(() => {
    const s = selection
    if (!s) return null
    const top = s.activeRow * ROW_H - vt
    const left = colLeft[s.activeCol] - scrollLeft
    const width = effWidths[s.activeCol]
    if (top + ROW_H <= 0 || top >= bodyH || left + width <= 0 || left >= bodyW) return null
    return { top, left, width }
  })

  // ---------------------------------------------------------------------------
  // Pointer interaction
  // ---------------------------------------------------------------------------

  function hitBody(clientX: number, clientY: number): Cell | null {
    if (!bodyEl || rowCount === 0) return null
    const rect = bodyEl.getBoundingClientRect()
    const r = Math.floor((vt + clientY - rect.top) / ROW_H)
    const c = columnAt(colLeft, clientX - rect.left + scrollLeft)
    if (r < 0 || r >= rowCount || c < 0) return null
    return { r, c }
  }

  type DragMode = 'cells' | 'rows'
  let dragMode: DragMode | null = null
  let lastPtr = { x: 0, y: 0 }
  let dragTimer = 0

  function dragTarget(): Cell {
    const rect = bodyEl!.getBoundingClientRect()
    const r = clamp(Math.floor((vt + lastPtr.y - rect.top) / ROW_H), 0, rowCount - 1)
    if (dragMode === 'rows') return { r, c: nCols - 1 }
    const x = lastPtr.x - rect.left + scrollLeft
    const c = x < 0 ? 0 : columnAt(colLeft, x)
    return { r, c: c < 0 ? nCols - 1 : c }
  }

  function onDragMove(e: PointerEvent) {
    lastPtr = { x: e.clientX, y: e.clientY }
    head = dragTarget()
  }

  function dragTick() {
    if (!bodyEl || !scroller) return
    const rect = bodyEl.getBoundingClientRect()
    let dy = 0
    let dx = 0
    if (lastPtr.y < rect.top) dy = -Math.min(60, (rect.top - lastPtr.y) / 2 + 4)
    else if (lastPtr.y > rect.bottom) dy = Math.min(60, (lastPtr.y - rect.bottom) / 2 + 4)
    if (dragMode === 'cells') {
      if (lastPtr.x < rect.left) dx = -Math.min(60, (rect.left - lastPtr.x) / 2 + 4)
      else if (lastPtr.x > rect.right) dx = Math.min(60, (lastPtr.x - rect.right) / 2 + 4)
    }
    if (!dx && !dy) return
    // In scaled mode one scroll pixel is several rows: slow the vertical drag to a usable speed.
    scroller.scrollTop += vm.scaled ? dy * 4 : dy
    scroller.scrollLeft += dx
    scrollTop = scroller.scrollTop
    scrollLeft = scroller.scrollLeft
    head = dragTarget()
  }

  function endDrag() {
    dragMode = null
    window.removeEventListener('pointermove', onDragMove)
    window.removeEventListener('pointerup', endDrag)
    clearInterval(dragTimer)
  }

  function beginDrag(mode: DragMode, e: PointerEvent) {
    dragMode = mode
    lastPtr = { x: e.clientX, y: e.clientY }
    window.addEventListener('pointermove', onDragMove)
    window.addEventListener('pointerup', endDrag)
    dragTimer = window.setInterval(dragTick, 40)
  }

  // An open editor commits through its blur handler.
  function blurEditor() {
    if (editing && document.activeElement instanceof HTMLElement) document.activeElement.blur()
  }

  function onBodyDown(e: PointerEvent) {
    if (e.button !== 0) return
    blurEditor()
    const hit = hitBody(e.clientX, e.clientY)
    scroller?.focus({ preventScroll: true })
    if (!hit) return
    e.preventDefault()
    if (e.shiftKey && anchor) head = hit
    else setSelection(hit)
    beginDrag('cells', e)
  }

  function onGutterDown(e: PointerEvent) {
    if (e.button !== 0 || !bodyEl || rowCount === 0) return
    blurEditor()
    scroller?.focus({ preventScroll: true })
    e.preventDefault()
    const rect = bodyEl.getBoundingClientRect()
    const r = clamp(Math.floor((vt + e.clientY - rect.top) / ROW_H), 0, rowCount - 1)
    const last = nCols - 1
    if (e.shiftKey && anchor) setSelection({ r: anchor.r, c: 0 }, { r, c: last })
    else setSelection({ r, c: 0 }, { r, c: last })
    beginDrag('rows', e)
  }

  function onBodyDblClick(e: MouseEvent) {
    const hit = hitBody(e.clientX, e.clientY)
    if (hit) startEdit(hit.r, hit.c)
  }

  function onBodyContext(e: MouseEvent) {
    e.preventDefault()
    const hit = hitBody(e.clientX, e.clientY)
    if (!hit) return
    const s = selection
    if (!s || hit.r < s.row0 || hit.r > s.row1 || hit.c < s.col0 || hit.c > s.col1) setSelection(hit)
    scroller?.focus({ preventScroll: true })
    onContextMenu?.(e, hit.r, hit.c)
  }

  // `rev` is only here so the template re-reads the row when the parent bumps `revision`.
  function rowAt(r: number, rev: number): Row | undefined {
    void rev
    return getRow(r)
  }

  function sortOf(name: string): Sort | undefined {
    return sort?.find((s) => s.column === name)
  }

  function headerClick(c: number) {
    if (!onSort || performance.now() - resizedAt < 150) return
    const name = columns[c].name
    const cur = sortOf(name)
    if (!cur) onSort(name, false)
    else if (!cur.descending) onSort(name, true)
    else onSort(name, false, true)
  }

  // ---------------------------------------------------------------------------
  // Editing
  // ---------------------------------------------------------------------------

  function startEdit(r: number, c: number, initial?: string) {
    if (r < 0 || r >= rowCount || c < 0 || c >= nCols) return
    if (onEditCell?.(r, c, initial) === true) return
    if (!editable || !editor) return
    if (canEditCell && !canEditCell(r, c)) return
    if (!getRow(r)) return
    ensureVisible(r, c)
    editing = { id: ++editSeq, row: r, col: c, initial }
  }

  function commitEdit(input: CellInput, mv: EditMove = 'none') {
    const e = editing
    if (!e) return
    editing = null
    onCommitCell?.(e.row, e.col, input)
    scroller?.focus({ preventScroll: true })
    if (mv === 'down') move(1, 0, false)
    else if (mv === 'right') move(0, 1, false)
    else if (mv === 'left') move(0, -1, false)
  }

  function cancelEdit() {
    if (!editing) return
    editing = null
    scroller?.focus({ preventScroll: true })
  }

  const editRect = $derived.by(() => {
    const e = editing
    if (!e) return null
    return {
      top: e.row * ROW_H - vt,
      left: colLeft[e.col] - scrollLeft,
      width: Math.max(effWidths[e.col] ?? DEFAULT_W, 110),
    }
  })

  // ---------------------------------------------------------------------------
  // Keyboard and clipboard
  // ---------------------------------------------------------------------------

  function onKeydown(e: KeyboardEvent) {
    if (editing || e.target !== scroller) return
    if (onKeyDown?.(e)) return
    const mod = e.metaKey || e.ctrlKey
    const pageRows = Math.max(1, Math.floor(bodyH / ROW_H) - 1)
    const big = rowCount + nCols
    let handled = true
    switch (e.key) {
      case 'ArrowDown':
        move(mod ? big : 1, 0, e.shiftKey)
        break
      case 'ArrowUp':
        move(mod ? -big : -1, 0, e.shiftKey)
        break
      case 'ArrowRight':
        move(0, mod ? big : 1, e.shiftKey)
        break
      case 'ArrowLeft':
        move(0, mod ? -big : -1, e.shiftKey)
        break
      case 'PageDown':
        move(pageRows, 0, e.shiftKey)
        break
      case 'PageUp':
        move(-pageRows, 0, e.shiftKey)
        break
      case 'Home':
        move(mod ? -big : 0, -big, e.shiftKey)
        break
      case 'End':
        move(mod ? big : 0, big, e.shiftKey)
        break
      case 'Tab': {
        const f = head
        if (!f) handled = false
        else if (!e.shiftKey) {
          if (f.c < nCols - 1) move(0, 1, false)
          else if (f.r < rowCount - 1) {
            setSelection({ r: f.r + 1, c: 0 })
            ensureVisible(f.r + 1, 0)
          }
        } else if (f.c > 0) move(0, -1, false)
        else if (f.r > 0) {
          setSelection({ r: f.r - 1, c: nCols - 1 })
          ensureVisible(f.r - 1, nCols - 1)
        }
        break
      }
      case 'Enter':
      case 'F2':
        if (anchor) startEdit(anchor.r, anchor.c)
        break
      default:
        if (mod && (e.key === 'a' || e.key === 'A')) {
          if (rowCount > 0 && nCols > 0) setSelection({ r: 0, c: 0 }, { r: rowCount - 1, c: nCols - 1 })
        } else if (mod && (e.key === 'c' || e.key === 'C')) {
          void copySelection(e.shiftKey)
        } else if (!mod && !e.altKey && e.key.length === 1 && anchor && editable) {
          startEdit(anchor.r, anchor.c, e.key)
        } else {
          handled = false
        }
    }
    if (handled) e.preventDefault()
  }

  function tsvEscape(text: string): string {
    return /[\t\n\r"]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text
  }

  export async function copySelection(withHeader = false) {
    const s = selection
    if (!s) return
    const lines: string[] = []
    if (withHeader) {
      const names: string[] = []
      for (let c = s.col0; c <= s.col1; c++) names.push(tsvEscape(columns[c].name))
      lines.push(names.join('\t'))
    }
    const width = s.col1 - s.col0 + 1
    const maxRows = Math.max(1, Math.floor(MAX_COPY_CELLS / width))
    const lastRow = Math.min(s.row1, s.row0 + maxRows - 1)
    for (let r = s.row0; r <= lastRow; r++) {
      const row = getRow(r)
      const parts: string[] = []
      for (let c = s.col0; c <= s.col1; c++) parts.push(tsvEscape(row ? valueToEditText(row[c]) : ''))
      lines.push(parts.join('\t'))
    }
    const text = lines.join('\n')
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      ta.remove()
      scroller?.focus({ preventScroll: true })
    }
  }

  // ---------------------------------------------------------------------------
  // Exported API
  // ---------------------------------------------------------------------------

  export function focus() {
    scroller?.focus({ preventScroll: true })
  }

  /** Starts editing a cell the same way a double click does. */
  export function edit(row: number, col: number) {
    startEdit(row, col)
  }

  export function getSelection(): GridSelection | null {
    return selection
  }

  export function selectCell(row: number, col = 0) {
    if (rowCount === 0 || nCols === 0) return
    const cell = { r: clamp(row, 0, rowCount - 1), c: clamp(col, 0, nCols - 1) }
    setSelection(cell)
    ensureVisible(cell.r, cell.c)
  }

  export function selectRows(from: number, to: number) {
    if (rowCount === 0 || nCols === 0) return
    setSelection({ r: clamp(from, 0, rowCount - 1), c: 0 }, { r: clamp(to, 0, rowCount - 1), c: nCols - 1 })
  }

  export function clearSelection() {
    anchor = head = null
  }

  export function scrollToRow(row: number) {
    if (rowCount === 0) return
    setVirtualTop(clamp(row, 0, rowCount - 1) * ROW_H)
  }
</script>

<div
  class="grid"
  bind:this={scroller}
  bind:clientWidth={clientW}
  bind:clientHeight={clientH}
  tabindex="0"
  role="grid"
  style:--row-h="{ROW_H}px"
  style:--head-h="{HEADER_H}px"
  aria-rowcount={rowCount}
  aria-colcount={nCols}
  onscroll={onScroll}
  onkeydown={onKeydown}
>
  <div class="spacer" style:width="{spacerW}px" style:height="{vm.spacerH}px">
    <div class="window" style:width="{clientW}px" style:height="{clientH}px">
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div
        class="corner"
        style:width="{gutterW}px"
        onclick={() => rowCount > 0 && setSelection({ r: 0, c: 0 }, { r: rowCount - 1, c: nCols - 1 })}
      >
        #
      </div>

      <div class="header" style:left="{gutterW}px">
        <div class="hrow" style:transform="translateX({-scrollLeft}px)">
          {#each colIdx as c (c)}
            {@const col = columns[c]}
            {@const srt = sortOf(col.name)}
            {@const hint = columnHint?.(c)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div
              class="h"
              class:sortable={!!onSort}
              style:left="{colLeft[c]}px"
              style:width="{effWidths[c]}px"
              title="{col.name}  ·  {col.typeName}"
              onclick={() => headerClick(c)}
            >
              <span class="hname">{col.name}</span>
              <span class="htype">{shortType(col.typeName)}</span>
              {#if hint}<span class="badge">{hint}</span>{/if}
              {#if srt}<span class="sort">{srt.descending ? '▼' : '▲'}</span>{/if}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="resize"
                onpointerdown={(e) => startResize(e, c)}
                onclick={(e) => e.stopPropagation()}
                ondblclick={(e) => {
                  e.stopPropagation()
                  autosize(c)
                }}
              ></div>
            </div>
          {/each}
        </div>
      </div>

      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="gutter" style:width="{gutterW}px" onpointerdown={onGutterDown}>
        <div class="gcol" style:transform="translateY({-rowsShift}px)">
          {#each rowIdx as r (r)}
            <div class="g" class:on={selection && r >= selection.row0 && r <= selection.row1}>
              {rowLabel ? rowLabel(r) : r + 1}
            </div>
          {/each}
        </div>
      </div>

      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="body"
        bind:this={bodyEl}
        style:left="{gutterW}px"
        onpointerdown={onBodyDown}
        ondblclick={onBodyDblClick}
        oncontextmenu={onBodyContext}
      >
        <div class="rows" style:width="{totalW}px" style:transform="translate({-scrollLeft}px, {-rowsShift}px)">
          {#each rowIdx as r (r)}
            {@const row = rowAt(r, revision)}
            <div class="row {rowClass?.(r) ?? ''}" class:alt={r % 2 === 1} class:cur={selection?.activeRow === r}>
              {#each colIdx as c (c)}
                {#if row}
                  {@const f = formatValue(row[c], columns[c].family)}
                  <div
                    class="cell {f.cls} {cellClass?.(r, c) ?? ''}"
                    style:left="{colLeft[c]}px"
                    style:width="{effWidths[c]}px"
                  >
                    {#if columns[c].family === 'enum' && typeof row[c] === 'string'}<span class="pill {statusTone(f.text)}">{f.text}</span>{:else}{f.text}{/if}
                  </div>
                {:else}
                  <div class="cell ph" style:left="{colLeft[c]}px" style:width="{effWidths[c]}px"></div>
                {/if}
              {/each}
            </div>
          {/each}
        </div>

        {#if selRect}
          <div
            class="sel"
            style:left="{selRect.left}px"
            style:top="{selRect.top}px"
            style:width="{selRect.width}px"
            style:height="{selRect.height}px"
          ></div>
        {/if}
        {#if activeRect}
          <div
            class="active"
            style:left="{activeRect.left}px"
            style:top="{activeRect.top}px"
            style:width="{activeRect.width}px"
            style:height="{ROW_H}px"
          ></div>
        {/if}

        {#if editing && editor && editRect}
          {@const ed = editing}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="edit-host"
            style:left="{editRect.left}px"
            style:top="{editRect.top}px"
            style:width="{editRect.width}px"
            style:height="{ROW_H}px"
            onpointerdown={(e) => e.stopPropagation()}
            ondblclick={(e) => e.stopPropagation()}
          >
            {#key ed.id}
              {@render editor({
                row: ed.row,
                col: ed.col,
                column: columns[ed.col],
                value: getRow(ed.row)?.[ed.col],
                initial: ed.initial,
                width: editRect.width,
                height: ROW_H,
                commit: commitEdit,
                cancel: cancelEdit,
              })}
            {/key}
          </div>
        {/if}

        {#if rowCount === 0 && nCols > 0}
          <div class="empty">{emptyMessage}</div>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .grid {
    --g-line: var(--border);
    --g-alt: transparent;
    --g-sel: var(--selection);
    --g-head: var(--bg-head);
    --g-inserted: rgb(46 160 67 / 0.2);
    --g-deleted: rgb(229 72 77 / 0.18);
    --g-edited: rgb(212 160 23 / 0.3);
    --hair: 1px;
    position: relative;
    width: 100%;
    height: 100%;
    overflow: auto;
    outline: none;
    background: var(--bg);
    color: var(--fg-cell);
    font-size: 11.5px;
    contain: layout paint;
  }

  @media (min-resolution: 2dppx) {
    .grid {
      --hair: 0.5px;
    }
  }

  .spacer {
    position: relative;
  }

  .window {
    position: sticky;
    top: 0;
    left: 0;
    overflow: hidden;
  }

  .corner,
  .header,
  .gutter,
  .body {
    position: absolute;
  }

  .corner {
    top: 0;
    left: 0;
    height: var(--head-h);
    z-index: 4;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--g-head);
    color: var(--fg-muted);
    border-right: var(--hair) solid var(--g-line);
    border-bottom: var(--hair) solid var(--g-line);
    cursor: pointer;
  }

  .header {
    top: 0;
    right: 0;
    height: var(--head-h);
    overflow: hidden;
    z-index: 3;
    background: var(--g-head);
    border-bottom: var(--hair) solid var(--g-line);
  }

  .hrow {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    will-change: transform;
  }

  .h {
    position: absolute;
    top: 0;
    height: 100%;
    padding: 0 14px;
    display: flex;
    align-items: center;
    gap: 6px;
    border-right: var(--hair) solid var(--g-line);
    overflow: hidden;
    white-space: nowrap;
  }

  .h.sortable {
    cursor: pointer;
  }

  .h.sortable:hover {
    background: var(--bg-hover);
  }

  .hname {
    font-weight: 600;
    font-size: 11px;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 1 auto;
    min-width: 0;
  }

  .htype {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 9.5px;
    font-weight: 500;
    color: var(--fg-faint);
  }

  .h:hover .hname {
    color: var(--accent);
  }

  .badge {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.03em;
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    border-radius: 3px;
    padding: 0 4px;
    line-height: 14px;
  }

  .sort {
    margin-left: auto;
    font-size: 8px;
    color: var(--accent);
  }

  .resize {
    position: absolute;
    top: 0;
    right: -3px;
    width: 7px;
    height: 100%;
    cursor: col-resize;
    z-index: 2;
  }

  .gutter {
    top: var(--head-h);
    left: 0;
    bottom: 0;
    z-index: 2;
    overflow: hidden;
    background: var(--bg);
    border-right: var(--hair) solid var(--g-line);
  }

  .gcol {
    will-change: transform;
  }

  .g {
    height: var(--row-h);
    line-height: var(--row-h);
    padding: 0 10px;
    text-align: right;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
    border-bottom: var(--hair) solid var(--g-line);
    overflow: hidden;
    cursor: pointer;
  }

  .g.on {
    color: var(--fg);
    background: var(--row-selected);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .body {
    top: var(--head-h);
    right: 0;
    bottom: 0;
    overflow: hidden;
    z-index: 1;
    cursor: cell;
  }

  .rows {
    position: absolute;
    top: 0;
    left: 0;
    will-change: transform;
  }

  .row {
    position: relative;
    height: var(--row-h);
  }

  .row.alt {
    background: var(--g-alt);
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.cur,
  .row.cur:hover {
    background: var(--row-selected);
  }

  .row.inserted {
    background: var(--g-inserted);
  }

  .row.deleted {
    background: var(--g-deleted);
  }

  .row.deleted .cell {
    text-decoration: line-through;
    color: var(--fg-muted);
  }

  .cell {
    position: absolute;
    top: 0;
    height: var(--row-h);
    line-height: calc(var(--row-h) - 1px);
    padding: 0 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    border-right: var(--hair) solid var(--g-line);
    border-bottom: var(--hair) solid var(--g-line);
  }

  .cell.num {
    text-align: right;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    color: var(--fg);
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border-radius: 5px;
    font-size: 10px;
    font-weight: 640;
    line-height: 20px;
    vertical-align: middle;
    color: var(--fg-muted);
    background: color-mix(in srgb, var(--fg-muted) 16%, transparent);
  }

  .pill::before {
    content: '';
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }

  .pill.good {
    color: var(--success);
    background: color-mix(in srgb, var(--success) 16%, transparent);
  }

  .pill.warn {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 18%, transparent);
  }

  .pill.bad {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }

  .pill.info {
    color: var(--info);
    background: color-mix(in srgb, var(--info) 16%, transparent);
  }

  .cell.null,
  .cell.missing {
    color: var(--fg-faint);
  }

  .cell.null {
    font-size: 10.5px;
    letter-spacing: 0.04em;
  }

  .cell.mono {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--mono-fg);
  }

  .cell.bool.yes {
    color: var(--success);
  }

  .cell.bool.no {
    color: var(--fg-muted);
  }

  .cell.edited {
    background: var(--g-edited);
  }

  .cell.ph::after {
    content: '';
    display: block;
    height: 8px;
    margin-top: 8px;
    width: 55%;
    border-radius: 4px;
    background: rgb(128 128 128 / 0.16);
  }

  .sel {
    position: absolute;
    pointer-events: none;
    background: var(--g-sel);
    z-index: 2;
  }

  .active {
    position: absolute;
    pointer-events: none;
    border: 1.5px solid var(--accent);
    border-radius: 2px;
    z-index: 3;
  }

  .edit-host {
    position: absolute;
    z-index: 5;
    background: var(--bg);
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.25);
    cursor: auto;
  }

  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--fg-muted);
    pointer-events: none;
  }
</style>
