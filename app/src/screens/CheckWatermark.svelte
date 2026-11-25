<script module lang="ts">
  import type { Area } from '../lib/editor/paint';
  import type { DetectOutcome, PhotoId } from '../lib/types';

  /**
   * Each set's edits, per photo, kept while the app runs: switching photos in the strip, or
   * leaving step 2 and coming back, keeps them. A new detection is a new outcome, so it starts
   * from the app's areas again.
   */
  const editsBySet = new WeakMap<DetectOutcome, Map<PhotoId, Area>>();

  /**
   * The number in each preview's URL (`preview/<id>?v=<n>`). It only grows, across every mount,
   * so a new preview never reuses the URL of a picture the page already loaded.
   */
  let previews = 0;

  function editsFor(outcome: DetectOutcome): Map<PhotoId, Area> {
    let edits = editsBySet.get(outcome);
    if (!edits) editsBySet.set(outcome, (edits = new Map()));
    return edits;
  }
</script>

<script lang="ts">
  // Step 2, "Check the watermark": the editor on one photo of the sample, a strip to switch
  // photos, a preview of the removal on this photo, and "Looks right →". With mode 'photo' it
  // is step 3's editor for a single photo: no strip, and "Save this photo" keeps the area for
  // that photo alone. The app only ever sees the area through putDraft, sent right before each
  // command that reads it.
  import { onMount, tick } from 'svelte';
  import {
    cancelJob,
    confirmSetArea,
    dropDraft,
    editSize,
    getArea,
    imageUrl,
    openPhoto,
    preview,
    putDraft,
    savePhotoArea,
  } from '../lib/api';
  import BeforeAfter from '../lib/components/BeforeAfter.svelte';
  import DeviceChip from '../lib/components/DeviceChip.svelte';
  import Waiting from '../lib/components/Waiting.svelte';
  import { copy } from '../lib/copy';
  import Editor from '../lib/editor/Editor.svelte';
  import { keepFocus } from '../lib/editor/keys';
  import { isEmpty } from '../lib/editor/paint';
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
  const outcome = single ? null : wizard.outcome;
  const sample: PhotoId[] = outcome
    ? outcome.sample.includes(outcome.representative)
      ? [...outcome.sample]
      : [outcome.representative, ...outcome.sample]
    : [];
  const edits = outcome ? editsFor(outcome) : new Map<PhotoId, Area>();

  // svelte-ignore state_referenced_locally
  let current: PhotoId | null = $state(single ? (photoId ?? null) : (outcome?.representative ?? null));
  let editor: Editor | undefined = $state();
  let loaded = $state(false);
  let hint = $state(false);
  let error: string | null = $state(null);
  /** The error was files the app downloaded that are missing: offer the download. */
  let missingData = $state(false);
  /** The photo whose area could not be read; "Try again" reads it again. */
  let failed: PhotoId | null = $state(null);
  /** edit: painting; previewing: the preview runs; preview: before/after shown. */
  let phase: 'edit' | 'previewing' | 'preview' = $state('edit');
  /** True while "Looks right →" or "Save this photo" waits for the app. */
  let saving = $state(false);
  let sentence: string = $state(c.trying);
  let fraction: number | null = $state(null);
  let version = $state(0);
  let chip = $state(0);
  /** Counts photo loads and previews, so an answer that arrives too late is dropped. */
  let shown = 0;
  let job = 0;
  let busy = false;

  onMount(() => {
    if (current !== null) {
      if (single) openPhoto(current).catch((e) => (error = message(e, c.photoFailed)));
      void show(current);
    }
    return () => {
      shown++;
      if (phase === 'previewing') {
        job++;
        cancelJob().catch(() => {});
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
    const [size, bytes] = await Promise.all([editSize(id), getArea(id)]);
    if (bytes.length !== size.width * size.height) {
      throw { message: c.areaFailed, details: `${bytes.length} bytes for ${size.width}x${size.height}` };
    }
    // The app sends 0 or 255; the editor relies on exactly 255 for "in the area".
    for (let i = 0; i < bytes.length; i++) if (bytes[i] !== 0) bytes[i] = 255;
    return { w: size.width, h: size.height, data: bytes };
  }

  async function show(id: PhotoId) {
    const mine = ++shown;
    current = id;
    loaded = false;
    hint = false;
    error = null;
    failed = null;
    try {
      const area = edits.get(id) ?? (await loadArea(id));
      edits.set(id, area);
      // The editor takes the new photo first, then its area.
      await tick();
      if (mine !== shown || !editor) return;
      editor.setArea(area);
      loaded = true;
    } catch (e) {
      if (mine !== shown) return;
      error = message(e, c.areaFailed);
      failed = id;
    }
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
      await preview(id, onEvent);
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
    cancelJob().catch(() => {
      // The preview reports how it ended; its answer is dropped either way.
    });
  }

  async function finish() {
    const id = current;
    const area = areaToSend(single);
    if (id === null || !area) return;
    error = null;
    saving = true;
    setBusy(true);
    try {
      await putDraft(id, area.w, area.h, area.data);
      if (single) await savePhotoArea(id);
      else await confirmSetArea(id);
    } catch (e) {
      error = message(e, single ? c.saveFailed : c.confirmFailed);
      return;
    } finally {
      saving = false;
      setBusy(false);
    }
    if (single) close(true);
    else wizard.go('checkAll');
  }

  function close(saved: boolean) {
    if (onDone) onDone(saved);
    else wizard.go('checkAll');
  }

  function back() {
    if (single) {
      // Nothing of this visit is kept: the area sent for a preview goes too.
      const id = current;
      if (id !== null) {
        Promise.resolve()
          .then(() => dropDraft(id))
          .catch(() => {});
      }
      close(false);
    } else wizard.back();
  }

  function downloadAgain() {
    setBusy(false);
    wizard.go('welcome');
  }

  const editing = $derived(phase === 'edit');
</script>

<section class="check">
  <p class="help">{single ? c.photoHelp : c.help}</p>

  {#if error !== null}
    <div class="problem">
      <p class="error" role="alert">{error}</p>
      {#if failed !== null}
        {@const again = failed}
        <button type="button" onmousedown={keepFocus} onclick={() => show(again)}>{c.tryAgain}</button>
      {/if}
      {#if missingData}
        <button type="button" onmousedown={keepFocus} onclick={downloadAgain}>{copy.common.downloadAgain}</button>
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
          <button type="button" disabled={!loaded || saving} onmousedown={keepFocus} onclick={runPreview}
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
  .help,
  .error,
  .hint {
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
