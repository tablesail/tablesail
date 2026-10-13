// CodeMirror 6 setup. Imported dynamically from SqlEditor so the editor code stays out of the
// cold-launch bundle.

import { indentWithTab } from '@codemirror/commands'
import { Compartment, EditorState, Prec, type Extension } from '@codemirror/state'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { MariaSQL, MySQL, PostgreSQL, SQLite, StandardSQL, sql, type SQLDialect } from '@codemirror/lang-sql'
import { EditorView, keymap, placeholder } from '@codemirror/view'
import { basicSetup } from 'codemirror'
import { tags as t } from '@lezer/highlight'
import type { Engine } from '../types'

export interface SchemaConfig {
  engine: Engine
  /** table name (schema-qualified where applicable) -> columns */
  columns: Record<string, string[]>
  defaultSchema?: string
}

export interface EditorOptions {
  parent: HTMLElement
  doc: string
  schema: SchemaConfig
  onChange: (text: string) => void
  onRun: () => void
  onRunAll: () => void
}

function dialectFor(engine: Engine): SQLDialect {
  switch (engine) {
    case 'postgres':
      return PostgreSQL
    case 'mysql':
      return MySQL
    case 'mariadb':
      return MariaSQL
    case 'sqlite':
      return SQLite
    default:
      return StandardSQL
  }
}

function languageFor(cfg: SchemaConfig): Extension {
  // MongoDB queries are not SQL: plain text, no highlighting or SQL completions.
  if (cfg.engine === 'mongodb') return []
  return sql({
    dialect: dialectFor(cfg.engine),
    schema: cfg.columns,
    defaultSchema: cfg.defaultSchema,
    upperCaseKeywords: true,
  })
}

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier], color: 'var(--syn-keyword)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--syn-string)' },
  { tag: [t.number, t.bool, t.null], color: 'var(--syn-number)' },
  { tag: [t.lineComment, t.blockComment], color: 'var(--syn-comment)', fontStyle: 'italic' },
  { tag: [t.typeName, t.standard(t.name)], color: 'var(--syn-type)' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: 'var(--syn-fn)' },
  { tag: [t.operator, t.punctuation], color: 'var(--syn-op)' },
  { tag: [t.name, t.variableName, t.propertyName], color: 'var(--syn-name)' },
])

const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--bg)', color: 'var(--fg)', fontSize: '13px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--mono)', lineHeight: '1.5' },
  '.cm-content': { caretColor: 'var(--fg)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--fg)' },
  '.cm-gutters': {
    backgroundColor: 'var(--bg-sidebar)',
    color: 'var(--fg-muted)',
    border: 'none',
    borderRight: '1px solid var(--border)',
  },
  '.cm-activeLine': { backgroundColor: 'var(--bg-hover)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--bg-hover)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'var(--selection) !important',
  },
  '.cm-selectionMatch': { backgroundColor: 'var(--selection)' },
  '.cm-tooltip': {
    backgroundColor: 'var(--bg-elev)',
    color: 'var(--fg)',
    border: '0.5px solid var(--border-strong)',
    borderRadius: '6px',
    boxShadow: 'var(--shadow-sm)',
    overflow: 'hidden',
  },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: 'var(--row-selected)',
    color: 'var(--fg)',
  },
  '.cm-panels': { backgroundColor: 'var(--bg-sidebar)', color: 'var(--fg)' },
  '.cm-searchMatch': { backgroundColor: 'var(--selection)' },
})

export interface EditorHandle {
  getText(): string
  /** UTF-16 range of the non-empty selection, or null */
  getSelection(): { from: number; to: number; text: string } | null
  getCursor(): number
  setText(text: string): void
  setSchema(cfg: SchemaConfig): void
  focus(): void
  destroy(): void
}

export function createEditor(opts: EditorOptions): EditorHandle {
  const lang = new Compartment()

  const runKeys = Prec.highest(
    keymap.of([
      {
        key: 'Mod-Enter',
        preventDefault: true,
        run: () => {
          opts.onRun()
          return true
        },
      },
      {
        key: 'Shift-Mod-Enter',
        preventDefault: true,
        run: () => {
          opts.onRunAll()
          return true
        },
      },
    ]),
  )

  const view = new EditorView({
    parent: opts.parent,
    state: EditorState.create({
      doc: opts.doc,
      extensions: [
        runKeys,
        basicSetup,
        syntaxHighlighting(highlight),
        keymap.of([indentWithTab]),
        placeholder(opts.schema.engine === 'mongodb' ? '' : 'Write SQL here. Cmd/Ctrl+Enter runs the current statement.'),
        lang.of(languageFor(opts.schema)),
        theme,
        EditorView.contentAttributes.of({ 'aria-label': 'SQL editor' }),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) opts.onChange(u.state.doc.toString())
        }),
      ],
    }),
  })

  return {
    getText: () => view.state.doc.toString(),
    getSelection() {
      const r = view.state.selection.main
      if (r.empty) return null
      return { from: r.from, to: r.to, text: view.state.sliceDoc(r.from, r.to) }
    },
    getCursor: () => view.state.selection.main.head,
    setText(text) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } })
    },
    setSchema(cfg) {
      view.dispatch({ effects: lang.reconfigure(languageFor(cfg)) })
    },
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  }
}
