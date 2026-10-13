<script lang="ts">
  import { pickFile, type FileFilter } from '../util/dialogs'

  interface Props {
    value: string | null | undefined
    placeholder?: string
    title?: string
    filters?: FileFilter[]
  }

  let { value = $bindable(), placeholder = '', title = 'Choose file', filters }: Props = $props()

  async function browse() {
    const path = await pickFile({ title, filters })
    if (path) value = path
  }
</script>

<div class="pick">
  <input type="text" bind:value={() => value ?? '', (v) => (value = v || null)} {placeholder} />
  <button type="button" onclick={browse}>Browse…</button>
</div>

<style>
  .pick {
    display: flex;
    gap: 6px;
  }

  input {
    flex: 1;
  }
</style>
