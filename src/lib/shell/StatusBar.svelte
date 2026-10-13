<!-- Window footer: connection state, address, safety mode, engine and app version. -->
<script lang="ts">
  import Icon from '../ui/Icon.svelte'
  import { sessions } from '../stores/sessions.svelte'
  import type { SessionInfo } from '../types'
  import { describeConnection } from '../util/env'

  interface Props {
    session: SessionInfo
    appVersion: string
  }

  let { session, appVersion }: Props = $props()

  let database = $derived(sessions.database(session.sessionId))
  let entry = $derived(sessions.schemaEntry(session.sessionId, database))
  let safety = $derived(
    session.config.readOnly
      ? 'Read-only'
      : session.config.env === 'production'
        ? 'Production · writes need confirmation'
        : 'Safe mode off',
  )
</script>

<footer class="footer">
  <span class="part"><i class="live-dot"></i><b>Connected</b></span>
  <span class="part"><Icon name="database" size={12} /> {describeConnection(session.config)}</span>
  <span class="part" class:warn={session.config.env === 'production'}><Icon name="shield" size={12} /> {safety}</span>
  {#if entry.loading}<span class="part">Refreshing schema…</span>{/if}
  <span class="right">
    {#if database}<span>{database}</span>{/if}
    <span>{session.serverVersion}</span>
    <span>TableSoft{appVersion ? ` v${appVersion}` : ''}</span>
  </span>
</footer>

<style>
  .footer {
    height: 30px;
    min-height: 30px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 13px;
    border-top: 1px solid var(--border);
    background: var(--bg-rail);
    color: var(--fg-faint);
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
  }

  .part {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .part b {
    color: var(--fg-muted);
    font-weight: 550;
  }

  .part.warn {
    color: var(--danger);
  }

  .right {
    margin-left: auto;
    display: flex;
    gap: 16px;
  }
</style>
