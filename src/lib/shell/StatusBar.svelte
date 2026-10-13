<script lang="ts">
  import { sessions } from '../stores/sessions.svelte'
  import { tabs } from '../stores/tabs.svelte'
  import type { SessionInfo } from '../types'
  import { ENGINE_LABEL, describeConnection } from '../util/env'
  import { formatCount } from '../util/format'

  interface Props {
    session: SessionInfo
  }

  let { session }: Props = $props()

  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  let tableCount = $derived(
    entry.snapshot ? Object.values(entry.snapshot.schemas).reduce((n, l) => n + l.length, 0) : null,
  )
  let tab = $derived(tabs.active(session.sessionId))
</script>

<footer>
  <span>{ENGINE_LABEL[session.config.engine]} · {describeConnection(session.config)}</span>
  {#if database}<span>db: {database}</span>{/if}
  {#if tableCount !== null}<span>{formatCount(tableCount)} objects</span>{/if}
  {#if entry.loading}<span>Refreshing schema…</span>{/if}
  <span class="spacer"></span>
  {#if tab}<span>{tab.kind === 'table' ? 'Table' : 'Query'}: {tab.title}</span>{/if}
</footer>

<style>
  footer {
    display: flex;
    gap: 16px;
    padding: 3px 10px;
    border-top: 1px solid var(--border);
    color: var(--fg-muted);
    font-size: 11px;
    background: var(--bg-sidebar);
    flex: none;
    white-space: nowrap;
    overflow: hidden;
  }

  .spacer {
    flex: 1;
  }
</style>
