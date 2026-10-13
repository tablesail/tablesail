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
      <button
        class="close icon"
        aria-label="Close tab"
        tabindex="-1"
        onmousedown={(e) => e.preventDefault()}
        onclick={(e) => {
          e.stopPropagation()
          tabs.close(t.id)
        }}><Icon name="close" size={10} strokeWidth={1.8} /></button
      >
      <span class="label"><span class="kind"><Icon name={t.kind === 'table' ? 'table' : 'code'} size={13} /></span><span class="t">{t.title}</span></span>
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
    align-items: stretch;
    background: var(--bg-tabbar);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    overflow-y: hidden;
    flex: none;
    height: 32px;
  }

  .bar::-webkit-scrollbar {
    height: 0;
  }

  .tab {
    position: relative;
    flex: 0 0 auto;
    min-width: 0;
    max-width: 240px;
    display: flex;
    align-items: center;
    padding: 0 26px;
    border-right: 1px solid var(--border);
    color: var(--fg-muted);
    font-size: 11px;
  }

  .tab:hover:not(.active) {
    background: var(--bg-active);
  }

  .tab.active {
    background: var(--bg);
    color: var(--fg);
  }

  .tab.active::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent);
  }

  .label {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }

  .t {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .kind {
    display: inline-flex;
    flex: none;
    opacity: 0.7;
  }

  .active .kind {
    color: var(--accent);
    opacity: 1;
  }

  .close {
    position: absolute;
    left: 5px;
    top: 50%;
    transform: translateY(-50%);
    width: 17px;
    min-width: 17px;
    min-height: 17px;
    height: 17px;
    border-radius: 4px;
    opacity: 0;
  }

  .tab:hover .close {
    opacity: 1;
  }

  .close:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .add {
    flex: none;
    align-self: center;
    margin: 0 4px;
    border-radius: 6px;
  }

  .add:hover:not(:disabled) {
    background: var(--bg-active);
  }
</style>
