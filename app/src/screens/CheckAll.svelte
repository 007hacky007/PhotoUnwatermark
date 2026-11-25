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
  import type { PhotoId, PhotoInfo, PhotoProblem, PhotoState, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';
  import CheckWatermark from './CheckWatermark.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  const c = copy.checkAll;

  let names: Map<PhotoId, string> = $state(new Map());
  /** Why a photo that cannot be read cannot be used. */
  let problems: Map<PhotoId, PhotoProblem> = $state(new Map());
  let states: PhotoState[] = $state([]);
  let blockers: PhotoId[] = $state([]);
  let versions: Record<PhotoId, number> = $state({});
  let loaded = $state(false);
  /** Ticks waiting for the app; Remove waits for them. */
  let ticking = $state(0);
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
  // The app's own rule (MatchSummary): a photo with its own area counts as found only when that
  // area is not empty.
  const found = $derived(states.filter((s) => s.status === 'found' || (s.status === 'edited' && s.hasArea)).length);
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
    problems = new Map(list.flatMap((p: PhotoInfo) => (p.problem ? [[p.id, p.problem] as const] : [])));
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
    let first: PhotoState[];
    try {
      first = await photoStates();
    } catch (e) {
      if (!leaving) error = message(e, c.loadFailed);
      loaded = true;
      return;
    }
    if (leaving) return;
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
      } catch (e) {
        if (!leaving) error = message(e, c.findingFailed);
        loaded = true;
        return;
      } finally {
        matching = false;
        setBusy(false);
      }
    }
    if (leaving) return;
    try {
      await refresh();
    } catch (e) {
      if (!leaving) error = message(e, first.some((s) => s.status === 'pending') ? c.refreshFailed : c.loadFailed);
    }
    loaded = true;
  }

  function stop() {
    leaving = true;
    cancelJob().catch(() => {});
    wizard.back();
  }

  async function tick(s: PhotoState, box: HTMLInputElement) {
    const on = box.checked;
    error = null;
    ticking++;
    try {
      let stuck: PhotoId[];
      try {
        await setIncluded(s.id, on);
      } catch (e) {
        error = message(e, c.tickFailed);
        // The box shows what the app has, not what was clicked.
        box.checked = s.included;
        return;
      }
      try {
        stuck = await removalBlockers();
      } catch {
        // The change was kept; read everything again so the list and Remove agree.
        try {
          await refresh();
        } catch (e) {
          error = message(e, c.loadFailed);
        }
        box.checked = states.find((x) => x.id === s.id)?.included ?? s.included;
        return;
      }
      // Both together, so Remove never sees one without the other.
      states = states.map((x) => (x.id === s.id ? { ...x, included: on } : x));
      blockers = stuck;
    } finally {
      ticking--;
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

<div class="all" class:single={open !== null} bind:this={root}>
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
    {:else if !loaded}
      <p class="summary">{c.loading}</p>
      <footer>
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
      </footer>
    {:else if states.length === 0}
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {:else}
        <p class="summary">{c.empty}</p>
      {/if}
      <footer>
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
      </footer>
    {:else}
      <p class="summary">{c.summary(found, states.length, blockers.length)}</p>
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {/if}
      <ul class="grid" aria-label={c.list}>
        {#each states as s (s.id)}
          {@const name = names.get(s.id) ?? ''}
          {@const kind = badge(s)}
          {@const unreadable = s.status === 'unreadable'}
          {@const reason = unreadable ? copy.photoProblem[problems.get(s.id) ?? 'damaged'] : c.noAreaTip}
          <li class="tile" class:unsure={kind === 'unsure'}>
            <div class="pic">
              {#if unreadable}
                <div class="problem" role="img" aria-label={name}>{name}</div>
              {:else}
                <button
                  type="button"
                  class="open"
                  aria-label={c.openPhoto(name, kind)}
                  onclick={() => openPhoto(s.id)}
                >
                  <img src={imageUrl('grid', s.id, versions[s.id])} alt={name} loading="lazy" decoding="async" />
                </button>
              {/if}
              {#if kind === 'unsure'}
                <span class="badge unsure" aria-hidden="true">{c.badgeUnsure}</span>
              {:else if kind === 'notFound'}
                <span class="badge notFound" aria-hidden="true">{c.badgeNotFound}</span>
              {:else if kind === 'edited'}
                <span class="badge edited" aria-hidden="true">{c.badgeEdited}</span>
              {/if}
            </div>
            <input
              type="checkbox"
              aria-label={c.include(name)}
              checked={s.included}
              disabled={unreadable || (!s.included && !s.hasArea)}
              title={unreadable || (!s.included && !s.hasArea) ? reason : undefined}
              onchange={(e) => tick(s, e.currentTarget)}
            />
            {#if unreadable || (!s.included && !s.hasArea)}
              <span class="why">{reason}</span>
            {/if}
          </li>
        {/each}
      </ul>
      <footer>
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
        <div class="actions">
          <span class="hint" aria-live="polite">{blocked ? c.needLookHint : ''}</span>
          <button type="button" class="primary" disabled={blocked || included === 0 || ticking > 0} onclick={() => wizard.go('remove')}>
            {c.removeFrom(included)}
          </button>
        </div>
      </footer>
    {/if}
  </section>
</div>

<style>
  /* The single-photo editor takes the whole height, like step 2. */
  .all.single {
    height: 100%;
  }
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
  .error {
    color: var(--danger);
  }
  .hint {
    color: var(--text-muted);
  }
  .why {
    display: block;
    padding: var(--space-1) var(--space-2);
    font-size: var(--text-small);
    color: var(--text-muted);
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
  /* The photo and its badge; the reason below it never sits under the badge. */
  .pic {
    position: relative;
  }
  .problem {
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    aspect-ratio: 1;
    padding: var(--space-2);
    overflow: hidden;
    background: var(--track);
    color: var(--text-muted);
    font-size: var(--text-small);
    overflow-wrap: anywhere;
    text-align: center;
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
