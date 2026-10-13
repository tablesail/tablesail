<!--
  ⌘K command palette: quick actions, tables of the current database, saved queries
  and open connections, with keyboard navigation.
-->
<script lang="ts">
  import Icon, { type IconName } from '../ui/Icon.svelte'
  import { dialogFocus } from '../ui/dialogFocus'
  import { inspector } from '../stores/inspector.svelte'
  import { saved } from '../stores/saved.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { theme } from '../stores/theme.svelte'
  import { ui } from '../stores/ui.svelte'
  import { fuzzyFilter } from '../util/fuzzy'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  interface Command {
    group: string
    icon: IconName
    label: string
    hint?: string
    run: () => void
  }

  let query = $state('')
  let index = $state(0)
  let listEl = $state<HTMLDivElement>()

  let session = $derived(sessions.active)
  let database = $derived(session ? sessions.database(session.sessionId) : '')

  let commands = $derived.by<Command[]>(() => {
    const out: Command[] = []
    if (session) {
      const s = session
      out.push(
        { group: 'Quick actions', icon: 'code', label: 'New SQL editor', hint: '⌘T', run: () => tabs.newQuery(s.sessionId, database) },
        { group: 'Quick actions', icon: 'refresh', label: 'Refresh schema', run: () => sessions.refreshSchema(s.sessionId, database) },
        { group: 'Quick actions', icon: 'panel', label: 'Toggle row inspector', run: () => inspector.toggle() },
        { group: 'Quick actions', icon: 'clock', label: 'Show query history', run: () => (ui.rail = 'history') },
        { group: 'Quick actions', icon: 'star', label: 'Show saved queries', run: () => (ui.rail = 'saved') },
      )
    }
    out.push(
      { group: 'Quick actions', icon: theme.isDark ? 'sun' : 'moon', label: 'Toggle appearance', hint: '⌘⇧L', run: () => theme.toggle() },
      { group: 'Quick actions', icon: 'plus', label: 'Add database connection', run: () => (sessions.view = 'connections') },
      { group: 'Quick actions', icon: 'command', label: 'Keyboard shortcuts', hint: '⌘/', run: () => (ui.shortcutsHelp = true) },
      { group: 'Quick actions', icon: 'settings', label: 'Settings', run: () => (ui.settings = true) },
    )
    if (session) {
      const s = session
      const snap = sessions.schemaEntry(s.sessionId, database).snapshot
      for (const t of snap ? Object.values(snap.schemas).flat() : []) {
        out.push({
          group: 'Tables',
          icon: t.kind === 'view' || t.kind === 'materializedView' ? 'view' : 'table',
          label: `Open ${t.schema && t.schema !== 'public' ? `${t.schema}.` : ''}${t.name}`,
          run: () => tabs.openTable(s.sessionId, { database: t.database, schema: t.schema ?? null, name: t.name }),
        })
      }
      for (const q of saved.list) {
        out.push({
          group: 'Saved queries',
          icon: 'star',
          label: q.name,
          run: () => tabs.newQuery(s.sessionId, q.database && s.databases.includes(q.database) ? q.database : database, q.sql),
        })
      }
    }
    for (const s of sessions.list) {
      if (s.sessionId === session?.sessionId) continue
      out.push({ group: 'Connections', icon: 'database', label: `Switch to ${s.config.name}`, run: () => sessions.activate(s.sessionId) })
    }
    return out
  })

  // With no query: quick actions plus a few tables. With a query: fuzzy over everything.
  let results = $derived.by(() => {
    if (!query.trim()) {
      const quick = commands.filter((c) => c.group === 'Quick actions')
      const tables = commands.filter((c) => c.group === 'Tables').slice(0, 5)
      return [...quick, ...tables]
    }
    return fuzzyFilter(commands, query, (c) => c.label, 40)
  })

  $effect(() => {
    void query
    index = 0
  })

  $effect(() => {
    listEl?.querySelector(`[data-i="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  function run(c: Command) {
    onClose()
    c.run()
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      index = Math.min(results.length - 1, index + 1)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      index = Math.max(0, index - 1)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      if (results[index]) run(results[index])
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      onClose()
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay" onclick={onClose}>
  <div
    class="palette"
    role="dialog"
    aria-label="Command palette"
    tabindex="-1"
    use:dialogFocus
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => {
      // Escape from anywhere in the palette (not only the input) closes it.
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
      }
    }}
  >
    <div class="input">
      <Icon name="search" size={18} />
      <!-- svelte-ignore a11y_autofocus -->
      <input type="text" placeholder="What would you like to do?" bind:value={query} onkeydown={onKeydown} autofocus spellcheck="false" />
      <kbd>ESC</kbd>
    </div>
    <div class="list" bind:this={listEl}>
      {#each results as c, i (c.group + c.label + i)}
        {#if i === 0 || results[i - 1].group !== c.group}
          <div class="group">{c.group}</div>
        {/if}
        <!-- svelte-ignore a11y_mouse_events_have_key_events -->
        <div class="opt" class:on={i === index} data-i={i} role="option" aria-selected={i === index} tabindex="-1" onclick={() => run(c)} onmousemove={() => (index = i)}>
          <Icon name={c.icon} size={14} />
          <span>{c.label}</span>
          <small>{c.hint ?? '↵'}</small>
        </div>
      {:else}
        <div class="none">No matching actions</div>
      {/each}
    </div>
    <div class="foot">↑ ↓ to navigate · ↵ to select · Esc to close</div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: min(21vh, 155px);
    background: rgb(3 6 11 / 0.6);
    backdrop-filter: blur(5px);
  }

  .palette {
    width: min(540px, calc(100% - 30px));
    overflow: hidden;
    background: var(--bg-elev);
    border: 1px solid var(--border-input);
    border-radius: 14px;
    box-shadow: var(--shadow-lg);
  }

  .input {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 19px;
    border-bottom: 1px solid var(--border-strong);
    color: var(--accent);
  }

  :global(:root[data-theme='light']) .input {
    color: color-mix(in srgb, var(--accent) 60%, #2d3a12);
  }

  .input input {
    flex: 1;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 13px;
  }

  .input input:focus {
    box-shadow: none;
  }

  .list {
    max-height: min(52vh, 440px);
    overflow-y: auto;
    padding-bottom: 6px;
  }

  .group {
    padding: 14px 18px 8px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.11em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 2px 7px;
    padding: 11px 16px;
    border-radius: 7px;
    font-size: 12px;
    color: var(--fg-cell);
  }

  .opt :global(svg) {
    color: var(--fg-muted);
  }

  .opt.on {
    background: var(--bg-active);
    color: var(--fg);
  }

  .opt small {
    margin-left: auto;
    color: var(--fg-faint);
  }

  .none {
    padding: 30px;
    text-align: center;
    color: var(--fg-muted);
  }

  .foot {
    padding: 12px 18px;
    border-top: 1px solid var(--border-strong);
    font-size: 10px;
    color: var(--fg-faint);
  }
</style>
