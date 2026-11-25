<script lang="ts">
  // The "why?" behind the device chip: the reason in plain words, the raw text on request, and
  // the fixes on offer.
  import { copy } from '../copy';
  import type { DeviceFix, DeviceStatus } from '../types';
  import Dialog from './Dialog.svelte';

  type Props = {
    status: DeviceStatus;
    /** `macos`, `windows` or another system name; Macs say "Neural Engine". */
    platform: string;
    onFix: (fix: DeviceFix) => void;
    onClose: () => void;
    /** True while a fix is being applied. */
    working?: boolean;
    error?: string | null;
  };

  let { status, platform, onFix, onClose, working = false, error = null }: Props = $props();

  const id = $props.id();
  let showDetails = $state(false);

  function label(fix: DeviceFix): string {
    if (typeof fix === 'object') return copy.device.fixes.downloadHelper(Math.round(fix.downloadHelper.bytes / 1_000_000));
    if (fix === 'useGraphicsCard') {
      return platform === 'macos' ? copy.device.fixes.useNeuralEngine : copy.device.fixes.useGraphicsCard;
    }
    return copy.device.fixes[fix];
  }
</script>

<Dialog title={copy.device.whyTitle} {onClose}>
  {#if status.reason}
    <p>{status.reason}</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if status.details}
    <div>
      <button
        type="button"
        class="quiet toggle"
        aria-expanded={showDetails}
        aria-controls="{id}-details"
        onclick={() => (showDetails = !showDetails)}
      >
        {showDetails ? copy.device.hideDetails : copy.device.showDetails}
      </button>
      {#if showDetails}
        <pre id="{id}-details">{status.details}</pre>
      {/if}
    </div>
  {/if}

  {#snippet actions()}
    {#each status.fixes as fix (label(fix))}
      <button type="button" class="primary" disabled={working} onclick={() => onFix(fix)}>{label(fix)}</button>
    {/each}
    <button type="button" disabled={working} onclick={onClose}>{copy.common.close}</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
  }
  .error {
    color: var(--danger);
  }
  .toggle {
    padding-left: 0;
    color: var(--accent);
  }
  pre {
    margin: var(--space-2) 0 0;
    padding: var(--space-3);
    overflow: auto;
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text-muted);
    font-size: var(--text-small);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
