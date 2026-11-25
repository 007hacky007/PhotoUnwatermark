<script lang="ts">
  import { onMount } from 'svelte';
  import {
    addPaths,
    chooseFolder,
    choosePhotos,
    detect,
    imageUrl,
    listPhotos,
    onDragDrop,
    removePhoto,
    suggestSaved,
    useSaved,
  } from '../lib/api';
  import { copy } from '../lib/copy';
  import { stopJob, stopping } from '../lib/jobs.svelte';
  import type { DetectOutcome, PhotoInfo, ProgressEvent, Skipped, Suggestion, UserError } from '../lib/types';
  import { sameSet, type Wizard } from '../lib/wizard.svelte';
  import Finding from './Finding.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let photos: PhotoInfo[] = $state([]);
  let skipped: Skipped[] = $state([]);
  let suggestion: Suggestion | null = $state(null);
  let error: string | null = $state(null);
  let hovering = $state(false);
  /** True while photos are added or removed; the list must not change twice at once. */
  let adding = $state(false);
  /** True while the saved watermarks are checked; nothing waits for it. */
  let checking = $state(false);
  /** The running check's call, so a stop names exactly its ticket. */
  let check: Promise<unknown> | null = null;
  /** The running job, or null while this screen is the one showing. */
  type Run = (onEvent: (e: ProgressEvent) => void) => Promise<DetectOutcome>;
  let job: { run: Run; first?: string } | null = $state(null);
  /** Counts offers asked for, so an answer that arrives after a newer question is dropped. */
  let asked = 0;

  onMount(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    listPhotos()
      .then((list) => (photos = list))
      .catch(() => {});
    onDragDrop((e) => {
      if (job !== null) return;
      if (e.kind === 'drop') {
        hovering = false;
        void add(e.paths);
      } else {
        hovering = e.kind === 'hover';
      }
    })
      .then((unlisten) => (gone ? unlisten() : (stop = unlisten)))
      .catch(() => {});
    return () => {
      gone = true;
      stop?.();
    };
  });

  /**
   * Checks the photos against the saved watermarks in the background: the buttons stay usable,
   * and adding or removing photos or finding the watermark stops the check first.
   */
  function refreshOffer() {
    const mine = ++asked;
    suggestion = null;
    checking = true;
    const call = suggestSaved();
    check = call;
    call
      .then((found) => {
        if (mine === asked) suggestion = found;
      })
      .catch(() => {
        // The offer is a bonus; without it the person just finds the watermark.
      })
      .finally(() => {
        if (mine === asked) {
          checking = false;
          check = null;
        }
      });
  }

  /**
   * Stops a check that is still running (its answer is dropped). The stop names the check's own
   * ticket and is sent at once; it resolves once the app has taken it, so a job started after it
   * waits for the check to end instead of finding the app busy.
   */
  function stopCheck(): Promise<void> {
    asked++;
    suggestion = null;
    const running = checking ? check : null;
    checking = false;
    check = null;
    return running === null ? Promise.resolve() : stopJob(running);
  }

  /** Reads the list again after a picture failed to load, to show why. */
  let rechecking = false;
  function pictureFailed() {
    if (rechecking) return;
    rechecking = true;
    queueMicrotask(() => {
      listPhotos()
        .then((list) => (photos = list))
        .catch(() => {})
        .finally(() => (rechecking = false));
    });
  }

  async function add(paths: string[]) {
    if (adding || job !== null || paths.length === 0) return;
    adding = true;
    error = null;
    let added = false;
    try {
      await stopCheck();
      const result = await addPaths(paths);
      skipped = result.skipped;
      photos = await listPhotos();
      added = result.added.length > 0;
    } catch (e) {
      error = (e as Partial<UserError> | null)?.message || copy.addPhotos.addFailed;
    } finally {
      adding = false;
    }
    if (added) refreshOffer();
  }

  async function choose() {
    await add(await choosePhotos().catch(() => []));
  }

  async function chooseDir() {
    const folder = await chooseFolder().catch(() => null);
    if (folder !== null) await add([folder]);
  }

  async function remove(photo: PhotoInfo) {
    if (adding || job !== null) return;
    adding = true;
    try {
      await stopCheck();
      await removePhoto(photo.id);
      photos = photos.filter((p) => p.id !== photo.id);
    } catch (e) {
      error = (e as Partial<UserError> | null)?.message || copy.addPhotos.removeFailed;
      return;
    } finally {
      adding = false;
    }
    if (photos.length > 0) refreshOffer();
  }

  async function begin(run: Run, first?: string) {
    if (adding || job !== null) return;
    adding = true;
    try {
      await stopCheck();
    } finally {
      adding = false;
    }
    wizard.busy = true;
    job = { run, first };
  }

  /** Step 2 again as it was when the photos did not change, else a new search. */
  function find() {
    if (sameSet(wizard, photos.map((p) => p.id))) {
      void stopCheck();
      wizard.go('check');
      return;
    }
    void begin(detect);
  }

  function downloadAgain() {
    wizard.busy = false;
    job = null;
    wizard.go('welcome');
  }

  function useSuggested() {
    if (suggestion === null) return;
    const id = suggestion.id;
    void begin((onEvent) => useSaved(id, onEvent), copy.finding.neutral);
  }

  function leave() {
    wizard.busy = false;
    job = null;
  }

  function done(outcome: DetectOutcome) {
    wizard.outcome = outcome;
    wizard.outcomePhotos = photos.map((p) => p.id);
    wizard.busy = false;
    job = null;
    wizard.go('check');
  }
</script>

{#if job !== null}
  <Finding run={job.run} first={job.first} onDone={done} onBack={leave} onDownload={downloadAgain} />
{:else}
  <section class="add">
    <div class="drop" class:hovering role="group" aria-label={copy.addPhotos.drop}>
      <p class="hint">{hovering ? copy.addPhotos.dropActive : copy.addPhotos.drop}</p>
      <div class="choosers">
        <button type="button" onclick={choose} disabled={adding}>{copy.addPhotos.choosePhotos}</button>
        <button type="button" onclick={chooseDir} disabled={adding}>{copy.addPhotos.chooseFolder}</button>
      </div>
    </div>

    {#if error !== null}
      <p class="error" role="alert">{error}</p>
    {/if}

    {#if skipped.length > 0}
      <div class="skipped" role="status">
        <p>{copy.addPhotos.skipped(skipped.length)}</p>
        <ul>
          {#each skipped as s}
            <li>{copy.addPhotos.skippedItem(s.name, s.reason)}</li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if adding && stopping.count > 0}
      <p class="checking" role="status">{copy.addPhotos.waitingForStop}</p>
    {:else if checking}
      <p class="checking" role="status">{copy.addPhotos.checkingSaved}</p>
    {/if}

    {#if suggestion !== null}
      <div class="offer" role="status">
        <p>{copy.addPhotos.offer(suggestion.name)}</p>
        <button type="button" class="primary" onclick={useSuggested}>
          {copy.addPhotos.yes}
        </button>
        <button type="button" onclick={() => (suggestion = null)}>{copy.addPhotos.no}</button>
      </div>
    {/if}

    {#if photos.length > 0}
      <p class="count">{copy.addPhotos.count(photos.length)}</p>
      <ul class="grid">
        {#each photos as photo (photo.id)}
          <li>
            {#if photo.problem}
              <div class="problem" role="img" aria-label={photo.name}>
                <span class="name">{photo.name}</span>
                <span>{copy.photoProblem[photo.problem]}</span>
              </div>
            {:else}
              <img src={imageUrl('thumb', photo.id)} alt={photo.name} loading="lazy" onerror={pictureFailed} />
            {/if}
            <button
              type="button"
              class="remove"
              aria-label={copy.addPhotos.remove(photo.name)}
              disabled={adding}
              onclick={() => remove(photo)}
            >
              ×
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <footer>
      <button type="button" onclick={() => wizard.go('saved')}>{copy.addPhotos.savedWatermarks}</button>
      <button
        type="button"
        class="primary"
        disabled={photos.length === 0 || adding}
        onclick={find}
      >
        {copy.addPhotos.find}
      </button>
    </footer>
  </section>
{/if}

<style>
  .add {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
  }
  .drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6) var(--space-4);
    border: 2px dashed var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .drop.hovering {
    border-color: var(--accent);
  }
  .hint,
  .count,
  .skipped p,
  .offer p {
    margin: 0;
  }
  .hint {
    font-size: var(--text-large);
  }
  .choosers,
  .offer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .skipped ul {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-5);
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .checking {
    margin: 0;
    color: var(--text-muted);
  }
  .problem {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-1);
    box-sizing: border-box;
    width: 100%;
    aspect-ratio: 1;
    padding: var(--space-2);
    overflow: hidden;
    border-radius: var(--radius);
    background: var(--track);
    color: var(--text-muted);
    font-size: var(--text-small);
    overflow-wrap: anywhere;
  }
  .problem .name {
    color: var(--text);
    font-weight: 600;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(110px, 1fr));
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .grid li {
    position: relative;
  }
  .grid img {
    display: block;
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: var(--radius);
    background: var(--track);
  }
  .remove {
    position: absolute;
    top: var(--space-1);
    right: var(--space-1);
    min-width: 28px;
    padding: 0 var(--space-2);
    opacity: 0;
  }
  .grid li:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-2);
  }
</style>
