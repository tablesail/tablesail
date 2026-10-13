<script lang="ts">
  import { tabs } from '../stores/tabs.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import Icon from '../ui/Icon.svelte'
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
      <span class="kind"><Icon name={t.kind === 'table' ? 'table' : 'query'} size={13} /></span>
      <span class="label">{t.title}</span>
      <button
        class="close icon"
        aria-label="Close tab"
        tabindex="-1"
        onclick={(e) => {
          e.stopPropagation()
          tabs.close(t.id)
        }}><Icon name="close" size={11} strokeWidth={1.8} /></button
      >
    </div>
  {/each}
  <button
    class="add icon"
    title="New query"
    aria-label="New query"
    tabindex="-1"
    onmousedown={(e) => e.preventDefault()}
    onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}
    ><Icon name="plus" size={14} /></button
  >
</div>

<style>
  .bar {
    display: flex;
    align-items: flex-end;
    gap: 1px;
    padding: 0 6px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    overflow-y: hidden;
    flex: none;
    height: 34px;
  }

  .bar::-webkit-scrollbar {
    height: 0;
  }

  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 29px;
    padding: 0 4px 0 10px;
    min-width: 90px;
    max-width: 210px;
    border-radius: 7px 7px 0 0;
    color: var(--fg-muted);
    flex: none;
    margin-bottom: -1px;
    border: 1px solid transparent;
    border-bottom: none;
  }

  .tab:hover:not(.active) {
    background: var(--bg-hover);
  }

  .tab:not(.active):not(:hover) + .tab:not(.active):not(:hover)::before {
    content: '';
    position: absolute;
    left: -1px;
    top: 8px;
    bottom: 8px;
    width: 1px;
    background: var(--border-strong);
  }

  .tab.active {
    background: var(--bg);
    color: var(--fg);
    border-color: var(--border);
    height: 30px;
  }

  .kind {
    opacity: 0.75;
  }

  .active .kind {
    color: var(--accent);
    opacity: 1;
  }

  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    width: 18px;
    min-width: 18px;
    min-height: 18px;
    height: 18px;
    border-radius: 4px;
    opacity: 0;
  }

  .tab:hover .close,
  .tab.active .close {
    opacity: 1;
  }

  .close:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .add {
    align-self: center;
    margin-left: 2px;
    border-radius: 6px;
  }

  .add:hover:not(:disabled) {
    background: var(--bg-hover);
  }
</style>
