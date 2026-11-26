<script lang="ts">
  let { photoId, mode, onDone }: { photoId?: number; mode?: string; onDone?: (saved: boolean) => void } = $props();
</script>

<div data-testid="stub" data-photo={photoId} data-mode={mode}>
  <button type="button" onclick={() => onDone?.(true)}>save</button>
  <button type="button" onclick={() => onDone?.(false)}>back</button>
</div>
