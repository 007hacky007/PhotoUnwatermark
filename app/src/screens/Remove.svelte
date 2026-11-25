<script module lang="ts">
  /**
   * The number in the finished photo's URL (`result/<id>?v=<n>`). It grows with every run, so a
   * photo done again never shows the picture of an earlier run.
   */
  let runs = 0;
</script>

<script lang="ts">
  // Step 4, "Remove": checks that the results can be saved (or asks for a folder), then removes
  // the watermark from every ticked photo with a bar, "Photo X of Y", the time left and the
  // photo being worked on. Stop asks first. While it runs, closing the window asks too.
  import { onMount } from 'svelte';
  import {
    appStatus,
    cancelJob,
    checkOutput,
    chooseFolder,
    closeWindow,
    imageUrl,
    onCloseRequested,
    onQuitRequested,
    remove,
    setOutputOverride,
  } from '../lib/api';
  import DeviceChip from '../lib/components/DeviceChip.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import type { PhotoId, ProgressEvent, RemoveReport, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  const c = copy.remove;
  /** A screen reader hears "Photo X of Y" at most this often. */
  const ANNOUNCE_GAP_MS = 10_000;

  type Phase = 'checking' | 'askFolder' | 'running' | 'failed';
  let phase: Phase = $state('checking');
  let unwritable: string[] = $state([]);
  let error: string | null = $state(null);
  let picking = $state(false);

  // The run.
  let sentence: string = $state(c.starting);
  let fraction: number | null = $state(null);
  let photo: PhotoId | null = $state(null);
  /** How many photos were finished when the current one started. */
  let startDone = 0;
  let number = $state(0);
  let total = $state(0);
  let finished = $state(false);
  let broken = $state(false);
  let secondsLeft: number | null = $state(null);
  let announced = $state('');
  let lastAnnounced = -Infinity;
  let version = $state(0);
  let chip = $state(0);
  let fellBack: string | null = $state(null);
  let details = $state(false);
  let mac = $state(false);
  let stopping = $state(false);
  /** The open question: stop, quit while running, or "the app will close" after Stop. */
  let asking: 'stop' | 'quit' | 'closing' | null = $state(null);
  let quitting = false;

  let running = false;
  let left = false;
  let stopGuards: (() => void)[] = [];

  function message(e: unknown, fallback: string): string {
    const text = (e as Partial<UserError> | null)?.message;
    return typeof text === 'string' && text !== '' ? text : fallback;
  }

  onMount(() => {
    appStatus()
      .then((s) => (mac = s.platform === 'macos'))
      .catch(() => {});
    void check();
    return () => {
      left = true;
      if (running) cancelJob().catch(() => {});
      unguard();
      wizard.busy = false;
    };
  });

  async function check() {
    phase = 'checking';
    error = null;
    let answer;
    try {
      answer = await checkOutput();
    } catch (e) {
      if (left) return;
      error = message(e, c.checkFailed);
      phase = 'failed';
      return;
    }
    if (left) return;
    if (answer.ok) {
      void run();
    } else {
      unwritable = answer.unwritable;
      phase = 'askFolder';
    }
  }

  async function pickFolder() {
    picking = true;
    error = null;
    try {
      const folder = await chooseFolder();
      if (folder === null || left) return;
      await setOutputOverride(folder);
    } catch (e) {
      if (!left) error = message(e, c.folderFailed);
      return;
    } finally {
      picking = false;
    }
    await check();
  }

  /**
   * The person closed the window or quit the app. Returns true when the app stays open to ask
   * first: only while the run goes on, and only once (asked again while quitting, it closes).
   */
  function closeRequested(): boolean {
    if (left || !running || quitting) return false;
    if (stopping) {
      // Already stopping: close once the current photo is done.
      quitting = true;
      sentence = c.quitting;
      asking = 'closing';
    } else {
      asking = 'quit';
    }
    return true;
  }

  function quitRequested() {
    if (!closeRequested()) closeWindow().catch(() => {});
  }

  /** Makes closing the window and quitting the app ask first while the run goes on. */
  async function guard() {
    const keep = (off: () => void) => {
      if (running && !left) stopGuards.push(off);
      else off();
    };
    await Promise.all([
      onCloseRequested(closeRequested).then(keep, () => {}),
      onQuitRequested(quitRequested).then(keep, () => {}),
    ]);
  }

  function unguard() {
    for (const off of stopGuards) off();
    stopGuards = [];
  }

  async function run() {
    phase = 'running';
    running = true;
    wizard.busy = true;
    version = ++runs;
    chip++;
    void guard();
    let report: RemoveReport | null = null;
    try {
      report = await remove(onEvent);
    } catch (e) {
      if (!left) error = message(e, c.failed);
    } finally {
      running = false;
      asking = null;
      unguard();
      wizard.busy = false;
    }
    if (left) return;
    if (quitting) {
      try {
        await closeWindow();
        return;
      } catch {
        // The window stays: show how the run ended as usual.
        quitting = false;
      }
    }
    if (report === null) {
      phase = 'failed';
      return;
    }
    wizard.report = report;
    wizard.resultIndex = 0;
    wizard.go('done');
  }

  function onEvent(e: ProgressEvent) {
    if (left) return;
    if (e.phase === 'loadingModels') {
      if (!stopping) sentence = copy.loading.remover;
      fraction = e.fraction;
    } else if (e.phase === 'removing') {
      if (e.photo !== photo) {
        // The models are ready once the first photo starts.
        if (photo === null) chip++;
        photo = e.photo;
        startDone = e.done;
        broken = false;
      }
      finished = e.done > startDone;
      total = e.total;
      number = Math.min(startDone + 1, e.total);
      fraction = e.total > 0 ? Math.min(1, (e.done + (finished ? 0 : e.photoFraction)) / e.total) : null;
      secondsLeft = e.secondsLeft;
      if (!stopping) sentence = c.photo(number, total);
      announce(c.photo(number, total));
    } else if (e.phase === 'fellBackToProcessor') {
      fellBack = e.reason;
      chip++;
    }
  }

  function announce(text: string) {
    if (text === announced) return;
    const now = Date.now();
    if (now - lastAnnounced < ANNOUNCE_GAP_MS) return;
    announced = text;
    lastAnnounced = now;
  }

  function stop() {
    asking = null;
    stopping = true;
    sentence = quitting ? c.quitting : c.stopping;
    cancelJob().catch(() => {
      // The run reports how it ended either way.
    });
  }

  function stopAndQuit() {
    quitting = true;
    stop();
  }

  const showResult = $derived(finished && !broken);
</script>

<section class="remove">
  {#if phase === 'checking'}
    <Waiting sentence={c.checking} fraction={null} />
  {:else if phase === 'askFolder'}
    <div class="ask">
      <p>{c.cannotSave(unwritable.join(', '))}</p>
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {/if}
      <div class="buttons">
        <button type="button" disabled={picking} onclick={() => wizard.back()}>{copy.common.back}</button>
        <button type="button" class="primary" disabled={picking} onclick={pickFolder}>{c.chooseFolder}</button>
      </div>
    </div>
  {:else if phase === 'failed'}
    <div class="ask">
      <p class="error" role="alert">{error}</p>
      <div class="buttons">
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
      </div>
    </div>
  {:else}
    {#if photo !== null}
      <div class="current">
        {#if showResult}
          <img src={imageUrl('result', photo, version)} alt={c.finishedPhoto} onerror={() => (broken = true)} />
        {:else}
          <img src={imageUrl('thumb', photo)} alt={c.currentPhoto} />
        {/if}
      </div>
    {/if}
    <Waiting
      {sentence}
      {fraction}
      detail={photo !== null && !stopping ? c.timeLeft(secondsLeft) : undefined}
      onCancel={stopping ? undefined : () => (asking = 'stop')}
      cancelLabel={c.stop}
    />
    <span class="visually-hidden" data-testid="announce" aria-live="polite">{announced}</span>
    <!-- Always there, so the sentence is heard when it appears. -->
    <div class="notice" class:shown={fellBack !== null}>
      <p aria-live="polite">{fellBack === null ? '' : c.fellBack(mac)}</p>
      {#if fellBack !== null}
        <button type="button" class="quiet" aria-expanded={details} onclick={() => (details = !details)}>
          {details ? copy.device.hideDetails : copy.device.showDetails}
        </button>
        {#if details}
          <pre>{fellBack}</pre>
        {/if}
      {/if}
    </div>
    <div class="chip"><DeviceChip refresh={chip} /></div>
  {/if}
</section>

{#if asking === 'stop'}
  <Dialog title={c.stopTitle} onClose={() => (asking = null)}>
    <p>{c.keptNote}</p>
    {#snippet actions()}
      <button type="button" onclick={() => (asking = null)}>{c.keepGoing}</button>
      <button type="button" class="danger" onclick={stop}>{c.stop}</button>
    {/snippet}
  </Dialog>
{:else if asking === 'quit'}
  <Dialog title={c.quitTitle} onClose={() => (asking = null)}>
    <p>{c.keptNote}</p>
    {#snippet actions()}
      <button type="button" onclick={() => (asking = null)}>{c.keepGoing}</button>
      <button type="button" class="danger" onclick={stopAndQuit}>{c.stopAndQuit}</button>
    {/snippet}
  </Dialog>
{:else if asking === 'closing'}
  <Dialog title={c.quitting} onClose={() => (asking = null)}>
    <p>{c.keptNote}</p>
    {#snippet actions()}
      <button type="button" class="primary" onclick={() => (asking = null)}>{c.ok}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .remove {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-4);
  }
  .ask {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: min(36rem, 100%);
  }
  .ask p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .buttons {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
  .current img {
    display: block;
    max-width: min(320px, 100%);
    max-height: 240px;
    border-radius: var(--radius);
    object-fit: contain;
  }
  .notice {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    width: min(32rem, 100%);
  }
  .notice.shown {
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--warning);
    border-radius: var(--radius);
  }
  .notice p {
    margin: 0;
  }
  .notice button {
    padding: 0;
    text-decoration: underline;
  }
  pre {
    max-width: 100%;
    margin: 0;
    overflow: auto;
    white-space: pre-wrap;
    font-size: var(--text-small);
    color: var(--text-muted);
  }
</style>
