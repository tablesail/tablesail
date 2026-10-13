<!--
  Assistant: a chat that writes SQL for the current connection (Anthropic Messages API,
  called from the Rust side). Fills its parent (flex column, min-height: 0).
  <AssistantPanel {session} />
  Model text is rendered through chatMarkdown's plain data, never as HTML.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { assistant, buildSchemaContext, firstTableName } from '../stores/assistant.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import { ui } from '../stores/ui.svelte'
  import type { SessionInfo } from '../types'
  import { parseMarkdown, type Inline } from '../util/chatMarkdown'
  import { highlightSql } from '../util/sqlHighlight'
  import Icon from '../ui/Icon.svelte'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  const sessionId = $derived(session.sessionId)
  const database = $derived(sessions.database(sessionId))
  const snapshot = $derived(sessions.schemaEntry(sessionId, database).snapshot)
  const chat = $derived(assistant.chat(sessionId))
  const draft = $derived(assistant.draft(sessionId))
  const busy = $derived(chat.requestId !== null)

  const suggestions = $derived.by(() => {
    const table = firstTableName(snapshot)
    const docs = session.config.engine === 'mongodb'
    const rows = docs ? 'documents' : 'rows'
    if (!table) {
      return [
        docs ? 'What collections are in this database?' : 'What tables are in this database?',
        `Write a query to count ${rows} per day`,
        'How do these tables relate to each other?',
      ]
    }
    return [
      `Show the 10 most recent ${rows} of ${table}`,
      docs ? `Which collections reference ${table}?` : `Which tables reference ${table}?`,
      `Write a query to count ${rows} per day`,
    ]
  })

  let scroller: HTMLDivElement | undefined = $state()
  let input: HTMLTextAreaElement | undefined = $state()

  onMount(() => {
    if (assistant.hasKey === null) void assistant.refreshKey()
  })

  // Keep the newest thing in view: a reply starts at its first line (so a long answer isn't
  // read from the bottom), everything else (question, thinking, error) sticks to the bottom.
  $effect(() => {
    const last = chat.messages.at(-1)
    void chat.requestId
    void chat.error
    void sessionId
    void tick().then(() => {
      if (!scroller) return
      const reply = scroller.querySelector<HTMLElement>('[data-latest-reply]')
      if (last?.role === 'assistant' && !busy && !chat.error && reply) scroller.scrollTop = reply.offsetTop - 8
      else scroller.scrollTop = scroller.scrollHeight
    })
  })

  const MAX_INPUT_PX = 6 * 18 + 12

  function autosize() {
    if (!input) return
    input.style.height = 'auto'
    input.style.height = `${Math.min(input.scrollHeight, MAX_INPUT_PX)}px`
  }

  // Re-measure when the draft changes from outside typing (sent, or another connection's draft).
  $effect(() => {
    void draft
    void sessionId
    void tick().then(autosize)
  })

  function ask(text: string) {
    assistant.send(session, database, buildSchemaContext(snapshot), text)
  }

  function submit() {
    if (!busy) ask(draft)
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault()
      submit()
    }
  }

  function openInEditor(sql: string, run = false) {
    tabs.newQuery(sessionId, database, sql, { run })
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      ui.toast('Copied', 'success', 1200)
    } catch {
      ui.toast('Could not copy to the clipboard', 'error')
    }
  }

  function newChat() {
    assistant.clear(sessionId)
    void tick().then(() => input?.focus())
  }
</script>

{#snippet inline(parts: Inline[])}
  {#each parts as part, p (p)}
    {#if part.kind === 'code'}<code>{part.text}</code>{:else if part.kind === 'bold'}<strong>{part.text}</strong>{:else}{part.text}{/if}
  {/each}
{/snippet}

<div class="assistant">
  {#if assistant.hasKey === false}
    <div class="state">
      <span class="mark"><Icon name="sparkle" size={20} /></span>
      <b>Assistant</b>
      <p>Add your Anthropic API key in Settings to use the assistant.</p>
      <button class="primary" onclick={() => (ui.settings = true)}>Open Settings</button>
    </div>
  {:else}
    <div class="messages" bind:this={scroller}>
      {#if chat.messages.length === 0}
        <div class="intro">
          <span class="mark"><Icon name="sparkle" size={20} /></span>
          <b>Ask about your data</b>
          <p>Describe what you need and get SQL for this connection.</p>
          <div class="chips">
            {#each suggestions as s (s)}
              <button class="chip" onclick={() => ask(s)}>{s}</button>
            {/each}
          </div>
        </div>
      {/if}

      {#each chat.messages as m, i (m.id)}
        {#if m.role === 'user'}
          <div class="user"><div class="bubble">{m.content}</div></div>
        {:else}
          <div class="reply" data-latest-reply={i === chat.messages.length - 1 ? '' : undefined}>
            {#each parseMarkdown(m.content) as block, b (b)}
              {#if block.kind === 'paragraph'}
                <p>{@render inline(block.inlines)}</p>
              {:else if block.kind === 'list'}
                <svelte:element this={block.ordered ? 'ol' : 'ul'}>
                  {#each block.items as item, k (k)}
                    <li>{@render inline(item)}</li>
                  {/each}
                </svelte:element>
              {:else}
                <div class="codebox">
                  <pre><code class="sql">{@html highlightSql(block.text)}</code></pre>
                  <div class="codeactions">
                    <button class="mini run" title="Open in a new query tab and run it" onclick={() => openInEditor(block.text, true)}><Icon name="play" size={10} /> Run</button>
                    <button class="mini" onclick={() => openInEditor(block.text)}>Open in editor</button>
                    <button class="mini" onclick={() => copy(block.text)}>Copy</button>
                  </div>
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      {/each}

      {#if busy}
        <div class="thinking" role="status">
          <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
          Thinking…
          <button class="mini" onclick={() => assistant.cancel(sessionId)}>Stop</button>
        </div>
      {/if}

      {#if chat.error}
        <div class="error" role="alert">
          <span>{chat.error}</span>
          {#if chat.messages.at(-1)?.role === 'user' && !busy}
            <button class="mini" onclick={() => assistant.retry(session, database, buildSchemaContext(snapshot))}>
              Try again
            </button>
          {/if}
        </div>
      {/if}
    </div>

    <div class="composer">
      <div class="field">
        <textarea
          bind:this={input}
          rows="1"
          placeholder="Ask about your data…"
          aria-label="Message the assistant"
          value={draft}
          oninput={(e) => {
            assistant.setDraft(sessionId, e.currentTarget.value)
            autosize()
          }}
          onkeydown={onKeydown}
        ></textarea>
        <button
          class="send"
          title="Send (Enter)"
          aria-label="Send"
          disabled={busy || !draft.trim()}
          onclick={submit}
        >
          <Icon name="send" size={13} strokeWidth={1.8} />
        </button>
      </div>
      <div class="foot">
        <button class="link" disabled={chat.messages.length === 0 && !busy} onclick={newChat}>New chat</button>
        <span class="model">{assistant.model}</span>
      </div>
    </div>
  {/if}
</div>

<style>
  .assistant {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--bg);
    color: var(--fg);
    font-size: 13px;
  }

  .state,
  .intro {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 6px;
    padding: 28px 8px 8px;
  }

  .state {
    flex: 1;
    justify-content: center;
    padding: 24px 20px;
    gap: 8px;
  }

  .state p,
  .intro p {
    margin: 0;
    max-width: 230px;
    color: var(--fg-muted);
    font-size: 12px;
    line-height: 1.45;
  }

  .state button {
    margin-top: 6px;
  }

  .mark {
    display: inline-flex;
    color: var(--fg-faint);
  }

  .chips {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    width: 100%;
    margin-top: 10px;
  }

  .chip {
    justify-content: flex-start;
    height: auto;
    padding: 6px 10px;
    text-align: left;
    white-space: normal;
    line-height: 1.35;
    font-size: 12px;
    color: var(--fg);
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: none;
  }

  .chip:hover:not(:disabled) {
    background: var(--bg-active);
  }

  .messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 12px 6px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }

  .user {
    display: flex;
    justify-content: flex-end;
  }

  .bubble {
    max-width: 88%;
    padding: 6px 11px;
    border-radius: 10px;
    background: var(--bg-active);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .reply {
    display: flex;
    flex-direction: column;
    gap: 8px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .reply p {
    margin: 0;
    white-space: pre-wrap;
  }

  .reply ul,
  .reply ol {
    margin: 0;
    padding-left: 18px;
  }

  .reply li + li {
    margin-top: 2px;
  }

  .reply code {
    font-family: var(--mono);
    font-size: 12px;
    padding: 0 4px;
    border-radius: 4px;
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
  }

  .codebox {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }

  .codebox pre {
    margin: 0;
    padding: 8px 10px;
    overflow-x: auto;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    color: var(--fg-cell);
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .codebox pre code {
    padding: 0;
    border: none;
    background: none;
    font-size: inherit;
    white-space: pre;
  }

  .codeactions {
    display: flex;
    gap: 6px;
  }

  button.mini {
    min-height: 22px;
    padding: 0 8px;
    font-size: 11px;
    border-radius: 5px;
  }

  button.mini.run {
    background: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 70%, #000);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .sql :global(.k) {
    color: var(--syn-keyword);
  }

  .sql :global(.s) {
    color: var(--syn-string);
  }

  .sql :global(.n) {
    color: var(--syn-number);
  }

  .sql :global(.c) {
    color: var(--syn-comment);
  }

  .thinking {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-muted);
  }

  .dots {
    display: inline-flex;
    gap: 3px;
  }

  .dots i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--fg-faint);
    animation: pulse 1.1s ease-in-out infinite;
  }

  .dots i:nth-child(2) {
    animation-delay: 0.15s;
  }

  .dots i:nth-child(3) {
    animation-delay: 0.3s;
  }

  @keyframes pulse {
    0%,
    80%,
    100% {
      opacity: 0.25;
    }
    40% {
      opacity: 1;
    }
  }

  .error {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
    color: var(--danger);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .composer {
    flex: none;
    padding: 8px 12px 6px;
    border-top: 1px solid var(--border);
  }

  .field {
    position: relative;
  }

  textarea {
    display: block;
    width: 100%;
    resize: none;
    overflow-y: auto;
    padding: 6px 34px 6px 10px;
    min-height: 30px;
    max-height: 120px;
    line-height: 18px;
    border-radius: 8px;
    font-size: 13px;
  }

  button.send {
    position: absolute;
    right: 4px;
    bottom: 4px;
    width: 22px;
    min-width: 22px;
    min-height: 22px;
    padding: 0;
    border-radius: 6px;
    background: var(--accent);
    border-color: transparent;
    color: var(--accent-fg);
    box-shadow: none;
  }

  button.send:hover:not(:disabled) {
    background: var(--accent);
    filter: brightness(0.95);
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 4px;
    font-size: 11px;
    color: var(--fg-faint);
  }

  button.link {
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    box-shadow: none;
    font-size: 11px;
    color: var(--fg-muted);
  }

  button.link:hover:not(:disabled) {
    background: none;
    color: var(--fg);
    text-decoration: underline;
  }

  .model {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
