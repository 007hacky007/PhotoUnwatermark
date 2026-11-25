<script lang="ts">
  let { photoId, onDone }: { photoId?: number; onDone?: (saved: boolean) => void } = $props();
</script>

<div data-testid="stub" data-photo={photoId}>
  <button type="button" onclick={() => onDone?.(true)}>save</button>
  <button type="button" onclick={() => onDone?.(false)}>back</button>
</div>
