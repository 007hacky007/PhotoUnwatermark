<script module lang="ts">
  import { SharedDrawing } from '../lib/editor/shared';
  import type { DetectOutcome, PhotoId, StripPlacement } from '../lib/types';

  /** A group's step 2: its one drawing, and where that drawing was placed on each strip photo at which revision. */
  type Strip = { shared: SharedDrawing; placed: Map<PhotoId, { p: StripPlacement; revision: number }> };

  /**
   * Each group's step 2, kept as long as the group's outcome: switching photos in the strip,
   * or leaving step 2 and coming back, keeps the drawing and its undo history. A new detection
   * is a new outcome, so it starts from the app's guess again.
   */
  const strips = new WeakMap<DetectOutcome, Strip>();

  /**
   * The number in each preview's URL (`preview/<id>?v=<n>`). It only grows, across every mount,
   * so a new preview never reuses the URL of a picture the page already loaded.
   */
  let previews = 0;
</script>

<script lang="ts">
  // Step 2, "Check the watermark": the editor on one photo of the sample, a strip to switch
  // photos, a preview of the removal on this photo, and "Looks right →". The group has one
  // drawing, kept at the reference photo's size: each strip photo shows it where the app finds
  // it there, and strokes made there are carried back into it. With mode 'photo' it is step 3's
  // editor for a single photo: no strip, and "Save this photo" keeps the area for that photo
  // alone. The app only ever sees an area through putDraft, sent right before each command
  // that reads it.
  import { onMount, tick } from 'svelte';
  import {
    clearGroups,
    confirmNewGroup,
    confirmSetArea,
    dropDraft,
    editSize,
    getArea,
    getStartArea,
    imageUrl,
    openPhoto,
    placeOn,
    preview,
    putDraft,
    savePhotoArea,
  } from '../lib/api';
  import BeforeAfter from '../lib/components/BeforeAfter.svelte';
  import DeviceChip from '../lib/components/DeviceChip.svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import { stopJob } from '../lib/jobs.svelte';
  import Editor from '../lib/editor/Editor.svelte';
  import { keepFocus } from '../lib/editor/keys';
  import { isEmpty, type Area, type Rect } from '../lib/editor/paint';
  import { IDENTITY, type OuterHistory } from '../lib/editor/shared';
  import type { ProgressEvent, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  type Props = {
    wizard: Wizard;
    mode?: 'set' | 'photo';
    /** The photo to edit in 'photo' mode. */
    photoId?: PhotoId;
    /** 'photo' mode: called after "Save this photo" (true) or Back (false, nothing kept). */
    onDone?: (saved: boolean) => void;
  };

  let { wizard, mode = 'set', photoId, onDone }: Props = $props();

  const c = copy.checkWatermark;
  // The screen is made for one set or one photo; a new one mounts a new screen.
  // svelte-ignore state_referenced_locally
  const single = mode === 'photo';
  // svelte-ignore state_referenced_locally
  const outcome = single ? null : (wizard.drawing?.outcome ?? null);
  // svelte-ignore state_referenced_locally
  const group = single ? 0 : (wizard.drawing?.group ?? 0);
  const sample: PhotoId[] = outcome
    ? outcome.sample.includes(outcome.representative)
      ? [...outcome.sample]
      : [outcome.representative, ...outcome.sample]
    : [];
  /** The group's drawing, once read (not reactive: the editor asks for it through `outer`). */
  let strip: Strip | null = outcome ? (strips.get(outcome) ?? null) : null;
  let loadingStrip: Promise<Strip> | null = null;

  // svelte-ignore state_referenced_locally
  let current: PhotoId | null = $state(single ? (photoId ?? null) : (outcome?.representative ?? null));
  let editor: Editor | undefined = $state();
  /** The photo is shown (or, on a strip photo, its placement failed and the note says so). */
  let loaded = $state(false);
  /**
   * How the drawing shows on the photo on screen: its placement's status, 'empty' for a strip
   * photo while the drawing is empty, null until known.
   */
  let kind: StripPlacement['status'] | 'empty' | null = $state(null);
  /** The note over a strip photo. */
  let note: 'placing' | 'unsure' | 'notFound' | 'placeFailed' | null = $state(null);
  let hint = $state(false);
  let error: string | null = $state(null);
  /** The error was files the app downloaded that are missing: offer the download. */
  let missingData = $state(false);
  /** The photo whose area or placement could not be read; "Try again" reads it again. */
  let failed: PhotoId | null = $state(null);
  /** edit: painting; previewing: the preview runs; preview: before/after shown. */
  let phase: 'edit' | 'previewing' | 'preview' = $state('edit');
  /** True while "Looks right →", "Save this photo" or Back waits for the app. */
  let saving = $state(false);
  let sentence: string = $state(c.trying);
  let fraction: number | null = $state(null);
  let version = $state(0);
  let chip = $state(0);
  /** Counts photo loads and previews, so an answer that arrives too late is dropped. */
  let shown = 0;
  let job = 0;
  /** The running preview's call, so a stop names exactly its ticket. */
  let running: Promise<void> | null = null;
  /** The running placement's call: it holds the app's one job, so it is stopped when no longer wanted. */
  let placing: Promise<StripPlacement> | null = null;
  /** The placement of the photo on screen, or null (placing, failed, or an empty drawing). */
  let placement: StripPlacement | null = null;
  let busy = false;

  /** Undo and redo act on the group's drawing, whichever strip photo they are pressed on. */
  const outer: OuterHistory | undefined = single
    ? undefined
    : {
        canUndo: () => strip?.shared.canUndo() ?? false,
        canRedo: () => strip?.shared.canRedo() ?? false,
        undo: () => {
          if (strip?.shared.undo()) drawingChanged();
        },
        redo: () => {
          if (strip?.shared.redo()) drawingChanged();
        },
      };

  onMount(() => {
    if (current !== null) {
      if (single) openPhoto(current).catch((e) => (error = message(e, c.photoFailed)));
      void show(current);
    }
    return () => {
      shown++;
      stopPlacing();
      if (phase === 'previewing') {
        job++;
        if (running !== null) void stopJob(running);
      }
      setBusy(false);
    };
  });

  function message(e: unknown, fallback: string): string {
    missingData = (e as Partial<UserError> | null)?.missingData === true;
    const text = (e as Partial<UserError> | null)?.message;
    return typeof text === 'string' && text !== '' ? text : fallback;
  }

  function setBusy(on: boolean) {
    if (busy === on) return;
    busy = on;
    wizard.busy = on;
  }

  async function loadArea(id: PhotoId): Promise<Area> {
    // Step 2 starts from the step-2 guess; a single photo shows the area step 3 has for it.
    const [size, bytes] = await Promise.all([editSize(id), single ? getArea(id) : getStartArea(id)]);
    if (bytes.length !== size.width * size.height) {
      throw { message: c.areaFailed, details: `${bytes.length} bytes for ${size.width}x${size.height}` };
    }
    // The app sends 0 or 255; the editor relies on exactly 255 for "in the area".
    for (let i = 0; i < bytes.length; i++) if (bytes[i] !== 0) bytes[i] = 255;
    return { w: size.width, h: size.height, data: bytes };
  }

  /** The group's drawing, read from the reference photo's guess the first time. */
  function loadStrip(from: DetectOutcome): Promise<Strip> {
    if (strip) return Promise.resolve(strip);
    loadingStrip ??= loadArea(from.representative).then(
      (area) => {
        const made: Strip = { shared: new SharedDrawing(area), placed: new Map() };
        strips.set(from, made);
        strip = made;
        return made;
      },
      (e) => {
        loadingStrip = null;
        throw e;
      },
    );
    return loadingStrip;
  }

  /** Stops the placement that runs, if any; its answer is dropped. */
  function stopPlacing() {
    const call = placing;
    placing = null;
    if (call !== null) void stopJob(call);
  }

  function show(id: PhotoId): Promise<void> {
    const mine = ++shown;
    stopPlacing();
    current = id;
    loaded = false;
    kind = null;
    note = null;
    placement = null;
    hint = false;
    error = null;
    failed = null;
    return single ? showPhoto(id, mine) : showOnStrip(id, mine);
  }

  async function showPhoto(id: PhotoId, mine: number) {
    try {
      const area = await loadArea(id);
      // The editor takes the new photo first, then its area.
      await tick();
      if (mine !== shown || !editor) return;
      editor.setArea(area);
      kind = 'found';
      loaded = true;
    } catch (e) {
      if (mine !== shown) return;
      error = message(e, c.areaFailed);
      failed = id;
    }
  }

  async function showOnStrip(id: PhotoId, mine: number) {
    if (!outcome) return;
    const reference = outcome.representative;
    let asking = false;
    try {
      const s = await loadStrip(outcome);
      if (mine !== shown) return;
      let p: StripPlacement | null;
      const known = s.placed.get(id);
      if (id === reference) p = IDENTITY;
      else if (isEmpty(s.shared.area)) p = null;
      else if (known && known.revision === s.shared.revision) p = known.p;
      else {
        // The app places the drawing it was last sent, so it gets the drawing as it is now.
        asking = true;
        note = 'placing';
        const revision = s.shared.revision;
        const a = s.shared.area;
        await putDraft(reference, a.w, a.h, a.data);
        if (mine !== shown) return;
        const call = placeOn(reference, id);
        placing = call;
        p = await call;
        if (placing === call) placing = null;
        if (mine !== shown) return;
        s.placed.set(id, { p, revision });
        asking = false;
      }
      const size = id === reference ? { width: s.shared.area.w, height: s.shared.area.h } : await editSize(id);
      await tick();
      if (mine !== shown || !editor) return;
      placement = p;
      kind = p?.status ?? 'empty';
      note = kind === 'unsure' ? 'unsure' : kind === 'notFound' ? 'notFound' : null;
      editor.setArea(p ? s.shared.viewOn(p, size.width, size.height) : blank(size.width, size.height));
      loaded = true;
    } catch (e) {
      if (mine !== shown) return;
      failed = id;
      if (asking) {
        placing = null;
        note = 'placeFailed';
        // The drawing itself is fine: "Looks right" still works from here.
        loaded = true;
      } else {
        note = null;
        error = message(e, c.areaFailed);
      }
    }
  }

  const blank = (w: number, h: number): Area => ({ w, h, data: new Uint8Array(w * h) });

  /** A stroke on the photo on screen: carried back into the drawing, then shown as the drawing has it. */
  function commit(dirty: Rect) {
    const view = editor?.getArea();
    if (!strip || !placement || !view) return;
    strip.shared.carryBack(placement, view, dirty);
    drawingChanged();
  }

  /** The drawing changed while this photo is on screen: show it again, keeping where it was placed. */
  function drawingChanged() {
    const s = strip;
    const id = current;
    if (!s || id === null || !editor) return;
    if (!placement) {
      // An empty drawing was shown here without a placement: place the changed one.
      if (kind === 'empty') void show(id);
      return;
    }
    const view = editor.getArea();
    if (!view) return;
    if (s.placed.has(id)) s.placed.set(id, { p: placement, revision: s.shared.revision });
    editor.showArea(s.shared.viewOn(placement, view.w, view.h));
  }

  /** The area on screen, or null (with the hint shown) when there is nothing to send. */
  function areaToSend(allowEmpty = false): Area | null {
    const area = loaded ? (editor?.getArea() ?? null) : null;
    if (!area) return null;
    if (!allowEmpty && isEmpty(area)) {
      hint = true;
      return null;
    }
    return area;
  }

  async function runPreview() {
    const id = current;
    const area = areaToSend();
    if (id === null || !area) return;
    const mine = ++job;
    error = null;
    sentence = c.trying;
    fraction = null;
    phase = 'previewing';
    setBusy(true);
    const onEvent = (e: ProgressEvent) => {
      if (mine !== job) return;
      if (e.phase === 'loadingModels') {
        sentence = copy.loading.remover;
        fraction = e.fraction;
      } else if (e.phase === 'preview') {
        sentence = c.trying;
        fraction = e.fraction;
      } else if (e.phase === 'fellBackToProcessor') {
        chip++;
      }
    };
    try {
      await putDraft(id, area.w, area.h, area.data);
      // Stopped while the area was on its way: the preview never starts.
      if (mine !== job) return;
      running = preview(id, onEvent);
      await running;
    } catch (e) {
      if (mine !== job) return;
      setBusy(false);
      phase = 'edit';
      error = message(e, c.previewFailed);
      return;
    }
    if (mine !== job) return;
    setBusy(false);
    version = ++previews;
    chip++;
    phase = 'preview';
  }

  function cancelPreview() {
    job++;
    setBusy(false);
    phase = 'edit';
    // The preview reports how it ended; its answer is dropped either way.
    if (running !== null) void stopJob(running);
  }

  async function finish() {
    if (single) return savePhoto();
    // The group is learned from the reference photo and the whole drawing, whichever photo is on screen.
    const s = strip;
    if (!outcome || !s || !loaded) return;
    if (isEmpty(s.shared.area)) {
      hint = true;
      return;
    }
    const reference = outcome.representative;
    const draft = wizard.draft !== null;
    error = null;
    saving = true;
    setBusy(true);
    try {
      const a = s.shared.area;
      await putDraft(reference, a.w, a.h, a.data);
      if (draft) await confirmNewGroup(reference);
      else await confirmSetArea(reference);
    } catch (e) {
      error = message(e, c.confirmFailed);
      return;
    } finally {
      saving = false;
      setBusy(false);
    }
    wizard.confirmDraft();
    wizard.go('checkAll');
  }

  async function savePhoto() {
    const id = current;
    const area = areaToSend(true);
    if (id === null || !area) return;
    error = null;
    saving = true;
    setBusy(true);
    try {
      await putDraft(id, area.w, area.h, area.data);
      await savePhotoArea(id);
    } catch (e) {
      error = message(e, c.saveFailed);
      return;
    } finally {
      saving = false;
      setBusy(false);
    }
    close(true);
  }

  function close(saved: boolean) {
    if (onDone) onDone(saved);
    else wizard.go('checkAll');
  }

  async function back() {
    if (single) {
      // Nothing of this visit is kept: the area sent for a preview goes too.
      const id = current;
      if (id !== null) {
        Promise.resolve()
          .then(() => dropDraft(id))
          .catch(() => {});
      }
      close(false);
      return;
    }
    // A placement still running is not wanted any more, and it holds the app's one job.
    const interrupted = note === 'placing';
    if (interrupted) {
      shown++;
      stopPlacing();
    }
    // Back from group 1 goes back to the photo list, so no group stays; the drawing does.
    if (wizard.drawing?.group === 1 && wizard.groups > 0) {
      error = null;
      saving = true;
      setBusy(true);
      try {
        await clearGroups();
      } catch (e) {
        // Staying: the photo whose placement was stopped is placed again.
        if (interrupted && current !== null) void show(current);
        error = message(e, copy.checkAll.tickFailed);
        return;
      } finally {
        saving = false;
        setBusy(false);
      }
      wizard.clearGroups();
    }
    wizard.backFromCheck();
  }

  function downloadAgain() {
    setBusy(false);
    wizard.go('welcome');
  }

  const editing = $derived(phase === 'edit');
  /** Preview needs the drawing shown on this photo. */
  const previewable = $derived(loaded && (kind === 'found' || kind === 'unsure'));
  const notes = { placing: c.placing, unsure: c.unsureHere, notFound: c.notFoundHere, placeFailed: c.placeFailed };
</script>

<section class="check">
  {#if group >= 2}
    <h2>{copy.groups.checkHeading(group)}</h2>
  {/if}
  <p class="help">{single ? c.photoHelp : c.help}</p>

  {#if error !== null}
    <div class="problem">
      <p class="error" role="alert">{error}</p>
      {#if failed !== null && note === null}
        {@const again = failed}
        <button type="button" onmousedown={keepFocus} onclick={() => show(again)}>{c.tryAgain}</button>
      {/if}
      {#if missingData}
        <button type="button" onmousedown={keepFocus} onclick={downloadAgain}>{copy.common.downloadAgain}</button>
      {/if}
    </div>
  {/if}

  {#if note !== null && editing}
    <div class="problem">
      <p class="note" role="status">{notes[note]}</p>
      {#if note === 'placeFailed' && failed !== null}
        {@const again = failed}
        <button type="button" onmousedown={keepFocus} onclick={() => show(again)}>{c.tryAgain}</button>
      {/if}
    </div>
  {/if}

  {#if !editing}
    <!-- Above the photo, so it never covers the photo or its labels. -->
    <div class="chip"><DeviceChip refresh={chip} /></div>
  {/if}

  <div class="work">
    {#if current !== null}
      <div class="pane" hidden={!editing}>
        <Editor
          bind:this={editor}
          photoId={current}
          active={editing && loaded && !saving}
          onChange={() => (hint = false)}
          {outer}
          onCommit={single ? undefined : commit}
          paintable={single || kind === 'found'}
        />
      </div>
    {/if}
    {#if phase === 'previewing'}
      <Waiting {sentence} {fraction} onCancel={cancelPreview} />
    {:else if phase === 'preview' && current !== null}
      <div class="result">
        <BeforeAfter before={imageUrl('edit', current)} after={imageUrl('preview', current, version)} />
        <button type="button" disabled={saving} onmousedown={keepFocus} onclick={() => (phase = 'edit')}
          >{c.backToEditing}</button
        >
      </div>
    {/if}
  </div>

  {#if !single && sample.length > 1 && editing}
    <div class="strip" role="group" aria-label={c.strip}>
      {#each sample as id, i (id)}
        <button
          type="button"
          aria-pressed={id === current}
          aria-label={c.stripPhoto(i + 1, sample.length)}
          disabled={saving}
          onmousedown={keepFocus}
          onclick={() => id !== current && show(id)}
        >
          <img src={imageUrl('thumb', id)} alt="" />
        </button>
      {/each}
    </div>
  {/if}

  {#if hint}
    <p class="hint" role="alert">{c.emptyArea}</p>
  {/if}

  {#if phase !== 'previewing'}
    <footer>
      <button type="button" disabled={saving} onmousedown={keepFocus} onclick={back}>{copy.common.back}</button>
      <div class="actions">
        {#if editing}
          <button type="button" disabled={!previewable || saving} onmousedown={keepFocus} onclick={runPreview}
            >{c.preview}</button
          >
        {/if}
        <button
          type="button"
          class="primary"
          disabled={!loaded || saving}
          onmousedown={keepFocus}
          onclick={finish}
        >
          {single ? c.savePhoto : c.looksRight}
        </button>
      </div>
    </footer>
  {/if}
</section>

<style>
  .check {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
    min-height: 0;
    padding: var(--space-4) var(--space-5);
  }
  h2 {
    margin: 0;
    font-size: var(--text-large);
    font-weight: 600;
  }
  .help,
  .error,
  .hint,
  .note {
    margin: 0;
  }
  .error,
  .hint {
    color: var(--danger);
  }
  .work {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 0;
  }
  .pane {
    align-self: stretch;
    flex: 1;
    min-height: 0;
  }
  .pane[hidden] {
    display: none;
  }
  .problem {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .result {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    max-width: 100%;
  }
  .chip {
    display: flex;
    justify-content: flex-end;
    min-height: 1.5rem;
  }
  .strip {
    display: flex;
    gap: var(--space-2);
    overflow-x: auto;
  }
  .strip button {
    flex: none;
    padding: 0;
    overflow: hidden;
    border-width: 2px;
  }
  .strip button[aria-pressed='true'] {
    border-color: var(--accent);
  }
  .strip img {
    display: block;
    width: 64px;
    height: 64px;
    object-fit: cover;
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
