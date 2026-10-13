<!--
  Shared modal dialog.
  <Modal title="Preview SQL" onClose={() => (open = false)} width="640px">
    body…
    {#snippet footer()}<button onclick={…}>Commit</button>{/snippet}
  </Modal>
  Escape and a click on the backdrop call onClose (unless `dismissable` is false).
-->
<script lang="ts">
  import type { Snippet } from 'svelte'

  interface Props {
    title: string
    onClose: () => void
    width?: string
    dismissable?: boolean
    children: Snippet
    footer?: Snippet
  }

  let { title, onClose, width = '520px', dismissable = true, children, footer }: Props = $props()

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && dismissable) {
      e.stopPropagation()
      onClose()
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="backdrop" onclick={() => dismissable && onClose()}>
  <div
    class="modal"
    style:width
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
  >
    <header>{title}</header>
    <div class="body">{@render children()}</div>
    {#if footer}
      <footer>{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 10vh;
    z-index: 100;
  }

  .modal {
    max-width: calc(100vw - 32px);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.3);
    user-select: text;
    -webkit-user-select: text;
  }

  header {
    padding: 10px 14px;
    font-weight: 600;
    border-bottom: 1px solid var(--border);
  }

  .body {
    padding: 12px 14px;
    overflow: auto;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
  }
</style>
