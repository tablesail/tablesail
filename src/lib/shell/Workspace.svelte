<!--
  Main window for the active connection. Every open tab stays mounted (hidden when inactive)
  so running queries, scroll positions and editor state survive tab and connection switches.
-->
<script lang="ts">
  import Sidebar from '../sidebar/Sidebar.svelte'
  import QueryTab from '../editor/QueryTab.svelte'
  import TableView from '../table/TableView.svelte'
  import ConsoleLog from './ConsoleLog.svelte'
  import Inspector from './Inspector.svelte'
  import TabBar from './TabBar.svelte'
  import TopBar from './TopBar.svelte'
  import { inspector } from '../stores/inspector.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import { modLabel } from '../util/format'

  interface Props {
    visible: boolean
    appVersion: string
  }

  let { visible, appVersion }: Props = $props()

  const WIDTH_KEY = 'tablesoft.sidebarWidth'
  function readWidth(): number {
    try {
      const n = Number(localStorage.getItem(WIDTH_KEY))
      if (n >= 180 && n <= 600) return n
    } catch {
      // ignore
    }
    return 230
  }

  let sidebarWidth = $state(readWidth())
  let windowWidth = $state(typeof window === 'undefined' ? 1400 : window.innerWidth)
  /** Small windows get slimmer side panels (instead of hiding the details panel). */
  let narrow = $derived(windowWidth < 1120)

  const LOG_KEY = 'tablesoft.consoleHeight'
  let logHeight = $state(readLogHeight())

  function readLogHeight(): number {
    try {
      const n = Number(localStorage.getItem(LOG_KEY))
      if (n >= 80 && n <= 900) return n
    } catch {
      // ignore
    }
    return 220
  }

  function onLogResizeDown(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const startY = e.clientY
    const startH = logHeight
    const move = (ev: PointerEvent) => {
      logHeight = Math.min(900, Math.max(80, startH + (startY - ev.clientY)))
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      try {
        localStorage.setItem(LOG_KEY, String(Math.round(logHeight)))
      } catch {
        // ignore
      }
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }

  let session = $derived(sessions.active)
  let activeTab = $derived(tabs.active(session?.sessionId ?? null))
  let sessionTabs = $derived(session ? tabs.forSession(session.sessionId) : [])

  function onResizeDown(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      sidebarWidth = Math.min(600, Math.max(180, ev.clientX))
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      try {
        localStorage.setItem(WIDTH_KEY, String(Math.round(sidebarWidth)))
      } catch {
        // ignore
      }
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }
</script>

<svelte:window bind:innerWidth={windowWidth} />

<div class="workspace" hidden={!visible || !session}>
  {#if session}
    <TopBar {session} />
    <div
      class="body"
      style:grid-template-columns="{ui.sidebarHidden ? '0px 0px' : `${narrow ? Math.min(sidebarWidth, 200) : sidebarWidth}px 1px`} minmax(0, 1fr) {inspector.open ? (narrow ? '260px' : '300px') : '0px'}"
    >
      {#key session.sessionId}
        <div class="side"><Sidebar {session} /></div>
      {/key}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="resize" class:off={ui.sidebarHidden} onpointerdown={onResizeDown}></div>
      <section class="main">
        <TabBar {session} />
        <div class="tabs">
          {#each tabs.list as tab (tab.id)}
            {@const s = sessions.list.find((x) => x.sessionId === tab.sessionId)}
            {#if s}
              {@const shown = visible && tab.sessionId === session.sessionId && activeTab?.id === tab.id}
              <div class="tab-body" hidden={!shown}>
                {#if tab.kind === 'table'}
                  <TableView
                    session={s}
                    table={tab.table}
                    tabId={tab.id}
                    initialFilters={tab.filters}
                    active={shown}
                    onExport={(request) => ui.exportTable(s, request)}
                    onImport={(table) => ui.importInto(s, table)}
                  />
                {:else}
                  <QueryTab {tab} session={s} active={shown} />
                {/if}
              </div>
            {/if}
          {/each}
          {#if sessionTabs.length === 0}
            <div class="empty">
              <div class="hint">
                <h2>{session.config.name}</h2>
                <p class="muted">Pick a table in the sidebar, or start a query.</p>
                <button
                  class="primary"
                  onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}
                >
                  New query
                </button>
                <dl>
                  <dt><kbd>{modLabel}</kbd><kbd>T</kbd></dt>
                  <dd>New query</dd>
                  <dt><kbd>{modLabel}</kbd><kbd>K</kbd></dt>
                  <dd>Search tables and commands</dd>
                  <dt><kbd>{modLabel}</kbd><kbd>/</kbd></dt>
                  <dd>All shortcuts</dd>
                </dl>
              </div>
            </div>
          {/if}
        </div>
        {#if ui.console}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="log-resize" onpointerdown={onLogResizeDown}></div>
          <div class="log" style:height="{logHeight}px">
            <ConsoleLog onClose={() => (ui.console = false)} />
          </div>
        {/if}
      </section>
      {#if inspector.open}
        <Inspector {session} activeTabId={activeTab?.id ?? null} />
      {:else}
        <span></span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .workspace {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .workspace[hidden] {
    display: none;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr);
    overflow: hidden;
    background: var(--bg-chrome);
  }

  .side {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .resize.off {
    pointer-events: none;
  }

  .resize {
    position: relative;
    cursor: col-resize;
    background: var(--border);
    min-height: 0;
    touch-action: none;
    z-index: 2;
  }

  /* Wider invisible grab area around the 1px divider. */
  .resize::after {
    content: '';
    position: absolute;
    inset: 0 -4px;
  }

  .resize:hover {
    background: var(--accent);
  }

  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
  }

  .log-resize {
    position: relative;
    height: 1px;
    flex: none;
    background: var(--border-strong);
    cursor: row-resize;
    touch-action: none;
    z-index: 2;
  }

  .log-resize::after {
    content: '';
    position: absolute;
    inset: -3px 0;
  }

  .log {
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .log > :global(*) {
    flex: 1;
    min-height: 0;
  }

  .tabs {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .tab-body {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
  }

  .tab-body[hidden] {
    display: none;
  }

  .tab-body > :global(*) {
    flex: 1;
    min-height: 0;
  }

  .empty {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .hint {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    text-align: center;
  }

  .hint h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  .hint p {
    margin: 0;
  }

  .hint dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 8px 12px;
    margin: 18px 0 0;
    align-items: center;
    color: var(--fg-muted);
    font-size: 12px;
  }

  .hint dt {
    display: flex;
    gap: 3px;
    justify-content: flex-end;
  }

  .hint dd {
    margin: 0;
    text-align: left;
  }
</style>
