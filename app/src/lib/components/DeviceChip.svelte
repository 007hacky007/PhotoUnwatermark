<script lang="ts">
  // Where the work runs: "Using graphics card (fast)" (Windows), "Using Neural Engine (fast)"
  // (Macs) or "Using processor (slower) - why?". Before any model is loaded it shows nothing,
  // except in Settings ("Not started yet").
  import { onMount } from 'svelte';
  import { appStatus, deviceFix, deviceStatus } from '../api';
  import { copy } from '../copy';
  import type { DeviceFix, DeviceStatus, UserError } from '../types';
  import WhyDialog from './WhyDialog.svelte';

  type Props = {
    inSettings?: boolean;
    /** The system name from `appStatus`; asked for when left out. */
    platform?: string;
    /** Change it to ask for the status again (after models load, say). */
    refresh?: number;
    /** Called after a fix was applied (it may have changed the settings). */
    onChange?: () => void;
  };

  let { inSettings = false, platform, refresh = 0, onChange }: Props = $props();

  let status: DeviceStatus | null = $state(null);
  let system: string | null = $state(null);
  let open = $state(false);
  let working = $state(false);
  let error: string | null = $state(null);
  /** Counts requests, so an answer that arrives after a newer request is dropped. */
  let asked = 0;

  const known = $derived(platform ?? system);
  const fast = $derived(known === 'macos' ? copy.device.neuralEngine : copy.device.gpu);

  onMount(() => {
    if (platform !== undefined) return;
    appStatus()
      .then((s) => (system = s.platform))
      .catch(() => (system = ''));
  });

  async function load() {
    const mine = ++asked;
    try {
      const next = await deviceStatus();
      if (mine === asked) status = next;
    } catch {
      // Without an answer the chip stays as it was; it is a hint, not a control.
    }
  }

  $effect(() => {
    void refresh;
    void load();
  });

  function show() {
    error = null;
    open = true;
  }

  async function fix(chosen: DeviceFix) {
    working = true;
    error = null;
    try {
      await deviceFix(chosen);
    } catch (e) {
      error = (e as Partial<UserError> | null)?.message || copy.device.fixFailed;
      return;
    } finally {
      working = false;
    }
    open = false;
    await load();
    onChange?.();
  }
</script>

{#if status !== null && known !== null}
  {#if status.state === 'notStarted'}
    {#if inSettings}
      <span class="chip">{copy.device.notStarted}</span>
    {/if}
  {:else if status.state === 'gpu' && status.reason === null}
    <span class="chip fast">{fast}</span>
  {:else}
    <button
      type="button"
      class="chip"
      class:fast={status.state === 'gpu'}
      class:slow={status.state === 'cpu'}
      aria-haspopup="dialog"
      onclick={show}
    >
      {status.state === 'gpu' ? fast : copy.device.cpu}
    </button>
  {/if}
  {#if open}
    <WhyDialog {status} platform={known} onFix={fix} onClose={() => (open = false)} {working} {error} />
  {/if}
{/if}

<style>
  .chip {
    display: inline-block;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-muted);
    font-size: var(--text-small);
  }
  .fast {
    color: var(--text);
  }
  button.slow {
    border-color: var(--warning);
    color: var(--text);
  }
</style>
