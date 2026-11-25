<script lang="ts">
  import { cancelJob, downloadModels } from '../lib/api';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy, DOWNLOAD_MEGABYTES } from '../lib/copy';
  import type { DownloadProgress, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let downloading = $state(false);
  let pausing = false;
  let doneBytes = $state(0);
  let error: string | null = $state(null);

  const total = DOWNLOAD_MEGABYTES;
  const megabytes = $derived(Math.min(total, Math.floor(doneBytes / 1_000_000)));

  function messageOf(e: unknown): string {
    const message = (e as Partial<UserError> | null)?.message;
    return typeof message === 'string' && message !== '' ? message : copy.welcome.failed;
  }

  async function start() {
    error = null;
    pausing = false;
    downloading = true;
    wizard.busy = true;
    try {
      await downloadModels((p: DownloadProgress) => {
        doneBytes = p.doneBytes;
      });
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
    try {
      await cancelJob();
    } catch {
      // The download itself reports how it ended.
    }
  }
</script>

<section class="welcome">
  {#if downloading}
    <Waiting
      sentence={copy.welcome.downloading}
      fraction={doneBytes / (total * 1_000_000)}
      detail={copy.welcome.progress(megabytes)}
      onCancel={pause}
      cancelLabel={copy.welcome.pause}
    />
  {:else}
    <h1>{copy.appName}</h1>
    <p>{copy.welcome.intro}</p>
    <p>{copy.welcome.needsDownload}</p>
    {#if error !== null}
      <p class="error" role="alert">{error}</p>
    {/if}
    <button type="button" class="primary" onclick={start}>
      {error !== null ? copy.welcome.tryAgain : copy.welcome.download}
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
