<!--
  Main window for the active connection. Every open tab stays mounted (hidden when inactive)
  so running queries, scroll positions and editor state survive tab and connection switches.
-->
<script lang="ts">
  import Sidebar from '../sidebar/Sidebar.svelte'
  import QueryTab from '../editor/QueryTab.svelte'
  import TableView from '../table/TableView.svelte'
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
    <div class="body" style:grid-template-columns="{sidebarWidth}px 1px 1fr">
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
                  <dt><kbd>{modLabel}</kbd><kbd>P</kbd></dt>
                  <dd>Open any table</dd>
                  <dt><kbd>{modLabel}</kbd><kbd>K</kbd></dt>
                  <dd>Switch connection</dd>
                  <dt><kbd>{modLabel}</kbd><kbd>/</kbd></dt>
                  <dd>All shortcuts</dd>
                </dl>
              </div>
            </div>
          {/if}
        </div>
      </section>
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
  }

  .resize {
    position: relative;
    cursor: col-resize;
    background: var(--border);
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
