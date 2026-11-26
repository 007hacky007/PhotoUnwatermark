<script module lang="ts">
  /**
   * The number in each grid tile's URL (`grid/<id>?v=<n>`). It only grows, across every mount,
   * so a tile whose area changed never reuses the URL of a picture the page already loaded.
   */
  let tiles = 0;
</script>

<script lang="ts">
  // Step 3, "Check all photos": the current group's watermark is looked for in every photo with
  // no group yet, then a grid shows each photo with a tick, a badge for the ones that need a
  // look, and a summary line. A tile opens the single-photo editor. "Remove" waits until no
  // ticked photo is still marked "?". While photos have no group, the next watermark can be
  // found for them, a saved one used for them, or they can be left out.
  import { onMount } from 'svelte';
  import {
    detect,
    groupSample,
    groupsState,
    imageUrl,
    listPhotos,
    matchAll,
    photoStates,
    removalBlockers,
    setIncluded,
    suggestSaved,
  } from '../lib/api';
  import Dialog from '../lib/components/Dialog.svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import { startFromSaved } from '../lib/groups';
  import { stopJob } from '../lib/jobs.svelte';
  import type {
    DetectOutcome,
    GroupsState,
    PhotoId,
    PhotoInfo,
    PhotoProblem,
    PhotoState,
    Suggestion,
    UserError,
  } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';
  import CheckWatermark from './CheckWatermark.svelte';
  import Finding from './Finding.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  const c = copy.checkAll;
  const g = copy.groups;

  let names: Map<PhotoId, string> = $state(new Map());
  /** Why a photo that cannot be read cannot be used. */
  let problems: Map<PhotoId, PhotoProblem> = $state(new Map());
  let states: PhotoState[] = $state([]);
  let blockers: PhotoId[] = $state([]);
  let groups = $state<GroupsState | null>(null);
  let versions: Record<PhotoId, number> = $state({});
  let loaded = $state(false);
  /** Ticks waiting for the app; the next steps wait for them. */
  let ticking = $state(0);
  /** Counts reads after a tick, so only the newest one is shown. */
  let reads = 0;
  let matching = $state(false);
  let done = $state(0);
  let total = $state(0);
  let error: string | null = $state(null);
  /** The photo open in the single-photo editor. */
  let open: PhotoId | null = $state(null);
  let root: HTMLElement | undefined = $state();
  let scrollTop = 0;
  /** The screen is gone. */
  let leaving = false;
  /** Back was pressed: on the way to step 2. */
  let backing = $state(false);
  /** The running search, so a stop names exactly its ticket. */
  let search: Promise<unknown> | null = null;

  /** On the way to the next group: checking the saved watermarks, the offer, or the first guess. */
  type Next = { kind: 'checking' } | { kind: 'offer'; suggestion: Suggestion } | { kind: 'finding' };
  let next: Next | null = $state(null);
  /** The running saved-watermark check, so Back stops exactly its ticket. */
  let checkCall: Promise<unknown> | null = null;
  /** Counts checks asked for, so an answer that arrives after Back is dropped. */
  let asked = 0;
  /** The offered saved watermark is being used. */
  let using = $state(false);
  /** The question before a removal that leaves the photos with no group out. */
  let asking = $state(false);

  const current = $derived(groups?.current ?? null);
  /** The current group's photos: what its matching found, and the photos opened for it. */
  const mine = $derived(
    states.filter((s) => current !== null && s.group === current && s.status !== 'pending' && s.status !== 'unreadable'),
  );
  // The app's own rule (MatchSummary): a photo with its own area counts as found only when that
  // area is not empty. Only the group's members count (an unticked photo has no group yet).
  const found = $derived(
    mine.filter((s) => s.member && (s.status === 'found' || (s.status === 'edited' && s.hasArea))).length,
  );
  const needLook = $derived.by(() => {
    const ids = new Set(mine.map((s) => s.id));
    return blockers.filter((id) => ids.has(id)).length;
  });
  /** The photos a removal takes: the members of every group. */
  const members = $derived(states.filter((s) => s.member).length);
  const leftovers = $derived(groups?.leftovers ?? 0);
  const blocked = $derived(blockers.length > 0);
  /**
   * The next steps wait for every "?", every tick on its way, and the current group's matching
   * (the app refuses them until it has finished, for example after a failed one).
   */
  const held = $derived(blocked || ticking > 0 || backing || (groups !== null && !groups.matched));

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
    const [list, now, stuck, gs] = await Promise.all([listPhotos(), photoStates(), removalBlockers(), groupsState()]);
    names = new Map(list.map((p: PhotoInfo) => [p.id, p.name]));
    problems = new Map(list.flatMap((p: PhotoInfo) => (p.problem ? [[p.id, p.problem] as const] : [])));
    for (const s of now) if (versions[s.id] === undefined) bump(s.id);
    states = now;
    blockers = stuck;
    groups = gs;
  }

  onMount(() => {
    void start();
    return () => {
      leaving = true;
      if (matching && search !== null && !backing) void stopJob(search);
      if (checkCall !== null) void stopJob(checkCall);
      setBusy(false);
    };
  });

  /** Reads the groups and the photos, and matches the current group first when it was not. */
  async function start() {
    loaded = false;
    error = null;
    let first: PhotoState[];
    let gs: GroupsState;
    try {
      [gs, first] = await Promise.all([groupsState(), photoStates()]);
    } catch (e) {
      if (!leaving) error = message(e, c.loadFailed);
      loaded = true;
      return;
    }
    // Back while reading: step 2 comes next, so nothing may start a job here.
    if (leaving || backing) return;
    groups = gs;
    const match = gs.current !== null && !gs.matched;
    if (match) {
      done = 0;
      total = first.filter((s) => s.status === 'pending').length;
      matching = true;
      setBusy(true);
      try {
        search = matchAll((e) => {
          if (e.phase === 'matching') {
            done = e.done;
            total = e.total;
          }
        });
        await search;
      } catch (e) {
        if (leaving || backing) return;
        error = message(e, c.findingFailed);
        // A grid from before (a saved watermark used in place) shows what the app has now:
        // the photos with no group were reset for the new group.
        if (states.length > 0) {
          versions = {};
          try {
            await refresh();
          } catch {
            // The error above is already shown.
          }
        }
        loaded = true;
        return;
      } finally {
        search = null;
        // Stopped by Back: the wait stays on screen until step 2 shows.
        if (!backing) matching = false;
        setBusy(false);
      }
      // The photos matched now have new areas: every tile loads its picture again.
      versions = {};
    }
    if (leaving || backing) return;
    try {
      await refresh();
    } catch (e) {
      if (!leaving) error = message(e, match ? c.refreshFailed : c.loadFailed);
    }
    loaded = true;
  }

  /**
   * Back from step 3, on every path (the grid, the wait while matching, an empty or failed
   * screen): to step 2 of the current group with its drawing kept. Matching still running is
   * stopped first, and step 2 waits for the stop to be taken. A group started from a saved
   * watermark gets its photos for step 2 first.
   */
  async function back() {
    if (backing) return;
    backing = true;
    const running = matching ? search : null;
    if (running !== null) await stopJob(running);
    const drawing = wizard.current;
    if (drawing !== null && drawing.outcome === null) {
      try {
        const outcome: DetectOutcome = await groupSample();
        // A new drawing, never one changed in place (the wizard does not proxy drawings).
        wizard.current = { ...drawing, outcome };
      } catch (e) {
        backing = false;
        if (leaving) return;
        matching = false;
        error = message(e, c.loadFailed);
        if (running !== null || !loaded) {
          try {
            await refresh();
          } catch {
            // The error above is already shown.
          }
          loaded = true;
        }
        return;
      }
    }
    if (leaving) return;
    wizard.go('check');
  }

  async function tick(s: PhotoState, box: HTMLInputElement) {
    const on = box.checked;
    error = null;
    ticking++;
    try {
      try {
        await setIncluded(s.id, on);
      } catch (e) {
        error = message(e, c.tickFailed);
        // The box shows what the app has, not what was clicked.
        box.checked = s.included;
        return;
      }
      const read = ++reads;
      try {
        // A tick can move a photo out of its group: read everything it changes, together, so
        // the tiles, the counts and the buttons always agree.
        const [now, stuck, gs] = await Promise.all([photoStates(), removalBlockers(), groupsState()]);
        if (read !== reads) return;
        const after = now.find((x) => x.id === s.id);
        // Out of an earlier group: its area is gone from the picture.
        if (after !== undefined && after.group !== s.group) bump(s.id);
        states = now;
        blockers = stuck;
        groups = gs;
        box.checked = after?.included ?? on;
      } catch {
        // The change was kept; read everything again so the list and the buttons agree.
        try {
          await refresh();
        } catch (e) {
          error = message(e, c.loadFailed);
        }
        box.checked = states.find((x) => x.id === s.id)?.included ?? s.included;
      }
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

  /** "Find the next watermark": a saved watermark is looked for first, as on Add photos. */
  async function findNext() {
    if (held || next !== null) return;
    const ask = ++asked;
    error = null;
    next = { kind: 'checking' };
    setBusy(true);
    const call = suggestSaved();
    checkCall = call;
    let suggestion: Suggestion | null = null;
    try {
      suggestion = await call;
    } catch {
      // The offer is a bonus; without it the first guess runs.
    }
    if (ask !== asked || leaving) return;
    checkCall = null;
    if (suggestion !== null) {
      setBusy(false);
      next = { kind: 'offer', suggestion };
    } else {
      next = { kind: 'finding' };
    }
  }

  /** Back while the saved watermarks are checked: the check stops, and the grid shows again. */
  async function stopChecking() {
    asked++;
    const call = checkCall;
    checkCall = null;
    if (call !== null) await stopJob(call);
    backToGrid();
  }

  function backToGrid() {
    next = null;
    setBusy(false);
  }

  function guessNext() {
    next = { kind: 'finding' };
    setBusy(true);
  }

  /** "Yes" on the offer: the next group comes from the saved watermark, matched right here. */
  async function useOffered(suggestion: Suggestion) {
    if (using) return;
    using = true;
    setBusy(true);
    try {
      await startFromSaved(wizard, suggestion.id);
    } catch (e) {
      next = null;
      error = message(e, copy.saved.useFailed);
      return;
    } finally {
      using = false;
      setBusy(false);
    }
    next = null;
    await start();
  }

  function foundNext(outcome: DetectOutcome) {
    wizard.startDraft(outcome);
    backToGrid();
    wizard.go('check');
  }

  function chooseSaved() {
    wizard.savedMode = 'choose';
    wizard.go('saved');
  }

  function removeAll() {
    asking = false;
    wizard.go('remove');
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
    {#if current !== null && current >= 2}
      <h2>{g.checkAllHeading(current)}</h2>
    {/if}
    {#if next !== null}
      {#if next.kind === 'checking'}
        <Waiting sentence={copy.addPhotos.checkingSaved} fraction={null} onCancel={stopChecking} cancelLabel={copy.common.back} />
      {:else if next.kind === 'offer'}
        {@const suggestion = next.suggestion}
        <div class="offer" role="status">
          <p>{copy.addPhotos.offer(suggestion.name)}</p>
          <div class="buttons">
            <button type="button" class="primary" disabled={using} onclick={() => useOffered(suggestion)}>
              {copy.addPhotos.yes}
            </button>
            <button type="button" disabled={using} onclick={guessNext}>{copy.addPhotos.no}</button>
            <button type="button" disabled={using} onclick={backToGrid}>{copy.common.back}</button>
          </div>
        </div>
      {:else}
        <Finding first={copy.finding.neutral} run={detect} onDone={foundNext} onBack={backToGrid} />
      {/if}
    {:else if matching}
      <Waiting
        sentence={c.finding(done, total)}
        fraction={total > 0 ? done / total : null}
        onCancel={back}
        cancelLabel={copy.common.back}
      />
    {:else if !loaded}
      <p class="summary">{c.loading}</p>
      <footer>
        <button type="button" disabled={backing} onclick={back}>{copy.common.back}</button>
      </footer>
    {:else if states.length === 0}
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {:else}
        <p class="summary">{c.empty}</p>
      {/if}
      <footer>
        <button type="button" disabled={backing} onclick={back}>{copy.common.back}</button>
      </footer>
    {:else}
      <p class="summary">{c.summary(found, mine.length, needLook)}</p>
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {/if}
      <ul class="grid" aria-label={c.list}>
        {#each states as s (s.id)}
          {@const name = names.get(s.id) ?? ''}
          {@const unreadable = s.status === 'unreadable'}
          {@const earlier = s.member && s.group !== null && s.group !== current}
          {@const ours = !earlier && current !== null && s.group === current}
          {@const loose = !unreadable && !earlier && !ours}
          {@const kind = earlier ? (s.status === 'edited' ? 'edited' : null) : ours ? badge(s) : null}
          {@const locked = unreadable || (loose && !s.included) || (!s.included && !s.hasArea)}
          {@const reason = unreadable
            ? copy.photoProblem[problems.get(s.id) ?? 'damaged']
            : loose
              ? g.notInGroup
              : c.noAreaTip}
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
              {#if s.status === 'pending'}
                <span class="checking" role="status"><span class="spinner" aria-hidden="true"></span>{c.checking}</span>
              {/if}
              {#if earlier && s.group !== null}
                <span class="group">{g.label(s.group)}</span>
              {:else if !unreadable && !s.member}
                <span class="group none">{g.noGroup}</span>
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
              disabled={locked}
              title={locked ? reason : undefined}
              onchange={(e) => tick(s, e.currentTarget)}
            />
            {#if locked}
              <span class="why">{reason}</span>
            {/if}
          </li>
        {/each}
      </ul>
      {#if groups !== null && (leftovers > 0 || groups.unreadable > 0)}
        <p class="line">{g.line(groups.groups, leftovers, groups.unreadable)}</p>
      {/if}
      <footer>
        <button type="button" disabled={backing} onclick={back}>{copy.common.back}</button>
        <div class="actions">
          <span class="hint" aria-live="polite">{blocked ? c.needLookHint : ''}</span>
          {#if leftovers > 0}
            <button type="button" disabled={held || members === 0} onclick={() => (asking = true)}>
              {c.removeFrom(members)}
            </button>
            <button type="button" disabled={held || members === 0} onclick={() => wizard.go('remove')}>
              {g.leaveOut}
            </button>
            <button type="button" disabled={held} onclick={chooseSaved}>{g.useSaved}</button>
            <button type="button" class="primary" disabled={held} onclick={findNext}>
              {g.findNext(leftovers)}
            </button>
          {:else}
            <button type="button" class="primary" disabled={held || members === 0} onclick={() => wizard.go('remove')}>
              {c.removeFrom(members)}
            </button>
          {/if}
        </div>
      </footer>
    {/if}
  </section>
</div>

{#if asking}
  <Dialog title={c.removeFrom(members)} onClose={() => (asking = false)}>
    <p>{g.confirmLeftovers(leftovers, members)}</p>
    {#snippet actions()}
      <button type="button" class="primary" disabled={held || members === 0} onclick={removeAll}>{g.confirmRemove}</button>
      <button type="button" onclick={() => (asking = false)}>{copy.common.back}</button>
    {/snippet}
  </Dialog>
{/if}

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
  h2 {
    margin: 0;
    font-size: var(--text-large);
    font-weight: 600;
  }
  .summary,
  .line,
  .error,
  .offer p {
    margin: 0;
  }
  .offer,
  .offer .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .offer {
    padding: var(--space-6) 0;
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
  .checking {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    background: var(--surface);
    color: var(--text);
    font-size: var(--text-small);
  }
  .spinner {
    width: 0.9rem;
    height: 0.9rem;
    border: 2px solid var(--track);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
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
  /* The group a tile is in, or that it is in none yet: top right, clear of the tick. */
  .group {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: 1px var(--space-2);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    font-size: var(--text-small);
    font-weight: 600;
  }
  .group.none {
    background: var(--track);
    color: var(--text-muted);
    font-weight: 400;
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
