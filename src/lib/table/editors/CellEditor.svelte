<!--
  Inline editor shown inside a grid cell (DataGrid `editor` snippet).
  Chooses the control from the column type: boolean select, enum select, date / time /
  datetime pickers (with a raw-text fallback, zero dates stay editable) or a text input.
  Enter commits and moves down, Tab commits and moves right, Escape cancels, blur commits.
  Cmd/Ctrl+Shift+N and Cmd/Ctrl+Backspace set NULL.
-->
<script lang="ts">
  import { untrack } from 'svelte'
  import type { CellInput, ColumnInfo, Engine } from '../../types'
  import type { EditMove, EditorContext } from '../../grid/types'
  import { isNumericFamily, valueToEditText } from '../../format'

  interface Props {
    ctx: EditorContext
    /** Structure info of the edited column (undefined for result sets without one). */
    info: ColumnInfo | undefined
    engine: Engine
  }

  let { ctx, info, engine }: Props = $props()

  type Kind = 'text' | 'bool' | 'enum' | 'date' | 'datetime' | 'time'

  // The editor is mounted per edit: everything below deliberately captures the initial props.
  const init = untrack(() => ({
    info,
    family: info?.family ?? ctx.column.family,
    nullable: info ? info.nullable : true,
    value: ctx.value,
    initial: ctx.initial,
    startText: valueToEditText(ctx.value),
    startIsNull: ctx.value === null,
  }))
  const { family, nullable, startText, startIsNull } = init
  const enumValues = init.info?.enumValues ?? null

  const kind: Kind = (() => {
    if (enumValues && enumValues.length > 0) return 'enum'
    switch (family) {
      case 'boolean':
        return 'bool'
      case 'enum':
        return enumValues ? 'enum' : 'text'
      case 'date':
        return 'date'
      case 'dateTime':
        return 'datetime'
      case 'time':
        return 'time'
      default:
        return 'text'
    }
  })()

  const DATE_RE = /^\d{4}-\d{2}-\d{2}$/
  const DATETIME_RE = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}(?::\d{2}(?:\.\d{1,6})?)?)$/
  const TIME_RE = /^\d{2}:\d{2}(?::\d{2}(?:\.\d{1,6})?)?$/

  function pickerValue(text: string): string | null {
    if (text === '') return ''
    if (kind === 'date') return DATE_RE.test(text) ? text : null
    if (kind === 'time') return TIME_RE.test(text) ? text : null
    const m = DATETIME_RE.exec(text)
    return m ? `${m[1]}T${m[2]}` : null
  }

  let text = $state(init.initial ?? startText)
  let touched = $state(init.initial !== undefined)
  // The picker is only offered when the loaded value fits it; otherwise raw text.
  let picker = $state((kind === 'date' || kind === 'datetime' || kind === 'time') && pickerValue(init.initial ?? startText) !== null)
  let pickerText = $state(pickerValue(init.initial ?? startText) ?? '')
  let done = false

  let enumValue = $state(startIsNull ? '__null' : startText)

  const boolOptions = ['true', 'false']
  let boolValue = $state(
    init.value === true || init.value === 1 || init.value === '1' || init.value === 'true'
      ? 'true'
      : init.value === false || init.value === 0 || init.value === '0' || init.value === 'false'
        ? 'false'
        : startIsNull
          ? '__null'
          : 'true',
  )

  function boolInput(v: boolean): CellInput {
    if (engine === 'mongodb') return { kind: 'json', value: v }
    if (engine === 'postgres') return { kind: 'text', value: v ? 'true' : 'false' }
    return { kind: 'text', value: v ? '1' : '0' }
  }

  function textInput(value: string): CellInput {
    // An emptied number/date field means NULL, not an empty string the server would reject.
    if (value === '' && nullable && (isNumericFamily(family) || ['date', 'time', 'dateTime', 'uuid', 'boolean'].includes(family))) {
      return { kind: 'null' }
    }
    return { kind: 'text', value }
  }

  function finish(input: CellInput | null, move: EditMove = 'none') {
    if (done) return
    done = true
    if (input === null) ctx.cancel()
    else ctx.commit(input, move)
  }

  function currentInput(): CellInput | null {
    if (kind === 'bool') {
      if (boolValue === '__null') return startIsNull ? null : { kind: 'null' }
      return boolInput(boolValue === 'true')
    }
    if (kind === 'enum') {
      if (enumValue === '__null') return startIsNull ? null : { kind: 'null' }
      return enumValue === startText && !startIsNull ? null : { kind: 'text', value: enumValue }
    }
    if (!touched) return null
    if (picker && (kind === 'date' || kind === 'datetime' || kind === 'time')) {
      let v = pickerText
      if (v !== '' && kind === 'datetime') {
        v = v.replace('T', ' ')
        if (/ \d{2}:\d{2}$/.test(v)) v += ':00'
      } else if (v !== '' && kind === 'time' && /^\d{2}:\d{2}$/.test(v)) {
        v += ':00'
      }
      return textInput(v)
    }
    return textInput(text)
  }

  function setNull() {
    if (!nullable) return
    finish(startIsNull ? null : { kind: 'null' })
  }

  function onKeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      finish(null)
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      e.stopPropagation()
      finish(currentInput(), 'down')
    } else if (e.key === 'Tab') {
      e.preventDefault()
      e.stopPropagation()
      finish(currentInput(), e.shiftKey ? 'left' : 'right')
    } else if (mod && ((e.shiftKey && (e.key === 'n' || e.key === 'N')) || e.key === 'Backspace')) {
      e.preventDefault()
      e.stopPropagation()
      setNull()
    } else {
      // Keep arrows, Home/End and typing inside the editor instead of moving the grid selection.
      e.stopPropagation()
    }
  }

  function onBlur() {
    finish(currentInput())
  }

  function togglePicker() {
    if (picker) {
      // Keep what the picker holds when switching to raw text.
      const v = currentInput()
      if (v && v.kind === 'text') text = v.value
      picker = false
    } else {
      const pv = pickerValue(text)
      if (pv === null) return
      pickerText = pv
      picker = true
    }
    touched = true
  }

  const canPick = $derived(picker || pickerValue(text) !== null)

  function focusSelect(node: HTMLInputElement | HTMLSelectElement) {
    node.focus()
    if (!(node instanceof HTMLInputElement) || node.type !== 'text') return
    const place = () => {
      if (init.initial !== undefined) node.setSelectionRange(node.value.length, node.value.length)
      else node.select()
    }
    place()
    // The double click that opened the editor can still move the caret afterwards.
    setTimeout(() => {
      if (!touched) place()
    }, 0)
  }
</script>

<div class="ed" style:width="{ctx.width}px" style:height="{ctx.height}px">
  {#if kind === 'bool'}
    <select
      class="grow"
      use:focusSelect
      bind:value={boolValue}
      onkeydown={onKeydown}
      onchange={() => finish(currentInput(), 'none')}
      onblur={() => finish(null)}
    >
      {#each boolOptions as o (o)}
        <option value={o}>{o}</option>
      {/each}
      {#if nullable || startIsNull}<option value="__null">NULL</option>{/if}
    </select>
  {:else if kind === 'enum'}
    <select
      class="grow"
      use:focusSelect
      bind:value={enumValue}
      onkeydown={onKeydown}
      onchange={() => finish(currentInput())}
      onblur={() => finish(null)}
    >
      {#each enumValues ?? [] as o (o)}
        <option value={o}>{o}</option>
      {/each}
      {#if nullable}<option value="__null">NULL</option>{/if}
    </select>
  {:else if picker && kind === 'date'}
    <input class="grow" type="date" use:focusSelect bind:value={pickerText} oninput={() => (touched = true)} onkeydown={onKeydown} onblur={onBlur} />
  {:else if picker && kind === 'datetime'}
    <input class="grow" type="datetime-local" step="1" use:focusSelect bind:value={pickerText} oninput={() => (touched = true)} onkeydown={onKeydown} onblur={onBlur} />
  {:else if picker && kind === 'time'}
    <input class="grow" type="time" step="1" use:focusSelect bind:value={pickerText} oninput={() => (touched = true)} onkeydown={onKeydown} onblur={onBlur} />
  {:else}
    <input
      class="grow"
      type="text"
      spellcheck="false"
      autocomplete="off"
      placeholder={startIsNull && !touched ? 'NULL' : ''}
      use:focusSelect
      bind:value={text}
      oninput={() => (touched = true)}
      onkeydown={onKeydown}
      onblur={onBlur}
    />
  {/if}

  {#if kind === 'date' || kind === 'datetime' || kind === 'time'}
    <!-- mousedown is prevented so the input keeps focus (no blur-commit) -->
    <button
      class="mini"
      type="button"
      title={picker ? 'Edit as raw text' : 'Use the date picker'}
      disabled={!canPick}
      onmousedown={(e) => e.preventDefault()}
      onclick={togglePicker}>{picker ? 'Aa' : '▦'}</button
    >
  {/if}
  {#if nullable && kind !== 'enum' && kind !== 'bool'}
    <button
      class="mini"
      type="button"
      title="Set NULL (⌘⇧N)"
      onmousedown={(e) => e.preventDefault()}
      onclick={setNull}>NULL</button
    >
  {/if}
</div>

<style>
  .ed {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 1px;
    background: var(--bg);
  }

  .grow {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    border-radius: 3px;
    padding: 0 6px;
    font-size: 12px;
    border-color: transparent;
    background: var(--bg-input, var(--bg));
  }

  .mini {
    flex: 0 0 auto;
    padding: 0 5px;
    font-size: 10px;
    line-height: 1;
    border-radius: 3px;
    color: var(--fg-muted);
  }
</style>
