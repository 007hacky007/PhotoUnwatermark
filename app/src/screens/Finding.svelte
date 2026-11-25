<script lang="ts">
  import { onMount } from 'svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import { stopJob } from '../lib/jobs.svelte';
  import type { DetectOutcome, ProgressEvent, UserError } from '../lib/types';

  type Props = {
    /** Starts the job (finding the watermark, or using a saved one) and reports its progress. */
    /** The sentence shown before the first event; the finder's loading sentence when left out. */
    first?: string;
    run: (onEvent: (e: ProgressEvent) => void) => Promise<DetectOutcome>;
    onDone: (outcome: DetectOutcome) => void;
    /** Leaves the wait: after Back, or after a failure. */
    onBack: () => void;
    /** Files the app downloaded are missing: goes to the download. */
    onDownload?: () => void;
  };

  let { run, first, onDone, onBack, onDownload }: Props = $props();

  // Detection begins by getting the models ready; using a saved watermark loads none.
  // svelte-ignore state_referenced_locally
  let sentence: string = $state(first ?? copy.loading.finder);
  let fraction: number | null = $state(null);
  let error: string | null = $state(null);
  let missingData = $state(false);
  let left = false;
  /** The running call, so Back stops exactly this job. */
  let call: Promise<DetectOutcome> | null = null;

  function onEvent(e: ProgressEvent) {
    if (left) return;
    if (e.phase === 'loadingModels') {
      sentence = e.which === 'finder' ? copy.loading.finder : copy.loading.remover;
      fraction = e.fraction;
    } else if (e.phase === 'reading') {
      sentence = copy.finding.reading;
      fraction = e.fraction;
    } else if (e.phase === 'looking') {
      sentence = copy.finding.looking(Math.min(e.done + 1, e.total), e.total);
      fraction = e.total > 0 ? e.done / e.total : null;
    }
  }

  onMount(async () => {
    try {
      call = run(onEvent);
      const outcome = await call;
      if (!left) onDone(outcome);
    } catch (e) {
      if (left) return;
      const message = (e as Partial<UserError> | null)?.message;
      error = typeof message === 'string' && message !== '' ? message : copy.finding.failed;
      missingData = (e as Partial<UserError> | null)?.missingData === true;
    }
  });

  async function back() {
    left = true;
    // The job reports how it ended; the screen is left either way.
    if (call !== null) await stopJob(call);
    onBack();
  }
</script>

<section class="finding">
  {#if error !== null}
    <p class="error" role="alert">{error}</p>
    <div class="buttons">
      <button type="button" onclick={onBack}>{copy.common.back}</button>
      {#if missingData && onDownload}
        <button type="button" class="primary" onclick={onDownload}>{copy.common.downloadAgain}</button>
      {/if}
    </div>
  {:else}
    <Waiting {sentence} {fraction} onCancel={back} cancelLabel={copy.common.back} />
  {/if}
</section>

<style>
  .finding {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6) var(--space-4);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .buttons {
    display: flex;
    gap: var(--space-2);
  }
</style>
