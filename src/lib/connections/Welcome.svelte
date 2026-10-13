<!-- Connection list: grouped by environment, searchable, Enter / double-click to open. -->
<script lang="ts">
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
        <h1>TableSoft</h1>
        <span class="muted">Connections</span>
      </div>
      <div class="actions">
        <input
          type="search"
          placeholder="Search connections"
          bind:value={search}
          bind:this={searchEl}
          spellcheck="false"
        />
        <button class="primary" onclick={() => (form = { initial: null })}>New connection</button>
        <button class="ghost" title="Keyboard shortcuts ({modLabel}+/)" onclick={() => (ui.shortcutsHelp = true)}>?</button>
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
        <div class="group-head">
          <span class="dot" style:background="var(--env-{group.env})"></span>
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
            <span class="bar" style:background={envColor(c)}></span>
            <div class="main">
              <div class="name">
                {c.name}
                {#if c.readOnly}<span class="tag">read-only</span>{/if}
                {#if live}<span class="tag live">connected</span>{/if}
              </div>
              <div class="sub">{ENGINE_LABEL[c.engine]} · {describeConnection(c)}</div>
              {#if openError?.id === c.id}
                <div class="err error-text">{openError.message}</div>
              {/if}
            </div>
            {#if opening === c.id}<span class="muted">Connecting…</span>{/if}
            <button
              class="ghost more"
              aria-label="Actions"
              onclick={(e) => {
                e.stopPropagation()
                showMenu(e, c)
              }}>⋯</button
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
    padding: 7vh 16px 16px;
  }

  .panel {
    width: 680px;
    max-width: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    max-height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  h1 {
    font-size: 17px;
    margin: 0;
  }

  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .actions input {
    width: 190px;
  }

  .list {
    flex: 1;
    overflow: auto;
    padding: 6px 8px 10px;
    min-height: 120px;
  }

  .empty {
    padding: 36px 16px;
    text-align: center;
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 8px 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .count {
    opacity: 0.6;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px 7px 0;
    border-radius: 6px;
    overflow: hidden;
  }

  .item:hover {
    background: var(--bg-hover);
  }

  .item.selected {
    background: var(--selection);
  }

  .bar {
    align-self: stretch;
    width: 4px;
    border-radius: 0 3px 3px 0;
    flex: none;
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
    font-size: 10px;
    font-weight: 500;
    padding: 0 6px;
    border-radius: 8px;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }

  .tag.live {
    color: var(--success);
    border-color: var(--success);
  }

  .more {
    padding: 0 8px;
  }

  footer {
    padding: 8px 16px;
    border-top: 1px solid var(--border);
    font-size: 11px;
  }
</style>
