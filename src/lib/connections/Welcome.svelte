<!-- Connection list: grouped by environment, searchable, Enter / double-click to open. -->
<script lang="ts">
  import EngineBadge from '../ui/EngineBadge.svelte'
  import TopBar from '../shell/TopBar.svelte'
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
  import { disconnect } from '../editor/transaction'
  import type { ConnectionConfig, EnvTag, Secrets } from '../types'
  import { connectionUrl, looksLikeUrl, parseConnectionUrl } from '../util/connUrl'
  import { ENGINE_LABEL, ENV_TAGS, describeConnection, envColor } from '../util/env'
  import { isMac, modLabel } from '../util/format'
  import { api } from '../api'
  import { newConfig } from '../util/env'

  let search = $state('')
  let selectedId = $state<string | null>(null)
  let form = $state<{ initial: ConnectionConfig | null; fromUrl?: string } | null>(null)
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

  /** The connection as a URL, without its password. */
  async function copyUrl(config: ConnectionConfig) {
    const url = connectionUrl(config)
    if (!url) return
    try {
      await navigator.clipboard.writeText(url)
      ui.toast('Connection URL copied (the password is not included)')
    } catch {
      ui.toast('Could not copy to the clipboard', 'error')
    }
  }

  /** A connection URL pasted into the search box opens a new connection filled from it. */
  function onSearchPaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') ?? ''
    // Only real URLs: a pasted file name is a search for a connection of that name.
    if (!looksLikeUrl(text) || !parseConnectionUrl(text).ok) return
    e.preventDefault()
    form = { initial: null, fromUrl: text }
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
      ...(live ? [{ label: 'Disconnect', onclick: () => disconnect(live.sessionId) }] : []),
      { separator: true },
      { label: 'Edit…', onclick: () => (form = { initial: c }) },
      { label: 'Duplicate', onclick: () => duplicate(c) },
      { label: 'Copy as URL', disabled: !connectionUrl(c), onclick: () => copyUrl(c) },
      { separator: true },
      { label: 'Delete…', danger: true, onclick: () => (confirmDelete = c) },
    ]
  }

  function onKeydown(e: KeyboardEvent) {
    // Keys already handled elsewhere (palette, dialogs, text fields) are not ours.
    if (form || prompt || confirmDelete || menu || ui.palette || e.defaultPrevented) return
    const t = e.target as HTMLElement | null
    if (t !== searchEl && t?.closest('input, textarea, select, [role="dialog"]')) return
    const i = flat.findIndex((c) => c.id === selectedId)
    // With the focus on a card, the focus follows the selection; from the search box it stays in the box.
    const onCard = !!t?.closest('.card')
    const select = (id: string | undefined) => {
      if (!id) return
      selectedId = id
      if (onCard) document.querySelector<HTMLElement>(`[data-conn="${CSS.escape(id)}"]`)?.focus()
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      select(flat[Math.min(flat.length - 1, i + 1)]?.id)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      select(flat[Math.max(0, i - 1)]?.id)
    } else if (e.key === 'Home' && onCard) {
      e.preventDefault()
      select(flat[0]?.id)
    } else if (e.key === 'End' && onCard) {
      e.preventDefault()
      select(flat[flat.length - 1]?.id)
    } else if (e.key === 'Enter' && t?.closest('button')) {
      // Enter on a card's own buttons (Open, Actions) belongs to the button.
    } else if (e.key === 'Enter' && i >= 0) {
      e.preventDefault()
      // A held Enter (e.g. the one that just ran a palette command) must not open a connection.
      if (e.repeat) return
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
  <TopBar session={null} />

  <div class="scroll">
    <div class="inner">
      <div class="listhead">
        <h1>Connections <span class="n">{connections.list.length}</span></h1>
        <span class="grow"></span>
        <label class="search">
          <Icon name="search" size={13} />
          <input type="search" placeholder="Search connections…" bind:value={search} bind:this={searchEl} spellcheck="false" onpaste={onSearchPaste} />
        </label>
        <button class="outline" onclick={() => (form = { initial: { ...newConfig('sqlite'), name: '' } })}><Icon name="database" size={13} /> Open SQLite file</button>
        <button class="primary" onclick={() => (form = { initial: null })}><Icon name="plus" size={13} strokeWidth={1.8} /> New connection</button>
      </div>

      {#if connections.error}
        <div class="empty error-text" role="alert">Could not load connections: {connections.error}</div>
      {:else if !connections.loaded}
        <div class="empty">Loading…</div>
      {:else if connections.list.length === 0}
        <div class="empty">No connections yet. Create one to get started.</div>
      {:else if flat.length === 0}
        <div class="empty">No connections match “{search}”.</div>
      {/if}

      <div class="cards" role="listbox" aria-label="Saved connections">
        {#each flat as c (c.id)}
          {@const live = sessions.byConnection(c.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <div
            class="card"
            class:selected={c.id === selectedId}
            style:--env={envColor(c)}
            data-conn={c.id}
            role="option"
            aria-selected={c.id === selectedId}
            tabindex={c.id === selectedId ? 0 : -1}
            onclick={() => (selectedId = c.id)}
            ondblclick={() => openConnection(c)}
            oncontextmenu={(e) => showMenu(e, c)}
          >
            <div class="top">
              <EngineBadge engine={c.engine} size={34} />
              <div class="main">
                <div class="name">{c.name}</div>
                <div class="sub">{ENGINE_LABEL[c.engine]}</div>
              </div>
              <button
                class="icon more"
                aria-label="Actions for {c.name}"
                onclick={(e) => {
                  e.stopPropagation()
                  showMenu(e, c)
                }}><Icon name="more" size={16} /></button
              >
            </div>
            <div class="addr">{describeConnection(c) || '—'}</div>
            <div class="bottom">
              <span class="tag">{envLabel[c.env]}</span>
              {#if c.readOnly}<span class="tag ro"><Icon name="lock" size={10} /> read-only</span>{/if}
              <span class="grow"></span>
              {#if opening === c.id}
                <span class="state">Connecting…</span>
              {:else if live}
                <span class="state on"><i class="live-dot"></i> Connected</span>
              {:else}
                <button class="open" aria-label="Open {c.name}" onclick={(e) => { e.stopPropagation(); openConnection(c) }}>Open</button>
              {/if}
            </div>
            {#if openError?.id === c.id}
              <div class="err error-text" role="alert">{openError.message}</div>
            {/if}
          </div>
        {/each}
      </div>

      <p class="hint">Double-click a card to connect · <kbd>↵</kbd> opens the selected one · <kbd>{modLabel}</kbd><kbd>N</kbd> new connection</p>
    </div>
  </div>
</div>

{#if form}
  <ConnectionForm initial={form.initial} fromUrl={form.fromUrl} onClose={() => (form = null)} onSaved={(c) => (selectedId = c.id)} />
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
    flex-direction: column;
    background: var(--bg-chrome);
  }

  .scroll {
    flex: 1;
    overflow: auto;
  }

  .inner {
    max-width: 1080px;
    margin: 0 auto;
    padding: 30px 30px 40px;
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 740;
    letter-spacing: -0.03em;
  }

  .listhead {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0 16px;
  }

  .n {
    margin-left: 4px;
    font-weight: 500;
    color: var(--fg-faint);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 240px;
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--border-strong);
    border-radius: 7px;
    background: var(--bg-input);
    color: var(--fg-muted);
  }

  .search:focus-within {
    border-color: var(--accent);
  }

  .search input {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
  }

  .search input:focus {
    box-shadow: none;
  }

  .empty {
    padding: 40px 16px;
    text-align: center;
    color: var(--fg-muted);
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    border-radius: 11px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    transition: border-color 0.12s;
  }

  .card:hover {
    border-color: var(--border-input);
  }

  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .card:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--accent);
  }

  .top {
    display: flex;
    align-items: center;
    gap: 11px;
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: -0.012em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sub {
    margin-top: 2px;
    font-size: 10px;
    color: var(--fg-muted);
  }

  .more {
    opacity: 0;
  }

  .card:hover .more,
  .card.selected .more {
    opacity: 1;
  }

  .addr {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--mono-fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bottom {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 20px;
    padding: 0 7px;
    border-radius: 5px;
    font-size: 10px;
    font-weight: 640;
    color: var(--env);
    background: color-mix(in srgb, var(--env) 15%, transparent);
  }

  .tag::before {
    content: '';
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }

  .tag.ro {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 15%, transparent);
  }

  .tag.ro::before {
    display: none;
  }

  .grow {
    flex: 1;
  }

  .state {
    font-size: 10px;
    color: var(--fg-muted);
  }

  .state.on {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--success);
  }

  .open {
    min-height: 24px;
    padding: 0 10px;
    font-size: 11px;
  }

  .card:hover .open,
  .card.selected .open {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg);
    font-weight: 700;
  }

  .err {
    white-space: pre-wrap;
    font-size: 11px;
  }

  .hint {
    margin: 22px 0 0;
    text-align: center;
    font-size: 11px;
    color: var(--fg-faint);
  }

  .hint kbd + kbd {
    margin-left: 2px;
  }
</style>
