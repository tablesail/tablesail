<script lang="ts">
  import { tabs } from '../stores/tabs.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import type { SessionInfo } from '../types'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let list = $derived(tabs.forSession(session.sessionId))
  let activeId = $derived(tabs.activeBySession[session.sessionId])

  function onAux(e: MouseEvent, id: string) {
    if (e.button === 1) {
      e.preventDefault()
      tabs.close(id)
    }
  }
</script>

<div class="bar" role="tablist">
  {#each list as t (t.id)}
    <div
      class="tab"
      class:active={t.id === activeId}
      role="tab"
      aria-selected={t.id === activeId}
      tabindex="-1"
      title={t.kind === 'table' ? [t.table.schema, t.table.name].filter(Boolean).join('.') : t.title}
      onclick={() => tabs.select(t.id)}
      onauxclick={(e) => onAux(e, t.id)}
      onmousedown={(e) => e.button === 1 && e.preventDefault()}
      onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && tabs.select(t.id)}
    >
      <span class="kind">{t.kind === 'table' ? '▦' : '›_'}</span>
      <span class="label">{t.title}</span>
      <button
        class="close ghost"
        aria-label="Close tab"
        onclick={(e) => {
          e.stopPropagation()
          tabs.close(t.id)
        }}>×</button
      >
    </div>
  {/each}
  <button
    class="add ghost"
    title="New query"
    aria-label="New query"
    onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}>+</button
  >
</div>

<style>
  .bar {
    display: flex;
    align-items: stretch;
    background: var(--bg-sidebar);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex: none;
    height: 32px;
  }

  .bar::-webkit-scrollbar {
    height: 0;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 12px;
    max-width: 200px;
    border-right: 1px solid var(--border);
    color: var(--fg-muted);
    flex: none;
  }

  .tab:hover {
    background: var(--bg-hover);
  }

  .tab.active {
    background: var(--bg);
    color: var(--fg);
    box-shadow: inset 0 -2px 0 var(--accent);
  }

  .kind {
    font-size: 10px;
    opacity: 0.7;
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    padding: 0 5px;
    line-height: 1.2;
    font-size: 15px;
    border-radius: 4px;
  }

  .add {
    padding: 0 12px;
    font-size: 16px;
    border-radius: 0;
  }
</style>
