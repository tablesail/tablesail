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

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
