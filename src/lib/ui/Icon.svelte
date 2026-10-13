<!--
  Inline stroke icons on a 16×16 grid. <Icon name="table" size={14} />
  Colour follows `currentColor`.
-->
<script lang="ts" module>
  export type IconName =
    | 'table'
    | 'view'
    | 'matview'
    | 'collection'
    | 'query'
    | 'refresh'
    | 'chevron-right'
    | 'chevron-down'
    | 'chevron-left'
    | 'plus'
    | 'close'
    | 'more'
    | 'filter'
    | 'search'
    | 'play'
    | 'play-all'
    | 'stop'
    | 'export'
    | 'import'
    | 'trash'
    | 'eye'
    | 'check'
    | 'database'
    | 'key'
    | 'lock'
    | 'columns'
    | 'rows'
    | 'undo'
    | 'server'
    | 'sequence'
    | 'pulse'
    | 'code'
    | 'clock'
    | 'star'
    | 'layers'
    | 'activity'
    | 'settings'
    | 'bolt'
    | 'panel'
    | 'shield'
    | 'command'
    | 'link'
    | 'sun'
    | 'moon'
    | 'grid'

  const PATHS: Record<IconName, string> = {
    table: 'M2.5 3.5h11v9h-11z M2.5 6.5h11 M2.5 9.5h11 M6.5 6.5v6',
    view: 'M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
    matview: 'M2.5 3.5h11v9h-11z M2.5 6.5h11 M8 8.2a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2z',
    collection: 'M8 2.5c3 0 5.5.9 5.5 2s-2.5 2-5.5 2-5.5-.9-5.5-2 2.5-2 5.5-2z M2.5 4.5v7c0 1.1 2.5 2 5.5 2s5.5-.9 5.5-2v-7 M2.5 8c0 1.1 2.5 2 5.5 2s5.5-.9 5.5-2',
    query: 'M2.5 3.5h11v9h-11z M4.5 6.5l2 1.5-2 1.5 M8 10h3',
    refresh: 'M13 8a5 5 0 1 1-1.5-3.6 M13 2.5v3h-3',
    'chevron-right': 'M6 3.5l4.5 4.5L6 12.5',
    'chevron-down': 'M3.5 6L8 10.5 12.5 6',
    'chevron-left': 'M10 3.5L5.5 8l4.5 4.5',
    plus: 'M8 3v10 M3 8h10',
    close: 'M4 4l8 8 M12 4l-8 8',
    more: 'M3.5 8h.01 M8 8h.01 M12.5 8h.01',
    filter: 'M2.5 3.5h11L9.5 8.5v4l-3-1.5V8.5z',
    search: 'M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9z M10.3 10.3l3.2 3.2',
    play: 'M5 3.5v9l7-4.5z',
    'play-all': 'M3 3.5v9l5.5-4.5z M8.5 3.5v9l5.5-4.5z',
    stop: 'M4 4h8v8H4z',
    export: 'M8 10V2.5 M5 5.5l3-3 3 3 M3 9.5v3.5h10V9.5',
    import: 'M8 2.5V10 M5 7l3 3 3-3 M3 9.5v3.5h10V9.5',
    trash: 'M3 4.5h10 M6.5 4.5V3h3v1.5 M4.5 4.5l.7 8.5h5.6l.7-8.5',
    eye: 'M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
    check: 'M3 8.5l3 3 7-7',
    database: 'M8 2.5c3 0 5.5.9 5.5 2s-2.5 2-5.5 2-5.5-.9-5.5-2 2.5-2 5.5-2z M2.5 4.5v7c0 1.1 2.5 2 5.5 2s5.5-.9 5.5-2v-7',
    key: 'M5.5 10.5a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M8 8.5l5.5 5.5 M11.5 12l1.5-1.5',
    lock: 'M4 7.5h8v6H4z M5.5 7.5V5.5a2.5 2.5 0 0 1 5 0v2',
    columns: 'M2.5 3.5h11v9h-11z M6.2 3.5v9 M9.8 3.5v9',
    rows: 'M2.5 3.5h11v9h-11z M2.5 6.5h11 M2.5 9.5h11',
    undo: 'M5 4.5L2.5 7 5 9.5 M2.5 7H10a3.5 3.5 0 0 1 0 7H7',
    server: 'M2.5 3h11v4h-11z M2.5 9h11v4h-11z M5 5h.01 M5 11h.01',
    sequence: 'M3 5h10 M3 8h10 M3 11h6',
    pulse: '',
    code: '',
    clock: '',
    star: '',
    layers: '',
    activity: '',
    settings: '',
    bolt: '',
    panel: '',
    shield: '',
    command: '',
    link: '',
    sun: '',
    moon: '',
    grid: '',
  }

  // Icons drawn on a 24×24 grid (from the Table Pulse concept), as inner SVG markup.
  const ICONS24: Partial<Record<IconName, string>> = {
    pulse: '<path d="M2 12h4l3.1-7 5.1 14 3.1-7H22"/>',
    code: '<path d="m8 7-5 5 5 5m8-10 5 5-5 5m-3-13-3 16"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/>',
    star: '<path d="m12 2 3.1 6.4 7.1 1-5.1 5 .9 7-6-3.3L5.9 21l1-6.6-5-5 7-1L12 2z"/>',
    layers: '<path d="m12 2 9 5-9 5-9-5 9-5Zm-9 10 9 5 9-5M3 17l9 5 9-5"/>',
    activity: '<path d="M3 18v-6m6 6V6m6 12v-9m6 9V3"/>',
    settings:
      '<path d="m10 2-.5 2.2-2 1.2-2.1-.7-2 3.5 1.7 1.5v2.4l-1.7 1.4 2 3.6 2.2-.8 2 1.2L10 20h4l.5-2.3 2-1.1 2.1.7 2-3.6-1.7-1.5V9.8l1.7-1.4-2-3.6-2.1.8-2-1.3L14 2z"/><circle cx="12" cy="11" r="2.5"/>',
    bolt: '<path d="m13 2-9 12h7l-1 8 10-13h-8l1-7Z"/>',
    panel: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>',
    shield: '<path d="m12 2 8 4v6c0 5-3 8-8 10-5-2-8-5-8-10V6l8-4Z"/><path d="m9 12 2 2 4-4"/>',
    command: '<path d="M8 8H5a3 3 0 1 1 3-3v14a3 3 0 1 1-3-3h14a3 3 0 1 1-3 3V5a3 3 0 1 1 3 3H8Z"/>',
    link: '<path d="M10 14a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1.5 1.5M14 10a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1.5-1.5"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z"/>',
    database: '<ellipse cx="12" cy="5" rx="8.5" ry="3.1"/><path d="M3.5 5v14c0 2 17 4 17 0V5M3.5 12c0 4 17 4 17 0"/>',
    table: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18M9 4v16"/>',
    grid: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18M9 4v16"/>',
  }
</script>

<script lang="ts">
  interface Props {
    name: IconName
    size?: number
    strokeWidth?: number
    class?: string
  }

  let { name, size = 14, strokeWidth = 1.4, class: cls = '' }: Props = $props()
</script>

{#if ICONS24[name]}
  <svg
    class="icon {cls}"
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width={strokeWidth * 1.2}
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {@html ICONS24[name]}
  </svg>
{:else}
  <svg
    class="icon {cls}"
    width={size}
    height={size}
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width={name === 'more' ? 2.4 : strokeWidth}
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d={PATHS[name]} />
  </svg>
{/if}

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
