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
  import type { DetectOutcome, PhotoInfo, ProgressEvent, Skipped, Suggestion, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';
  import Finding from './Finding.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let photos: PhotoInfo[] = $state([]);
  let skipped: Skipped[] = $state([]);
  let suggestion: Suggestion | null = $state(null);
  let error: string | null = $state(null);
  let hovering = $state(false);
  let adding = $state(false);
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

  async function refreshOffer() {
    const mine = ++asked;
    suggestion = null;
    try {
      const found = await suggestSaved();
      if (mine === asked) suggestion = found;
    } catch {
      // The offer is a bonus; without it the person just finds the watermark.
    }
  }

  async function add(paths: string[]) {
    if (adding || job !== null || paths.length === 0) return;
    adding = true;
    error = null;
    try {
      const result = await addPaths(paths);
      skipped = result.skipped;
      photos = await listPhotos();
      if (result.added.length > 0) await refreshOffer();
      else suggestion = null;
    } catch (e) {
      error = (e as Partial<UserError> | null)?.message || copy.addPhotos.addFailed;
    } finally {
      adding = false;
    }
  }

  async function choose() {
    await add(await choosePhotos().catch(() => []));
  }

  async function chooseDir() {
    const folder = await chooseFolder().catch(() => null);
    if (folder !== null) await add([folder]);
  }

  async function remove(photo: PhotoInfo) {
    try {
      await removePhoto(photo.id);
      photos = photos.filter((p) => p.id !== photo.id);
    } catch (e) {
      error = (e as Partial<UserError> | null)?.message || copy.addPhotos.removeFailed;
      return;
    }
    if (photos.length > 0) await refreshOffer();
    else {
      asked++;
      suggestion = null;
    }
  }

  function begin(run: Run, first?: string) {
    asked++;
    suggestion = null;
    wizard.busy = true;
    job = { run, first };
  }

  function useSuggested() {
    if (suggestion === null) return;
    const id = suggestion.id;
    begin((onEvent) => useSaved(id, onEvent), copy.finding.neutral);
  }

  function leave() {
    wizard.busy = false;
    job = null;
  }

  function done(outcome: DetectOutcome) {
    wizard.outcome = outcome;
    wizard.busy = false;
    job = null;
    wizard.go('check');
  }
</script>

{#if job !== null}
  <Finding run={job.run} first={job.first} onDone={done} onBack={leave} />
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
            <img src={imageUrl('thumb', photo.id)} alt={photo.name} loading="lazy" />
            <button type="button" class="remove" aria-label={copy.addPhotos.remove(photo.name)} onclick={() => remove(photo)}>
              ×
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <footer>
      <button
        type="button"
        class="primary"
        disabled={photos.length === 0 || adding}
        onclick={() => begin(detect)}
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
    justify-content: flex-end;
  }
</style>
