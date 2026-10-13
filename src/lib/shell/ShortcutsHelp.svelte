<script lang="ts">
  import Modal from '../ui/Modal.svelte'
  import { modLabel } from '../util/format'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  const M = modLabel
  const rows: [string, string][] = [
    ['New query tab', `${M}+T`],
    ['Close tab', `${M}+W`],
    ['Select tab 1-9', `${M}+1 … 9`],
    ['Next / previous tab', 'Ctrl+Tab / Ctrl+Shift+Tab'],
    ['Command palette (tables, actions, saved queries)', `${M}+K  or  ${M}+P`],
    ['Toggle light / dark theme', `${M}+Shift+L`],
    ['Run statement under cursor (or selection)', `${M}+Enter`],
    ['Run whole script (or selection)', `${M}+Shift+Enter`],
    ['Commit pending changes', `${M}+S`],
    ['Refresh table data', `${M}+R`],
    ['Filters bar (show / hide)', `${M}+F`],
    ['Duplicate selected rows', `${M}+D`],
    ['Set selected cells to NULL', `${M}+Shift+N  or  ${M}+Backspace`],
    ['Delete selected rows', 'Delete'],
    ['New connection (connections screen)', `${M}+N`],
    ['Settings', `${M}+,`],
    ['Keyboard shortcuts', `${M}+/`],
  ]
</script>

<Modal title="Keyboard shortcuts" {onClose} width="480px">
  <table>
    <tbody>
      {#each rows as [label, keys] (label)}
        <tr>
          <td>{label}</td>
          <td class="keys"><kbd>{keys}</kbd></td>
        </tr>
      {/each}
    </tbody>
  </table>
</Modal>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
  }

  td {
    padding: 5px 0;
    border-bottom: 1px solid var(--border);
  }

  .keys {
    text-align: right;
    white-space: nowrap;
  }
</style>
