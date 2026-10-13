<script lang="ts">
  import { ui } from '../stores/ui.svelte'
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="toast {t.kind}" onclick={() => ui.dismiss(t.id)}>{t.text}</div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 14px;
    bottom: 34px;
    z-index: 300;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 380px;
  }

  .toast {
    padding: 8px 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-left: 4px solid var(--accent);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.25);
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }

  .toast.error {
    border-left-color: var(--danger);
  }

  .toast.success {
    border-left-color: var(--success);
  }
</style>
