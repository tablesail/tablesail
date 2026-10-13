<!--
  Tab strip, TablePlus-style: tabs share the row equally with centred titles,
  the active one raised as a white card. Arrows step between tabs.
-->
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
  let index = $derived(list.findIndex((t) => t.id === activeId))

  let strip = $state<HTMLDivElement>()

  // Keep the active tab visible when there are more tabs than fit.
  $effect(() => {
    const id = activeId
    void list.length
    const el = strip?.querySelector<HTMLElement>(`[data-tab="${CSS.escape(id ?? '')}"]`)
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })

  function step(d: number) {
    const t = list[index + d]
    if (t) tabs.select(t.id)
  }

  // Arrow keys / Home / End move the focus along the strip (Enter or Space activates the focused tab).
  function onStripKey(e: KeyboardEvent) {
    const cur = (e.target as HTMLElement).closest<HTMLElement>('[role=tab]')
    if (!cur || !strip) return
    const all = [...strip.querySelectorAll<HTMLElement>('[role=tab]')]
    const i = all.indexOf(cur)
    let next: HTMLElement | undefined
    if (e.key === 'ArrowRight') next = all[(i + 1) % all.length]
    else if (e.key === 'ArrowLeft') next = all[(i - 1 + all.length) % all.length]
    else if (e.key === 'Home') next = all[0]
    else if (e.key === 'End') next = all[all.length - 1]
    else return
    e.preventDefault()
    next?.focus()
  }

  function onAux(e: MouseEvent, id: string) {
    if (e.button === 1) {
      e.preventDefault()
      tabs.close(id)
    }
  }
</script>

<div class="bar">
  <div class="nav">
    <button class="icon" tabindex="-1" aria-label="Previous tab" title="Previous tab" disabled={index <= 0} onclick={() => step(-1)}>
      <Icon name="chevron-left" size={14} strokeWidth={1.6} />
    </button>
    <button class="icon" tabindex="-1" aria-label="Next tab" title="Next tab" disabled={index < 0 || index >= list.length - 1} onclick={() => step(1)}>
      <Icon name="chevron-right" size={14} strokeWidth={1.6} />
    </button>
  </div>
  <div class="strip" role="tablist" aria-label="Open tabs" bind:this={strip}>
    {#each list as t (t.id)}
      <!-- The close button is a sibling of the tab (not inside it) so the tab's name is just its title. -->
      <div class="item" role="presentation">
        <div
          class="tab"
          class:active={t.id === activeId}
          data-tab={t.id}
          role="tab"
          aria-selected={t.id === activeId}
          tabindex={t.id === activeId ? 0 : -1}
          title={t.kind === 'table' ? [t.table.schema, t.table.name].filter(Boolean).join('.') : t.title}
          onclick={() => tabs.select(t.id)}
          ondblclick={() => tabs.keep(t.id)}
          onauxclick={(e) => onAux(e, t.id)}
          onmousedown={(e) => e.button === 1 && e.preventDefault()}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              tabs.select(t.id)
            } else onStripKey(e)
          }}
        >
          <span class="t" class:preview={t.kind === 'table' && t.preview}>{t.title}</span>
        </div>
        <button
          class="close icon"
          aria-label="Close tab {t.title}"
          tabindex="-1"
          onmousedown={(e) => e.preventDefault()}
          onclick={(e) => {
            e.stopPropagation()
            tabs.close(t.id)
          }}><Icon name="close" size={9} strokeWidth={1.8} /></button
        >
      </div>
    {/each}
  </div>
  <button
    class="add icon"
    title="New query"
    aria-label="New query"
    onmousedown={(e) => e.preventDefault()}
    onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}
    ><Icon name="plus" size={14} /></button
  >
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 34px;
    padding: 0 6px;
    background: var(--bg-tabbar);
    border-bottom: 0.5px solid var(--border-strong);
    flex: none;
  }

  .nav {
    display: flex;
    flex: none;
  }

  .nav .icon {
    width: 24px;
    min-width: 24px;
    min-height: 24px;
    height: 24px;
  }

  .strip {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 3px;
    height: 26px;
    overflow-x: auto;
  }

  .strip::-webkit-scrollbar {
    height: 0;
  }

  .item {
    position: relative;
    flex: 1 1 0;
    min-width: 110px;
    display: flex;
  }

  .tab {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 24px;
    border-radius: 6px;
    color: var(--fg-muted);
    font-size: 12.5px;
    outline: none;
  }

  .tab:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .tab.active:focus-visible {
    box-shadow:
      0 0 0 2px var(--accent),
      var(--btn-shadow);
  }

  .tab:hover:not(.active) {
    background: var(--bg-hover);
  }

  .tab.active {
    background: var(--bg);
    color: var(--fg);
    box-shadow:
      0 0 0 0.5px var(--border-strong),
      var(--btn-shadow);
  }

  .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Preview tab: replaced by the next table you click; double-click to keep it. */
  .t.preview {
    font-style: italic;
  }

  .close {
    position: absolute;
    left: 5px;
    top: 50%;
    transform: translateY(-50%);
    width: 16px;
    min-width: 16px;
    min-height: 16px;
    height: 16px;
    border-radius: 4px;
    opacity: 0;
  }

  .item:hover .close {
    opacity: 1;
  }

  .close:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .add {
    flex: none;
    width: 24px;
    min-width: 24px;
    min-height: 24px;
    height: 24px;
    border-radius: 6px;
  }
</style>
