<!--
  Main window for the active connection. Every open tab stays mounted (hidden when inactive)
  so running queries, scroll positions and editor state survive tab and connection switches.
-->
<script lang="ts">
  import Sidebar from '../sidebar/Sidebar.svelte'
  import QueryTab from '../editor/QueryTab.svelte'
  import TableView from '../table/TableView.svelte'
  import StatusBar from './StatusBar.svelte'
  import TabBar from './TabBar.svelte'
  import TopBar from './TopBar.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import { modLabel } from '../util/format'

  interface Props {
    visible: boolean
  }

  let { visible }: Props = $props()

  const WIDTH_KEY = 'tablesoft.sidebarWidth'
  function readWidth(): number {
    try {
      const n = Number(localStorage.getItem(WIDTH_KEY))
      if (n >= 160 && n <= 600) return n
    } catch {
      // ignore
    }
    return 250
  }

  let sidebarWidth = $state(readWidth())

  let session = $derived(sessions.active)
  let activeTab = $derived(tabs.active(session?.sessionId ?? null))
  let sessionTabs = $derived(session ? tabs.forSession(session.sessionId) : [])

  function onResizeDown(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      sidebarWidth = Math.min(600, Math.max(160, ev.clientX))
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

<div class="workspace" hidden={!visible || !session}>
  {#if session}
    <TopBar {session} />
    <div class="body" style:grid-template-columns="{sidebarWidth}px 4px 1fr">
      {#key session.sessionId}
        <Sidebar {session} />
      {/key}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="resize" onpointerdown={onResizeDown}></div>
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
              <p>Open a table from the sidebar, or start a query.</p>
              <button
                class="primary"
                onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}
              >
                New query ({modLabel}+T)
              </button>
              <p class="muted">{modLabel}+P jumps to any table.</p>
            </div>
          {/if}
        </div>
      </section>
    </div>
    <StatusBar {session} />
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
  }

  .resize {
    cursor: col-resize;
    background: var(--border);
    touch-action: none;
  }

  .resize:hover {
    background: var(--accent);
  }

  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
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
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
</style>
