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
  import { modLabel } from '../util/format'

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

  onMount(() => {
    if (!connections.loaded) void connections.load()
    searchEl?.focus()
  })

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
  <div class="panel">
    <header>
      <div class="brand">
        <span class="logo"><Icon name="database" size={18} strokeWidth={1.6} /></span>
        <div>
          <h1>TableSoft</h1>
          <span class="muted">Free, open-source database client</span>
        </div>
      </div>
      <div class="actions">
        <div class="search">
          <span class="search-icon"><Icon name="search" size={12} /></span>
          <input
            type="search"
            placeholder="Search"
            bind:value={search}
            bind:this={searchEl}
            spellcheck="false"
          />
        </div>
        <button class="primary" onclick={() => (form = { initial: null })}><Icon name="plus" size={13} strokeWidth={1.8} /> New</button>
        <button class="icon" title="Keyboard shortcuts ({modLabel}/)" aria-label="Keyboard shortcuts" onclick={() => (ui.shortcutsHelp = true)}>?</button>
      </div>
    </header>

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

      {#each groups as group (group.env)}
        <div class="group-head" style:--env="var(--env-{group.env})">
          <span class="dot"></span>
          {group.env}
          <span class="count">{group.items.length}</span>
        </div>
        {#each group.items as c (c.id)}
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
            <span class="tile">
              <EngineBadge engine={c.engine} size={32} />
              <span class="env-dot" style:background={envColor(c)}></span>
            </span>
            <div class="main">
              <div class="name">
                {c.name}
                {#if c.readOnly}<span class="tag"><Icon name="lock" size={9} /> read-only</span>{/if}
                {#if live}<span class="tag live">connected</span>{/if}
              </div>
              <div class="sub">{ENGINE_LABEL[c.engine]} · {describeConnection(c)}</div>
              {#if openError?.id === c.id}
                <div class="err error-text">{openError.message}</div>
              {/if}
            </div>
            {#if opening === c.id}<span class="muted connecting">Connecting…</span>{/if}
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
      {/each}
    </div>

    <footer class="muted">
      <span><kbd>↑</kbd> <kbd>↓</kbd> select · <kbd>Enter</kbd> open · <kbd>{modLabel}</kbd> <kbd>N</kbd> new</span>
    </footer>
  </div>
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
    display: flex;
    justify-content: center;
    background: var(--bg-sidebar);
    padding: 8vh 16px 16px;
  }

  .panel {
    width: 720px;
    max-width: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    max-height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 16px 16px 14px 18px;
    border-bottom: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 11px;
  }

  .logo {
    width: 34px;
    height: 34px;
    border-radius: 9px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background: linear-gradient(145deg, #3d8bfd, #0a58ca);
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.25);
  }

  h1 {
    font-size: 15px;
    font-weight: 700;
    margin: 0;
    line-height: 1.2;
  }

  .brand .muted {
    font-size: 12px;
  }

  .actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .search {
    position: relative;
  }

  .search-icon {
    position: absolute;
    left: 9px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--fg-faint);
    pointer-events: none;
  }

  .search input {
    width: 200px;
    padding-left: 26px;
    background: var(--bg-active);
    border-color: transparent;
  }

  .search input:focus {
    background: var(--bg-input);
  }

  .list {
    flex: 1;
    overflow: auto;
    padding: 4px 10px 12px;
    min-height: 120px;
  }

  .empty {
    padding: 40px 16px;
    text-align: center;
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 14px 10px 6px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--env);
  }

  .count {
    color: var(--fg-faint);
    font-weight: 400;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 10px;
    border-radius: 8px;
  }

  .item:hover {
    background: var(--bg-hover);
  }

  .item.selected {
    background: var(--selection);
  }

  .tile {
    position: relative;
    display: inline-flex;
  }

  .env-dot {
    position: absolute;
    right: -3px;
    bottom: -3px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 2px solid var(--bg);
  }

  .item:hover .env-dot,
  .item.selected .env-dot {
    border-color: transparent;
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 600;
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .sub {
    margin-top: 1px;
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

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 16px;
    font-size: 10px;
    font-weight: 500;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--bg-active);
    color: var(--fg-muted);
  }

  .tag.live {
    color: var(--success);
    background: color-mix(in srgb, var(--success) 14%, transparent);
  }

  .connecting {
    font-size: 12px;
  }

  .more {
    opacity: 0;
  }

  .item:hover .more,
  .item.selected .more {
    opacity: 1;
  }

  footer {
    padding: 9px 18px;
    border-top: 1px solid var(--border);
    background: var(--bg-chrome);
    font-size: 11px;
  }
</style>
