<!--
  Right-hand panel, TablePlus-style: "Details" lists every field of the
  selected row as label + value box (with a field search and foreign-key jumps);
  "Assistant" is the AI SQL chat.
-->
<script lang="ts">
  import AssistantPanel from './AssistantPanel.svelte'
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte'
  import Icon from '../ui/Icon.svelte'
  import { formatValue, valueToEditText } from '../format'
  import { inspector } from '../stores/inspector.svelte'
  import { pulse } from '../stores/pulse.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import type { Filter, SessionInfo, Value } from '../types'
  import { jsonOf, jsonTokens } from '../util/jsonView'

  interface Props {
    session: SessionInfo
    /** Only rows published by the visible tab are shown. */
    activeTabId: string | null
  }

  let { session, activeTabId }: Props = $props()

  let query = $state('')
  /** JSON fields shown on one line instead of pretty-printed, by column name. */
  let minified = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)

  const JSON_LIMIT = 50_000

  let cur = $derived(
    inspector.current && inspector.current.session.sessionId === session.sessionId && inspector.current.tabId === activeTabId
      ? inspector.current
      : null,
  )

  let fields = $derived.by(() => {
    if (!cur) return []
    const q = query.trim().toLowerCase()
    return cur.columns
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => !q || c.name.toLowerCase().includes(q))
  })

  function valueText(v: Value | undefined, i: number): { text: string; cls: string } {
    if (!cur) return { text: '', cls: '' }
    const f = formatValue(v, cur.columns[i].family)
    return { text: f.text.length > 2000 ? f.text.slice(0, 2000) + '…' : f.text, cls: f.cls }
  }

  /** Pretty (or minified) JSON text of a JSON cell, or null for other values. */
  function jsonText(v: Value | undefined, i: number): string | null {
    if (!cur) return null
    const j = jsonOf(v, cur.columns[i].family)
    if (j === undefined) return null
    const text = JSON.stringify(j, null, minified[cur.columns[i].name] ? 0 : 2) ?? 'null'
    return text.length > JSON_LIMIT ? text.slice(0, JSON_LIMIT) + '\n…' : text
  }

  /** Inline editing is for plain values; JSON, bytes and truncated text keep their viewers. */
  function canEditField(i: number): boolean {
    if (!cur?.edit?.canEdit(i)) return false
    const v = cur.row[i]
    if (typeof v === 'object' && v !== null && ('$bytes' in v || '$text' in v)) return false
    return cur.columns[i].family !== 'binary'
  }

  function commitText(i: number, before: string, el: HTMLInputElement | HTMLTextAreaElement) {
    if (!cur?.edit || el.value === before) return
    cur.edit.commit(i, { kind: 'text', value: el.value })
  }

  let jsonError = $state<Record<string, string>>({})

  function commitJson(i: number, before: string, el: HTMLTextAreaElement) {
    if (!cur?.edit || el.value === before) return
    const name = cur.columns[i].name
    try {
      const value = JSON.parse(el.value)
      delete jsonError[name]
      cur.edit.commit(i, { kind: 'json', value })
    } catch (err) {
      jsonError[name] = err instanceof Error ? err.message : 'Invalid JSON'
    }
  }

  function editKeys(e: KeyboardEvent, before: string) {
    const el = e.currentTarget as HTMLInputElement | HTMLTextAreaElement
    if (e.key === 'Escape') {
      el.value = before
      el.blur()
    } else if (e.key === 'Enter' && el instanceof HTMLInputElement) {
      el.blur()
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      el.blur()
    }
  }

  function fieldMenu(e: MouseEvent, i: number, text: string, isJson: boolean) {
    if (!cur) return
    const name = cur.columns[i].name
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const items: MenuItem[] = []
    const edit = canEditField(i) || (isJson && !!cur.edit?.canEdit(i)) ? cur.edit : undefined
    if (edit) {
      items.push(
        { label: 'Set NULL', onclick: () => edit.commit(i, { kind: 'null' }) },
        { label: 'Set DEFAULT', onclick: () => edit.commit(i, { kind: 'default' }) },
        { label: 'Set EMPTY', disabled: isJson, onclick: () => edit.commit(i, { kind: 'text', value: '' }) },
        { separator: true },
      )
    }
    if (isJson) {
      items.push(
        { label: minified[name] ? 'Pretty' : '✓  Pretty', onclick: () => (minified[name] = false) },
        { label: minified[name] ? '✓  Minify' : 'Minify', onclick: () => (minified[name] = true) },
        { separator: true },
      )
    }
    items.push({ label: 'Copy value', onclick: () => void navigator.clipboard?.writeText(text) })
    menu = { x: r.right - 170, y: r.bottom + 4, items }
  }

  /** Raw text of a value, for building an equality filter. */
  function filterText(v: Value | undefined): string | null {
    if (v === null || v === undefined) return null
    if (typeof v !== 'object') return String(v)
    const o = v as Record<string, unknown>
    for (const k of ['$int', '$dec', '$uuid', '$oid', '$date', '$datetime', '$time', '$other', '$text']) {
      if (typeof o[k] === 'string') return o[k] as string
    }
    return null
  }

  function openRelated(fkIndex: number) {
    if (!cur) return
    const fk = cur.foreignKeys[fkIndex]
    const filters: Filter[] = []
    fk.columns.forEach((col, i) => {
      const ci = cur.columns.findIndex((c) => c.name === col)
      const text = ci >= 0 ? filterText(cur.row[ci]) : null
      if (text !== null) filters.push({ column: fk.referencedColumns[i], op: 'eq', value: text })
    })
    tabs.openTable(cur.session.sessionId, fk.referenced, filters)
  }

  let samples = $derived(pulse.for(session.sessionId))
  let last = $derived(samples.length ? samples[samples.length - 1] : null)
</script>

<aside class="inspector">
  <div class="ihead">
    <div class="views" role="tablist">
      <button role="tab" aria-selected={inspector.tab === 'details'} class:on={inspector.tab === 'details'} onclick={() => (inspector.tab = 'details')}>Details</button>
      <button role="tab" aria-selected={inspector.tab === 'assistant'} class:on={inspector.tab === 'assistant'} onclick={() => (inspector.tab = 'assistant')}>Assistant</button>
    </div>
  </div>

  {#if inspector.tab === 'assistant'}
    <div class="assistant">
      <AssistantPanel {session} />
    </div>
  {:else}
    <div class="top">
      <label class="search">
        <Icon name="search" size={13} />
        <input type="search" placeholder="Search for field…" bind:value={query} spellcheck="false" />
      </label>
    </div>

    <div class="scroll">
      {#if cur}
        {#each fields as { c, i } (c.name + i)}
          {@const v = valueText(cur.row[i], i)}
          {@const json = jsonText(cur.row[i], i)}
          {@const editableField = canEditField(i)}
          {@const before = valueToEditText(cur.row[i])}
          <div class="field">
            <div class="label"><span class="lname">{c.name}</span><span class="ltype">{c.typeName}</span></div>
            <div class="box">
              {#if json !== null && cur.edit?.canEdit(i)}
                {#key json}
                  <textarea
                    class="value json-edit"
                    spellcheck="false"
                    rows={Math.min(16, json.split('\n').length)}
                    value={json}
                    onkeydown={(e) => editKeys(e, json)}
                    onchange={(e) => commitJson(i, json, e.currentTarget)}
                  ></textarea>
                {/key}
              {:else if json !== null}
                <pre class="value json" class:min={minified[c.name]}>{#each jsonTokens(json) as t, ti (ti)}<span class={t.cls}>{t.text}</span>{/each}</pre>
              {:else if editableField}
                {#key before}
                  {#if before.length > 60 || before.includes('\n')}
                    <textarea
                      class="value edit"
                      spellcheck="false"
                      rows={Math.min(8, Math.ceil(before.length / 34) + before.split('\n').length)}
                      value={before}
                      onkeydown={(e) => editKeys(e, before)}
                      onchange={(e) => commitText(i, before, e.currentTarget)}
                    ></textarea>
                  {:else}
                    <input
                      class="value edit {v.cls}"
                      type="text"
                      spellcheck="false"
                      value={before}
                      placeholder={cur.row[i] === null ? 'NULL' : ''}
                      onkeydown={(e) => editKeys(e, before)}
                      onchange={(e) => commitText(i, before, e.currentTarget)}
                    />
                  {/if}
                {/key}
              {:else}
                <div class="value {v.cls}" class:multi={v.text.length > 40 || v.text.includes('\n')}>{v.text || ' '}</div>
              {/if}
              <button class="more" aria-label="Field options" title="Options" onclick={(e) => fieldMenu(e, i, json ?? v.text, json !== null)}>
                <Icon name="chevron-down" size={11} strokeWidth={1.8} />
              </button>
            </div>
            {#if jsonError[c.name]}<div class="jerr">{jsonError[c.name]}</div>{/if}
          </div>
        {:else}
          <div class="empty">No matching fields.</div>
        {/each}

        {#if cur.foreignKeys.length > 0 && !query}
          <div class="rel-title">Relations</div>
          {#each cur.foreignKeys as fk, i (fk.name)}
            <button class="related" onclick={() => openRelated(i)} title="Open the related row">
              <span>{fk.columns.join(', ')}</span>
              <span class="to">→ {fk.referenced.name}</span>
            </button>
          {/each}
        {/if}
      {:else}
        <div class="empty">No row selected</div>
      {/if}
    </div>

    <div class="foot">
      {#if cur?.table}<span class="src">{cur.table.schema ? `${cur.table.schema}.` : ''}{cur.table.name}</span>{/if}
      <span class="grow"></span>
      {#if last !== null}<span class="lat" title="Last round trip"><span class="dot" class:slow={last >= 300}></span>{Math.round(last)} ms</span>{/if}
    </div>
  {/if}
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .inspector {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-sidebar);
    border-left: 0.5px solid var(--border-strong);
  }

  .ihead {
    display: flex;
    justify-content: center;
    padding: 8px 8px 4px;
  }

  .views {
    display: flex;
    gap: 4px;
  }

  .views button {
    min-height: 24px;
    padding: 0 12px;
    border: none;
    border-radius: 6px;
    background: none;
    box-shadow: none;
    color: var(--fg-muted);
  }

  .views button:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .views button.on,
  .views button.on:hover:not(:disabled) {
    background: var(--bg-active);
    color: var(--fg);
  }

  .assistant {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .assistant > :global(*) {
    flex: 1;
    min-height: 0;
  }

  .top {
    padding: 4px 8px 6px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 0.5px solid var(--border-input);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--fg-faint);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .search input {
    flex: 1;
    width: 100%;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 13px;
  }

  .search input:focus {
    box-shadow: none;
  }

  .scroll {
    flex: 1;
    overflow: auto;
    padding: 2px 8px 12px;
  }

  .field {
    margin-bottom: 8px;
  }

  .label {
    display: flex;
    gap: 8px;
    margin: 0 1px 3px;
    font-size: 12px;
    color: var(--fg-muted);
    white-space: nowrap;
  }

  .lname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ltype {
    flex: none;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--fg-faint);
  }

  .value {
    min-height: 26px;
    padding: 4px 8px;
    border: 0.5px solid var(--border-input);
    border-radius: 5px;
    background: var(--bg-input);
    font-family: var(--mono);
    font-size: 13px;
    line-height: 17px;
    color: var(--fg-cell);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: text;
    -webkit-user-select: text;
  }

  .value.multi {
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 160px;
    overflow: auto;
  }

  .box {
    position: relative;
  }

  .box .value {
    padding-right: 24px;
  }

  .more {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 18px;
    min-width: 18px;
    height: 18px;
    min-height: 18px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: none;
    box-shadow: none;
    color: var(--fg-faint);
  }

  .more:hover:not(:disabled) {
    background: var(--bg-active);
    color: var(--fg);
  }

  /* Editable fields look like the read-only boxes until focused. */
  .value.edit,
  .value.json-edit {
    display: block;
    width: 100%;
    margin: 0;
    resize: vertical;
    min-height: 26px;
    font: inherit;
    font-family: var(--mono);
    font-size: 13px;
    line-height: 17px;
    border-radius: 5px;
  }

  .value.json-edit {
    font-size: 12px;
    line-height: 18px;
    white-space: pre;
    max-height: 420px;
  }

  .jerr {
    margin: 3px 1px 0;
    font-size: 11px;
    color: var(--danger);
  }

  .value.json {
    margin: 0;
    max-height: 420px;
    overflow: auto;
    white-space: pre;
    font-size: 12px;
    line-height: 18px;
  }

  .value.json.min {
    white-space: pre-wrap;
    word-break: break-all;
  }

  .json .key {
    color: var(--syn-keyword);
  }

  .json .str {
    color: var(--syn-string);
  }

  .json .num {
    color: var(--syn-number);
  }

  .json .lit {
    color: var(--syn-type);
  }

  .json .punct {
    color: var(--fg-muted);
  }

  .value.null,
  .value.missing {
    color: var(--fg-faint);
  }

  .rel-title {
    margin: 14px 1px 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .related {
    display: flex;
    width: 100%;
    justify-content: flex-start;
    gap: 6px;
    margin-bottom: 4px;
    font-family: var(--mono);
    font-size: 12px;
  }

  .related .to {
    color: var(--info);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .empty {
    padding: 40px 12px;
    text-align: center;
    color: var(--fg-faint);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding: 0 12px;
    border-top: 0.5px solid var(--border-strong);
    font-size: 11px;
    color: var(--fg-muted);
    flex: none;
  }

  .grow {
    flex: 1;
  }

  .src {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lat {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-variant-numeric: tabular-nums;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success);
  }

  .dot.slow {
    background: var(--warning);
  }
</style>
