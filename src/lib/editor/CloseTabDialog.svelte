<!--
  Asks what to do with a query tab that cannot just be closed: an open transaction first
  (commit / roll back), then unsaved edits to its .sql file (save / discard).
-->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import TxPrompt from './TxPrompt.svelte'
  import { queryDirty, tabs, type QueryTab } from '../stores/tabs.svelte'
  import { saveTab } from './sqlFiles'

  interface Props {
    tab: QueryTab
    onClose: () => void
  }

  let { tab, onClose }: Props = $props()

  // svelte-ignore state_referenced_locally
  const id = tab.id
  let busy = $state(false)
  let step = $derived(tab.txId ? 'tx' : queryDirty(tab) ? 'file' : 'done')

  // Nothing (left) to ask about: close it.
  $effect(() => {
    if (step === 'done') finish()
  })

  /** Closes the tab (a no-op if it is already gone, e.g. once saving made this dialog moot) and dismisses the prompt. */
  function finish() {
    tabs.close(id, true)
    onClose()
  }

  async function save() {
    busy = true
    try {
      if (await saveTab(tab)) finish()
    } finally {
      busy = false
    }
  }
</script>

{#if step === 'tx'}
  <TxPrompt {tab} action="close this tab" onDone={() => {}} onCancel={onClose} />
{:else if step === 'file'}
  <Modal title="Unsaved changes" onClose={onClose} width="460px" dismissable={!busy}>
    <p class="msg">Save the changes to <b>{tab.title}</b> before closing?</p>
    {#snippet footer()}
      <button onclick={onClose} disabled={busy}>Cancel</button>
      <button class="danger" onclick={finish} disabled={busy}>Don’t save</button>
      <button class="primary" onclick={save} disabled={busy}>Save</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .msg {
    margin: 0;
    line-height: 1.5;
  }
</style>
