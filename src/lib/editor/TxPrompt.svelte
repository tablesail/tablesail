<!-- A transaction is still open: commit it, roll it back, or cancel whatever asked. -->
<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import type { QueryTab } from '../stores/tabs.svelte'
  import { finishTx } from './transaction'

  interface Props {
    tab: QueryTab
    /** what the user was about to do, for the explanation ("close this tab" / "switch to auto-commit") */
    action: string
    /** the transaction ended (committed or rolled back); the caller carries on with `action` */
    onDone: () => void
    onCancel: () => void
  }

  let { tab, action, onDone, onCancel }: Props = $props()

  let busy = $state(false)
  let n = $derived(tab.txStatements ?? 0)

  async function end(how: 'commit' | 'rollback') {
    busy = true
    try {
      if (await finishTx(tab, how)) onDone()
    } finally {
      busy = false
    }
  }
</script>

<Modal title="Transaction still open" onClose={onCancel} width="460px" dismissable={!busy}>
  <p class="msg">
    <b>{tab.title}</b> has an open transaction{n ? ` with ${n} statement${n === 1 ? '' : 's'}` : ''}. Commit to keep its
    changes or roll back to discard them before you {action}.
  </p>
  {#snippet footer()}
    <button onclick={onCancel} disabled={busy}>Cancel</button>
    <button class="danger" onclick={() => end('rollback')} disabled={busy}>Roll back</button>
    <button class="primary" onclick={() => end('commit')} disabled={busy}>Commit</button>
  {/snippet}
</Modal>

<style>
  .msg {
    margin: 0;
    line-height: 1.5;
  }
</style>
