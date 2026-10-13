<!-- Coloured tile identifying a database engine. -->
<script lang="ts">
  import type { Engine } from '../types'

  interface Props {
    engine: Engine
    size?: number
  }

  let { engine, size = 30 }: Props = $props()

  const STYLE: Record<Engine, { label: string; bg: string }> = {
    postgres: { label: 'Pg', bg: '#336791' },
    mysql: { label: 'My', bg: '#00758f' },
    mariadb: { label: 'Ma', bg: '#a3612a' },
    sqlite: { label: 'Lt', bg: '#4a7fb0' },
    mongodb: { label: 'Mo', bg: '#3f8f3b' },
  }

  let s = $derived(STYLE[engine])
</script>

<span
  class="engine"
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.4)}px"
  style:background={s.bg}
  aria-hidden="true">{s.label}</span
>

<style>
  .engine {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 22%;
    color: #fff;
    font-weight: 700;
    letter-spacing: -0.02em;
    box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.08), inset 0 1px 0 rgb(255 255 255 / 0.15);
  }
</style>
