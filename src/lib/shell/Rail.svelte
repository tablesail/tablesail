<!-- Left icon rail: switches the explorer panel and holds global actions. -->
<script lang="ts">
  import Icon, { type IconName } from '../ui/Icon.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui, type RailView } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { modLabel } from '../util/format'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  const VIEWS: { view: RailView; icon: IconName; title: string }[] = [
    { view: 'browser', icon: 'database', title: 'Database browser' },
    { view: 'history', icon: 'clock', title: 'Query history' },
    { view: 'saved', icon: 'star', title: 'Saved queries' },
  ]

</script>

<nav class="rail" aria-label="Main navigation">
  {#each VIEWS as v (v.view)}
    <button class="rb" class:active={ui.rail === v.view} title={v.title} aria-label={v.title} onclick={() => (ui.rail = v.view)}>
      <Icon name={v.icon} size={18} />
    </button>
  {/each}
  <button
    class="rb"
    title="New SQL editor ({modLabel}T)"
    aria-label="New SQL editor"
    onclick={() => tabs.newQuery(session.sessionId, sessions.database(session.sessionId))}
  >
    <Icon name="code" size={18} />
  </button>

  <span class="spacer"></span>

  <button class="rb" title="Connections" aria-label="Connections" onclick={() => (sessions.view = 'connections')}>
    <Icon name="plus" size={18} />
  </button>
  <button class="rb" title="Settings" aria-label="Settings" onclick={() => (ui.settings = true)}>
    <Icon name="settings" size={18} />
  </button>
</nav>

<style>
  .rail {
    width: 55px;
    min-width: 55px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
    padding: 14px 7px;
    background: var(--bg-rail);
    border-right: 1px solid var(--border);
  }

  .rb {
    position: relative;
    width: 37px;
    min-width: 37px;
    height: 37px;
    padding: 0;
    border: none;
    border-radius: 9px;
    background: none;
    box-shadow: none;
    color: var(--fg-faint);
  }

  .rb:hover:not(:disabled) {
    background: var(--bg-input);
    color: var(--fg);
  }

  .rb.active,
  .rb.active:hover:not(:disabled) {
    background: var(--accent-solid);
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .rb.active {
    color: color-mix(in srgb, var(--accent) 55%, #2d3a12);
  }

  .rb.active::before {
    content: '';
    position: absolute;
    left: -7px;
    top: 8px;
    width: 3px;
    height: 20px;
    border-radius: 0 3px 3px 0;
    background: var(--accent);
  }

  .spacer {
    flex: 1;
  }
</style>
