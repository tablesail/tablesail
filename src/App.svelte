<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  interface CoreInfo {
    version: string
    engines: string[]
  }

  const info: Promise<CoreInfo> = invoke('app_info')
</script>

<main>
  <aside></aside>
  <section>
    <footer>
      {#await info}
        <span>Connecting to core…</span>
      {:then core}
        <span>dbcore v{core.version}</span>
        <span>{core.engines.join(' · ')}</span>
      {:catch err}
        <span class="error">Core unavailable: {String(err)}</span>
      {/await}
    </footer>
  </section>
</main>

<style>
  main {
    display: grid;
    grid-template-columns: 240px 1fr;
    height: 100%;
  }

  aside {
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
  }

  section {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
  }

  footer {
    display: flex;
    justify-content: space-between;
    padding: 4px 10px;
    border-top: 1px solid var(--border);
    color: var(--fg-muted);
    font-size: 11px;
  }

  .error {
    color: #d93025;
  }
</style>
