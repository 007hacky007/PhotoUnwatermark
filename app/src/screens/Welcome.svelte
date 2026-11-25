<script lang="ts">
  import { downloadModels } from '../lib/api';
  import { stopJob } from '../lib/jobs.svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy, DOWNLOAD_MEGABYTES } from '../lib/copy';
  import type { DownloadProgress, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let downloading = $state(false);
  let pausing = false;
  /** The running download's call, so Pause names exactly its ticket. */
  let call: Promise<void> | null = null;
  let doneBytes = $state(0);
  let error: string | null = $state(null);
  /** Seconds the rest should take at the speed so far; null until that speed is known. */
  let secondsLeft: number | null = $state(null);
  /** Where the speed is measured from: the first report of this download (resumed bytes count as there already). */
  let since: { at: number; bytes: number } | null = null;

  const total = DOWNLOAD_MEGABYTES;
  const megabytes = $derived(Math.min(total, Math.floor(doneBytes / 1_000_000)));
  /** The speed is trusted after this long and this much. */
  const MEASURE_MS = 2000;
  const MEASURE_BYTES = 1_000_000;

  function onProgress(p: DownloadProgress) {
    doneBytes = p.doneBytes;
    const now = Date.now();
    if (since === null) {
      since = { at: now, bytes: p.doneBytes };
      return;
    }
    const ms = now - since.at;
    const bytes = p.doneBytes - since.bytes;
    if (ms >= MEASURE_MS && bytes >= MEASURE_BYTES) {
      const perSecond = bytes / (ms / 1000);
      secondsLeft = Math.max(0, (p.totalBytes - p.doneBytes) / perSecond);
    }
  }

  function messageOf(e: unknown): string {
    const message = (e as Partial<UserError> | null)?.message;
    return typeof message === 'string' && message !== '' ? message : copy.welcome.failed;
  }

  async function start() {
    if (downloading) return;
    error = null;
    pausing = false;
    downloading = true;
    secondsLeft = null;
    since = null;
    wizard.busy = true;
    try {
      call = downloadModels(onProgress);
      await call;
      wizard.busy = false;
      wizard.go('add');
    } catch (e) {
      // A pause also ends the call with an error; it is not one to show.
      if (!pausing) error = messageOf(e);
    } finally {
      downloading = false;
      wizard.busy = false;
    }
  }

  async function pause() {
    pausing = true;
    // The download itself reports how it ended.
    if (call !== null) await stopJob(call);
  }
</script>

<section class="welcome">
  {#if downloading}
    <Waiting
      sentence={copy.welcome.downloading}
      fraction={doneBytes / (total * 1_000_000)}
      detail={copy.welcome.progressLeft(megabytes, secondsLeft === null ? null : copy.remove.timeLeft(secondsLeft))}
      onCancel={pause}
      cancelLabel={copy.welcome.pause}
    />
  {:else}
    <h1>{copy.appName}</h1>
    <p>{copy.welcome.intro}</p>
    <p>{copy.welcome.needsDownload}</p>
    {#if error !== null}
      <p class="error" role="alert">{error}</p>
    {:else if doneBytes > 0}
      <p>{copy.welcome.pausedAt(megabytes)}</p>
    {/if}
    <button type="button" class="primary" onclick={start}>
      {error !== null ? copy.welcome.tryAgain : doneBytes > 0 ? copy.welcome.resume : copy.welcome.download}
    </button>
  {/if}
</section>

<style>
  .welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    max-width: 36rem;
    margin: 0 auto;
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }
  h1,
  p {
    margin: 0;
  }
  .error {
    color: var(--danger, inherit);
  }
</style>
