import type { Engine } from '../types'
import { ROW_FORMAT_LABEL, availableFormats, type RowFormat } from '../util/rowFormats'

export interface RowMenuItem {
  label?: string
  hint?: string
  danger?: boolean
  disabled?: boolean
  separator?: boolean
  onclick?: () => void
  /** Opens a flyout with these items (the item itself does nothing when clicked). */
  submenu?: RowMenuItem[]
}

/**
 * The copy entries shared by the table view and query results: Copy, Copy with headers and a
 * "Copy as" flyout (INSERT statements, JSON, Markdown table, tab-separated; MongoDB: JSON and TSV).
 * Everything is disabled while no selected row is loaded.
 */
export function copyMenuItems(opts: {
  engine: Engine
  disabled: boolean
  copy: (withHeader: boolean) => void
  copyAs: (format: RowFormat) => void
}): RowMenuItem[] {
  return [
    { label: 'Copy', hint: '⌘C', disabled: opts.disabled, onclick: () => opts.copy(false) },
    { label: 'Copy with headers', hint: '⌘⇧C', disabled: opts.disabled, onclick: () => opts.copy(true) },
    {
      label: 'Copy as',
      disabled: opts.disabled,
      submenu: availableFormats(opts.engine).map((format) => ({
        label: ROW_FORMAT_LABEL[format],
        onclick: () => opts.copyAs(format),
      })),
    },
  ]
}
