<!-- Connection list: grouped by environment, searchable, Enter / double-click to open. -->
<script lang="ts">
  import EngineBadge from '../ui/EngineBadge.svelte'
  import Icon from '../ui/Icon.svelte'
  import { onMount } from 'svelte'
  import Modal from '../ui/Modal.svelte'
  import ContextMenu, { type MenuItem } from '../shell/ContextMenu.svelte'
  import ConnectionForm from './ConnectionForm.svelte'
  import PasswordPrompt from './PasswordPrompt.svelte'
  import { errorMessage } from '../api'
  import { connections } from '../stores/connections.svelte'
  import { isAuthError, sessions } from '../stores/sessions.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { ConnectionConfig, EnvTag, Secrets } from '../types'
  import { ENGINE_LABEL, ENV_TAGS, describeConnection, envColor } from '../util/env'
  import { isMac, modLabel } from '../util/format'
  import { api } from '../api'
  import { newConfig } from '../util/env'

  let search = $state('')
  let selectedId = $state<string | null>(null)
  let form = $state<{ initial: ConnectionConfig | null } | null>(null)
  let menu = $state<{ x: number; y: number; config: ConnectionConfig } | null>(null)
  let confirmDelete = $state<ConnectionConfig | null>(null)
  let opening = $state<string | null>(null)
  let openError = $state<{ id: string; message: string } | null>(null)
  let prompt = $state<{ config: ConnectionConfig; error: string; busy: boolean } | null>(null)
  let searchEl = $state<HTMLInputElement>()

  const groupOrder: EnvTag[] = [...ENV_TAGS].reverse()

  let groups = $derived.by(() => {
    const q = search.trim().toLowerCase()
    const match = (c: ConnectionConfig) =>
      !q ||
      c.name.toLowerCase().includes(q) ||
      c.host.toLowerCase().includes(q) ||
      c.database.toLowerCase().includes(q) ||
      ENGINE_LABEL[c.engine].toLowerCase().includes(q)
    return groupOrder
      .map((env) => ({
        env,
        items: connections.list
          .filter((c) => c.env === env && match(c))
          .sort((a, b) => a.name.localeCompare(b.name)),
      }))
      .filter((g) => g.items.length)
  })

  let flat = $derived(groups.flatMap((g) => g.items))

  let version = $state('')

  onMount(() => {
    if (!connections.loaded) void connections.load()
    searchEl?.focus()
    api.appInfo().then((i) => (version = i.version), () => {})
  })

  const envLabel: Record<EnvTag, string> = { local: 'local', development: 'dev', staging: 'staging', production: 'production' }

  $effect(() => {
    if (!flat.some((c) => c.id === selectedId)) selectedId = flat[0]?.id ?? null
  })

  async function openConnection(config: ConnectionConfig, secrets: Secrets | null = null, remember = false) {
    const existing = sessions.byConnection(config.id)
    if (existing) {
      sessions.activate(existing.sessionId)
      return
    }
    opening = config.id
    openError = null
    try {
      await sessions.open(config.id, secrets)
      if (secrets && remember) {
        try {
          await connections.save(config, secrets)
        } catch (e) {
          ui.toast(`Connected, but saving the password failed: ${errorMessage(e)}`, 'error')
        }
      }
      prompt = null
    } catch (e) {
      const message = errorMessage(e)
      if (isAuthError(e) && config.engine !== 'sqlite') {
        prompt = { config, error: secrets ? message : '', busy: false }
      } else {
        prompt = null
        openError = { id: config.id, message }
      }
    } finally {
      opening = null
    }
  }

  async function submitPrompt(secrets: Secrets, remember: boolean) {
    if (!prompt) return
    prompt.busy = true
    const config = prompt.config
    await openConnection(config, secrets, remember)
    if (prompt) prompt.busy = false
  }

  async function doDelete(config: ConnectionConfig) {
    confirmDelete = null
    try {
      const session = sessions.byConnection(config.id)
      if (session) await sessions.close(session.sessionId)
      await connections.remove(config.id)
    } catch (e) {
      ui.toast(`Delete failed: ${errorMessage(e)}`, 'error')
    }
  }

  async function duplicate(config: ConnectionConfig) {
    try {
      const copy = await connections.duplicate(config)
      selectedId = copy.id
    } catch (e) {
      ui.toast(`Duplicate failed: ${errorMessage(e)}`, 'error')
    }
  }

  function showMenu(e: MouseEvent, config: ConnectionConfig) {
    e.preventDefault()
    selectedId = config.id
    menu = { x: e.clientX, y: e.clientY, config }
  }

  function menuItems(c: ConnectionConfig): MenuItem[] {
    const live = sessions.byConnection(c.id)
    return [
      { label: live ? 'Switch to' : 'Open', onclick: () => openConnection(c) },
      ...(live ? [{ label: 'Disconnect', onclick: () => sessions.close(live.sessionId) }] : []),
      { separator: true },
      { label: 'Edit…', onclick: () => (form = { initial: c }) },
      { label: 'Duplicate', onclick: () => duplicate(c) },
      { separator: true },
      { label: 'Delete…', danger: true, onclick: () => (confirmDelete = c) },
    ]
  }

  function onKeydown(e: KeyboardEvent) {
    if (form || prompt || confirmDelete || menu) return
    const i = flat.findIndex((c) => c.id === selectedId)
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      selectedId = flat[Math.min(flat.length - 1, i + 1)]?.id ?? selectedId
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      selectedId = flat[Math.max(0, i - 1)]?.id ?? selectedId
    } else if (e.key === 'Enter' && i >= 0) {
      e.preventDefault()
      void openConnection(flat[i])
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'n') {
      e.preventDefault()
      form = { initial: null }
    }
  }

  $effect(() => {
    if (selectedId) document.querySelector(`[data-conn="${CSS.escape(selectedId)}"]`)?.scrollIntoView({ block: 'nearest' })
  })
</script>

<svelte:window onkeydown={onKeydown} />

<div class="welcome">
  <aside class="intro" data-tauri-drag-region class:mac={isMac}>
    <div class="hero" data-tauri-drag-region>
      <span class="logo"><Icon name="database" size={34} strokeWidth={1.3} /></span>
      <h1>TableSoft</h1>
      <p class="muted">{version ? `Version ${version}` : '\u00a0'}</p>
    </div>
    <div class="actions">
      <button class="action" onclick={() => (form = { initial: null })}>
        <Icon name="plus" size={14} /> Create a new connection…
      </button>
      <button class="action" onclick={() => (form = { initial: { ...newConfig('sqlite'), name: '' } })}>
        <Icon name="database" size={14} /> Open a SQLite file…
      </button>
      <button class="action" onclick={() => (ui.shortcutsHelp = true)}>
        <Icon name="key" size={14} /> Keyboard shortcuts
      </button>
    </div>
    <p class="foot muted">Free and open source · MIT</p>
  </aside>

  <section class="conns">
    <div class="search" data-tauri-drag-region>
      <span class="search-icon"><Icon name="search" size={13} /></span>
      <input
        type="search"
        placeholder="Search for connection…"
        bind:value={search}
        bind:this={searchEl}
        spellcheck="false"
      />
    </div>

    <div class="list">
      {#if connections.error}
        <div class="empty error-text">Could not load connections: {connections.error}</div>
      {:else if !connections.loaded}
        <div class="empty muted">Loading…</div>
      {:else if connections.list.length === 0}
        <div class="empty">
          <p>No connections yet.</p>
          <button class="primary" onclick={() => (form = { initial: null })}>Create your first connection</button>
        </div>
      {:else if flat.length === 0}
        <div class="empty muted">No connections match “{search}”.</div>
      {/if}

      {#each flat as c (c.id)}
        {@const live = sessions.byConnection(c.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
        <div
          class="item"
          class:selected={c.id === selectedId}
          data-conn={c.id}
          role="option"
          aria-selected={c.id === selectedId}
          tabindex="-1"
          onclick={() => (selectedId = c.id)}
          ondblclick={() => openConnection(c)}
          oncontextmenu={(e) => showMenu(e, c)}
        >
          <EngineBadge engine={c.engine} size={36} />
          <div class="main">
            <div class="name">
              <span class="n">{c.name}</span>
              <span class="tag" style:--env={envColor(c)}>{envLabel[c.env]}</span>
              {#if c.readOnly}<span class="lock" title="Read-only"><Icon name="lock" size={11} /></span>{/if}
              {#if live}<span class="live" title="Connected"></span>{/if}
            </div>
            <div class="sub">{describeConnection(c) || ENGINE_LABEL[c.engine]}</div>
            {#if openError?.id === c.id}
              <div class="err error-text">{openError.message}</div>
            {/if}
          </div>
          {#if opening === c.id}<span class="connecting">Connecting…</span>{/if}
          <button
            class="icon more"
            aria-label="Actions"
            onclick={(e) => {
              e.stopPropagation()
              showMenu(e, c)
            }}><Icon name="more" size={16} /></button
          >
        </div>
      {/each}
    </div>

    <footer class="muted">
      <span><kbd>↑</kbd><kbd>↓</kbd> select · <kbd>↵</kbd> open · <kbd>{modLabel}</kbd><kbd>N</kbd> new</span>
    </footer>
  </section>
</div>

{#if form}
  <ConnectionForm initial={form.initial} onClose={() => (form = null)} onSaved={(c) => (selectedId = c.id)} />
{/if}

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.config)} onClose={() => (menu = null)} />
{/if}

{#if prompt}
  <PasswordPrompt
    config={prompt.config}
    error={prompt.error}
    busy={prompt.busy}
    onSubmit={submitPrompt}
    onClose={() => (prompt = null)}
  />
{/if}

{#if confirmDelete}
  {@const target = confirmDelete}
  <Modal title="Delete connection" onClose={() => (confirmDelete = null)} width="400px">
    Delete <b>{target.name}</b>? Its saved password is removed from the keychain too.
    {#snippet footer()}
      <button onclick={() => (confirmDelete = null)}>Cancel</button>
      <button class="danger" onclick={() => doDelete(target)}>Delete</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .welcome {
    height: 100%;
    display: grid;
    grid-template-columns: 280px 1fr;
    background: var(--bg);
  }

  .intro {
    display: flex;
    flex-direction: column;
    padding: 40px 20px 16px;
    background: var(--bg-chrome);
    border-right: 1px solid var(--border-strong);
  }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    margin: 24px 0 28px;
  }

  .logo {
    width: 84px;
    height: 84px;
    border-radius: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background: linear-gradient(160deg, #4f9bff 0%, #1462d6 60%, #0d47a8 100%);
    box-shadow:
      inset 0 1px 0 rgb(255 255 255 / 0.3),
      0 8px 20px rgb(13 71 168 / 0.3);
    margin-bottom: 10px;
  }

  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }

  .hero p {
    margin: 0;
    font-size: 12px;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .action {
    justify-content: flex-start;
    gap: 9px;
    height: 32px;
    padding: 0 12px;
    border-radius: 7px;
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    color: var(--fg);
  }

  .action :global(svg) {
    color: var(--accent);
  }

  .action:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .foot {
    margin: auto 0 0;
    text-align: center;
    font-size: 11px;
  }

  .conns {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .search {
    position: relative;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
  }

  .search-icon {
    position: absolute;
    left: 24px;
    top: 50%;
    transform: translateY(-48%);
    color: var(--fg-faint);
    pointer-events: none;
  }

  .search input {
    width: 100%;
    height: 30px;
    padding-left: 30px;
    border-radius: 7px;
  }

  .list {
    flex: 1;
    overflow: auto;
    padding: 6px 8px;
    min-height: 120px;
  }

  .empty {
    padding: 48px 16px;
    text-align: center;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px;
    border-radius: 8px;
  }

  .item + .item {
    margin-top: 1px;
  }

  .item:hover {
    background: var(--bg-hover);
  }

  .item.selected {
    background: var(--accent-solid);
    color: #fff;
  }

  .item.selected .sub,
  .item.selected .lock,
  .item.selected .more,
  .item.selected .connecting {
    color: inherit;
    opacity: 0.85;
  }

  .item.selected .tag {
    background: rgb(255 255 255 / 0.22);
    color: #fff;
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    display: flex;
    gap: 7px;
    align-items: center;
    font-weight: 600;
  }

  .n {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tag {
    flex: none;
    height: 16px;
    line-height: 16px;
    padding: 0 6px;
    border-radius: 4px;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--env);
    background: color-mix(in srgb, var(--env) 15%, transparent);
  }

  .lock {
    display: inline-flex;
    color: var(--warning);
  }

  .live {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--success) 25%, transparent);
  }

  .sub {
    margin-top: 2px;
    font-size: 12px;
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .err {
    margin-top: 2px;
    white-space: pre-wrap;
  }

  .connecting {
    font-size: 12px;
    color: var(--fg-muted);
  }

  .more {
    opacity: 0;
  }

  .item:hover .more,
  .item.selected .more {
    opacity: 1;
  }

  footer {
    padding: 8px 16px;
    border-top: 1px solid var(--border);
    font-size: 11px;
  }

  footer kbd + kbd {
    margin-left: 2px;
  }
</style>
