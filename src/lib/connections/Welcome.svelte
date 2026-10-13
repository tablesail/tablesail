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
  <TopBar session={null} />

  <div class="scroll">
    <div class="inner">
      <section class="hero">
        <span class="mark"><Icon name="pulse" size={24} strokeWidth={1.5} /></span>
        <h1>Your databases, one calm place.</h1>
        <p>
          Browse millions of rows without waiting, edit safely with a SQL preview, and keep production
          clearly marked. PostgreSQL, MySQL, MariaDB, SQLite and MongoDB.
        </p>
        <div class="hero-actions">
          <button class="primary big" onclick={() => (form = { initial: null })}><Icon name="plus" size={14} strokeWidth={1.8} /> New connection</button>
          <button class="outline big" onclick={() => (form = { initial: { ...newConfig('sqlite'), name: '' } })}><Icon name="database" size={14} /> Open a SQLite file</button>
        </div>
      </section>

      <div class="listhead">
        <span class="overline">Connections <span class="n">{connections.list.length}</span></span>
        <label class="search">
          <Icon name="search" size={13} />
          <input type="search" placeholder="Search connections…" bind:value={search} bind:this={searchEl} spellcheck="false" />
        </label>
      </div>

      {#if connections.error}
        <div class="empty error-text">Could not load connections: {connections.error}</div>
      {:else if !connections.loaded}
        <div class="empty">Loading…</div>
      {:else if connections.list.length === 0}
        <div class="empty">No connections yet. Create one to get started.</div>
      {:else if flat.length === 0}
        <div class="empty">No connections match “{search}”.</div>
      {/if}

      <div class="cards">
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
            tabindex="-1"
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
                aria-label="Actions"
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
                <button class="open" onclick={(e) => { e.stopPropagation(); openConnection(c) }}>Open</button>
              {/if}
            </div>
            {#if openError?.id === c.id}
              <div class="err error-text">{openError.message}</div>
            {/if}
          </div>
        {/each}
      </div>

      <p class="hint">Double-click a card to connect · <kbd>↵</kbd> opens the selected one · <kbd>{modLabel}</kbd><kbd>N</kbd> new connection</p>
    </div>
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

  .hero {
    padding: 30px;
    border-radius: 12px;
    border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border-strong));
    background: linear-gradient(135deg, var(--bg-input), color-mix(in srgb, var(--success) 12%, var(--bg-input)));
  }

  .mark {
    display: grid;
    place-items: center;
    width: 46px;
    height: 46px;
    margin-bottom: 20px;
    border-radius: 12px;
    background: var(--accent);
    color: var(--accent-fg);
    box-shadow: 0 0 22px color-mix(in srgb, var(--accent) 18%, transparent);
  }

  h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 760;
    letter-spacing: -0.05em;
  }

  .hero p {
    max-width: 560px;
    margin: 12px 0 22px;
    line-height: 1.8;
    color: var(--fg-muted);
  }

  .hero-actions {
    display: flex;
    gap: 9px;
  }

  .big {
    min-height: 34px;
    padding: 0 14px;
    gap: 7px;
  }

  .listhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 34px 0 14px;
  }

  .n {
    margin-left: 6px;
    color: var(--fg-muted);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 260px;
    height: 31px;
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
