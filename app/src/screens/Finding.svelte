<script lang="ts">
  import { onMount } from 'svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import { cancelJob } from '../lib/api';
  import type { DetectOutcome, ProgressEvent, UserError } from '../lib/types';

  type Props = {
    /** Starts the job (finding the watermark, or using a saved one) and reports its progress. */
    /** The sentence shown before the first event; the finder's loading sentence when left out. */
    first?: string;
    run: (onEvent: (e: ProgressEvent) => void) => Promise<DetectOutcome>;
    onDone: (outcome: DetectOutcome) => void;
    /** Leaves the wait: after Back, or after a failure. */
    onBack: () => void;
  };

  let { run, first, onDone, onBack }: Props = $props();

  // Detection begins by getting the models ready; using a saved watermark loads none.
  // svelte-ignore state_referenced_locally
  let sentence: string = $state(first ?? copy.loading.finder);
  let fraction: number | null = $state(null);
  let error: string | null = $state(null);
  let left = false;

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
      const outcome = await run(onEvent);
      if (!left) onDone(outcome);
    } catch (e) {
      if (left) return;
      const message = (e as Partial<UserError> | null)?.message;
      error = typeof message === 'string' && message !== '' ? message : copy.finding.failed;
    }
  });

  async function back() {
    left = true;
    try {
      await cancelJob();
    } catch {
      // The job reports how it ended; the screen is left either way.
    }
    onBack();
  }
</script>

<section class="finding">
  {#if error !== null}
    <p class="error" role="alert">{error}</p>
    <button type="button" onclick={onBack}>{copy.common.back}</button>
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
</style>
