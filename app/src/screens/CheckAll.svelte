<script module lang="ts">
  /**
   * The number in each grid tile's URL (`grid/<id>?v=<n>`). It only grows, across every mount,
   * so a tile whose area changed never reuses the URL of a picture the page already loaded.
   */
  let tiles = 0;
</script>

<script lang="ts">
  // Step 3, "Check all photos": the watermark is looked for in every photo, then a grid shows
  // each photo with a tick, a badge for the ones that need a look, and a summary line. A tile
  // opens the single-photo editor. "Remove" waits until no ticked photo is still marked "?".
  import { onMount } from 'svelte';
  import { cancelJob, imageUrl, listPhotos, matchAll, photoStates, removalBlockers, setIncluded } from '../lib/api';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import type { PhotoId, PhotoInfo, PhotoState, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';
  import CheckWatermark from './CheckWatermark.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  const c = copy.checkAll;

  let names: Map<PhotoId, string> = $state(new Map());
  let states: PhotoState[] = $state([]);
  let blockers: PhotoId[] = $state([]);
  let versions: Record<PhotoId, number> = $state({});
  let matching = $state(false);
  let done = $state(0);
  let total = $state(0);
  let error: string | null = $state(null);
  /** The photo open in the single-photo editor. */
  let open: PhotoId | null = $state(null);
  let root: HTMLElement | undefined = $state();
  let scrollTop = 0;
  let leaving = false;

  const included = $derived(states.filter((s) => s.included).length);
  const found = $derived(states.filter((s) => s.status === 'found' || s.status === 'edited').length);
  const blocked = $derived(blockers.length > 0);

  function message(e: unknown, fallback: string): string {
    const text = (e as Partial<UserError> | null)?.message;
    return typeof text === 'string' && text !== '' ? text : fallback;
  }

  function setBusy(on: boolean) {
    wizard.busy = on;
  }

  function bump(id: PhotoId) {
    versions[id] = ++tiles;
  }

  async function refresh() {
    const [list, now, stuck] = await Promise.all([listPhotos(), photoStates(), removalBlockers()]);
    names = new Map(list.map((p: PhotoInfo) => [p.id, p.name]));
    for (const s of now) if (versions[s.id] === undefined) bump(s.id);
    states = now;
    blockers = stuck;
  }

  onMount(() => {
    void start();
    return () => {
      if (matching && !leaving) cancelJob().catch(() => {});
      leaving = true;
      setBusy(false);
    };
  });

  async function start() {
    try {
      const first = await photoStates();
      total = first.length;
      if (first.some((s) => s.status === 'pending')) {
        matching = true;
        setBusy(true);
        try {
          await matchAll((e) => {
            if (e.phase === 'matching') {
              done = e.done;
              total = e.total;
            }
          });
        } finally {
          matching = false;
          setBusy(false);
        }
      }
      if (leaving) return;
      await refresh();
    } catch (e) {
      if (leaving) return;
      error = message(e, c.findingFailed);
    }
  }

  function stop() {
    leaving = true;
    cancelJob().catch(() => {});
    wizard.back();
  }

  async function tick(s: PhotoState, box: HTMLInputElement) {
    const on = box.checked;
    error = null;
    try {
      await setIncluded(s.id, on);
      states = states.map((x) => (x.id === s.id ? { ...x, included: on } : x));
      blockers = await removalBlockers();
    } catch (e) {
      error = message(e, c.tickFailed);
      // The box shows what the app has, not what was clicked.
      box.checked = s.included;
    }
  }

  function openPhoto(id: PhotoId) {
    scrollTop = root?.parentElement?.scrollTop ?? 0;
    open = id;
  }

  async function closed(id: PhotoId, saved: boolean) {
    if (saved) bump(id);
    open = null;
    try {
      await refresh();
    } catch (e) {
      error = message(e, c.loadFailed);
    }
    // The grid was hidden while the editor was open, which resets the scroll position.
    requestAnimationFrame(() => {
      if (root?.parentElement) root.parentElement.scrollTop = scrollTop;
    });
    if (root?.parentElement) root.parentElement.scrollTop = scrollTop;
  }

  const badge = (s: PhotoState) =>
    s.status === 'unsure'
      ? 'unsure'
      : s.status === 'notFound'
        ? 'notFound'
        : s.status === 'edited'
          ? 'edited'
          : null;
</script>

<div class="all" bind:this={root}>
  {#if open !== null}
    {@const id = open}
    <CheckWatermark {wizard} mode="photo" photoId={id} onDone={(saved) => closed(id, saved)} />
  {/if}

  <section class="grid-page" hidden={open !== null}>
    {#if matching}
      <Waiting
        sentence={c.finding(done, total)}
        fraction={total > 0 ? done / total : null}
        onCancel={stop}
        cancelLabel={copy.common.back}
      />
    {:else if error !== null && states.length === 0}
      <p class="error" role="alert">{error}</p>
      <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
    {:else if states.length > 0}
      <p class="summary">{c.summary(found, states.length, blockers.length)}</p>
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {/if}
      <ul class="grid" aria-label={c.list}>
        {#each states as s (s.id)}
          {@const name = names.get(s.id) ?? ''}
          {@const kind = badge(s)}
          <li class="tile" class:unsure={kind === 'unsure'}>
            <button
              type="button"
              class="open"
              aria-label={c.openPhoto(name)}
              onclick={() => openPhoto(s.id)}
            >
              <img src={imageUrl('grid', s.id, versions[s.id])} alt={name} loading="lazy" decoding="async" />
            </button>
            {#if kind === 'unsure'}
              <span class="badge unsure"
                ><span aria-hidden="true">{c.badgeUnsure}</span><span class="sr">{c.badgeUnsureLabel}</span></span
              >
            {:else if kind === 'notFound'}
              <span class="badge notFound">{c.badgeNotFound}</span>
            {:else if kind === 'edited'}
              <span class="badge edited">{c.badgeEdited}</span>
            {/if}
            <input
              type="checkbox"
              aria-label={name}
              checked={s.included}
              disabled={!s.included && !s.hasArea}
              title={!s.included && !s.hasArea ? c.noAreaTip : undefined}
              onchange={(e) => tick(s, e.currentTarget)}
            />
          </li>
        {/each}
      </ul>
      <footer>
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
        <div class="actions">
          {#if blocked}
            <span class="hint">{c.needLookHint}</span>
          {/if}
          <button type="button" class="primary" disabled={blocked || included === 0} onclick={() => wizard.go('remove')}>
            {c.removeFrom(included)}
          </button>
        </div>
      </footer>
    {/if}
  </section>
</div>

<style>
  .grid-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) 0;
  }
  .grid-page[hidden] {
    display: none;
  }
  .summary,
  .error {
    margin: 0;
  }
  .error,
  .hint {
    color: var(--danger);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tile {
    position: relative;
    overflow: hidden;
    border: 2px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    /* Tiles far off screen are not laid out or painted until they scroll near. */
    content-visibility: auto;
    contain-intrinsic-size: 150px 150px;
  }
  .tile.unsure {
    border-color: var(--warning);
  }
  .open {
    display: block;
    width: 100%;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: none;
  }
  img {
    display: block;
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
  }
  input {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    width: 20px;
    height: 20px;
    margin: 0;
  }
  .badge {
    position: absolute;
    right: var(--space-2);
    bottom: var(--space-2);
    padding: 1px var(--space-2);
    border-radius: 999px;
    font-size: var(--text-small);
    font-weight: 600;
  }
  .badge.unsure {
    background: var(--warning);
    color: #1c1f24;
  }
  .badge.notFound {
    background: var(--track);
    color: var(--text-muted);
  }
  .badge.edited {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  footer {
    position: sticky;
    bottom: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    background: var(--bg);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
</style>
