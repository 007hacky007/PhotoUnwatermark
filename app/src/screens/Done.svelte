<script module lang="ts">
  /** The number in the result URLs (`result/<id>?v=<n>`); it grows with every visit. */
  let visits = 0;
</script>

<script lang="ts">
  // Step 5, "Done": how many photos were saved and where, a before/after view of each result
  // (arrow buttons, or Left and Right), the photos that could not be finished, and the ways on:
  // open the folder, save the watermark for the next set, start a new set.
  import { onMount } from 'svelte';
  import { clearPhotos, defaultWatermarkName, groupsState, imageUrl, revealItemInDir, saveWatermark } from '../lib/api';
  import BeforeAfter from '../lib/components/BeforeAfter.svelte';
  import { copy } from '../lib/copy';
  import type { RemoveReport, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  const c = copy.done;
  const EMPTY: RemoveReport = { done: [], failed: [], leftOut: [], cancelled: false };
  // svelte-ignore state_referenced_locally
  const report = wizard.report ?? EMPTY;
  const version = ++visits;
  const fieldId = $props.id();

  /** One save row per group that can be saved. */
  interface Row {
    group: number;
    name: string;
    saving: boolean;
    error: string | null;
  }
  let rows: Row[] = $state([]);
  /** More than one group was made, so names and labels say which. */
  let several = $state(false);
  /** "Photo X of Y" for screen readers, said when the person browses. */
  let announced = $state('');
  let error: string | null = $state(null);
  let starting = $state(false);

  const count = report.done.length;
  // The position and "Saved" live in the wizard, so they stay when Settings is visited.
  const index = $derived(Math.min(wizard.resultIndex, Math.max(0, count - 1)));
  const shown = $derived(report.done[index]);

  /** The parts of a path, for either kind of separator. */
  const parts = (path: string) => path.split(/[\\/]/).filter((p) => p !== '');
  const fileName = (path: string) => parts(path).at(-1) ?? path;
  /** The folder the results went to: the name of the first result's folder. */
  const folder = count > 0 ? (parts(report.done[0].output).at(-2) ?? '') : '';

  const headline = report.cancelled ? c.stopped(count) : count > 0 ? c.saved(count, folder) : c.nothingSaved;

  function message(e: unknown, fallback: string): string {
    const text = (e as Partial<UserError> | null)?.message;
    return typeof text === 'string' && text !== '' ? text : fallback;
  }

  onMount(() => {
    void loadRows();
  });

  async function loadRows() {
    const base = await defaultWatermarkName().catch(() => '');
    try {
      const { groups } = await groupsState();
      several = groups.length > 1;
      rows = groups
        .filter((g) => g.canSave)
        .map((g) => ({
          group: g.number,
          name:
            g.savedName !== null && g.edited
              ? c.editedName(g.savedName)
              : base !== '' && several
                ? c.groupDefault(base, g.number)
                : base,
          saving: false,
          error: null,
        }));
    } catch {
      // No rows, so nothing to save.
    }
  }

  const update = (group: number, change: Partial<Row>) => {
    rows = rows.map((r) => (r.group === group ? { ...r, ...change } : r));
  };

  function browse(step: number) {
    const next = Math.min(count - 1, Math.max(0, index + step));
    if (next === index) return;
    wizard.resultIndex = next;
    announced = c.position(next + 1, count);
  }

  function keydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || count === 0) return;
    const target = e.target;
    if (target instanceof Element && target.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (e.key === 'ArrowLeft') browse(-1);
    else if (e.key === 'ArrowRight') browse(1);
    else return;
    e.preventDefault();
  }

  async function openFolder() {
    error = null;
    try {
      await revealItemInDir(report.done[0].output);
    } catch (e) {
      error = message(e, c.openFailed);
    }
  }

  async function save(e: SubmitEvent, row: Row) {
    e.preventDefault();
    const chosen = row.name.trim();
    if (chosen === '') {
      update(row.group, { error: copy.saved.emptyName });
      return;
    }
    update(row.group, { error: null, saving: true });
    try {
      await saveWatermark(row.group, chosen);
      if (!wizard.savedGroups.includes(row.group)) wizard.savedGroups = [...wizard.savedGroups, row.group];
      update(row.group, { saving: false });
    } catch (err) {
      update(row.group, { saving: false, error: message(err, c.saveFailed) });
    }
  }

  async function newSet() {
    error = null;
    starting = true;
    try {
      await clearPhotos();
    } catch (e) {
      error = message(e, c.newSetFailed);
      starting = false;
      return;
    }
    wizard.restart();
  }
</script>

<svelte:window onkeydown={keydown} />

<section class="done">
  <h1>{headline}</h1>

  {#if shown}
    <div class="browse" role="group" aria-label={c.results}>
      <div class="compare">
        <BeforeAfter before={imageUrl('original', shown.id)} after={imageUrl('result', shown.id, version)} />
      </div>
      <div class="nav">
        <button type="button" aria-label={c.previous} title={c.previous} disabled={index === 0} onclick={() => browse(-1)}>
          <span aria-hidden="true">{c.previousGlyph}</span>
        </button>
        <span class="where">
          <span>{c.position(index + 1, count)}</span>
          <span class="file">{fileName(shown.output)}</span>
        </span>
        <span class="visually-hidden" data-testid="announce" aria-live="polite">{announced}</span>
        <button
          type="button"
          aria-label={c.next}
          title={c.next}
          disabled={index === count - 1}
          onclick={() => browse(1)}
        >
          <span aria-hidden="true">{c.nextGlyph}</span>
        </button>
      </div>
    </div>
  {/if}

  {#if error !== null}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if report.failed.length > 0}
    <div class="failed">
      <p>{c.failedTitle(report.failed.length)}</p>
      <ul>
        {#each report.failed as f (f.id)}
          <li>{c.failedItem(f.name, f.reason)}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if report.leftOut.length > 0}
    <div class="failed">
      <p>{c.leftOut(report.leftOut.length)}</p>
      <ul>
        {#each report.leftOut as l (l.id)}
          <li>{l.name}</li>
        {/each}
      </ul>
    </div>
  {/if}

  <div class="keep">
    {#each rows as row (row.group)}
      {#if wizard.savedGroups.includes(row.group)}
        <p class="saved" role="status">{c.savedWatermark}</p>
      {:else}
        <form onsubmit={(e) => save(e, row)}>
          <label for="{fieldId}-name-{row.group}">{several ? c.groupName(row.group) : c.name}</label>
          <div class="row">
            <input
              id="{fieldId}-name-{row.group}"
              type="text"
              value={row.name}
              oninput={(e) => update(row.group, { name: e.currentTarget.value })}
              disabled={row.saving}
            />
            <button type="submit" disabled={row.saving}>{c.saveWatermark}</button>
          </div>
          {#if row.error !== null}
            <p class="error" role="alert">{row.error}</p>
          {/if}
        </form>
      {/if}
    {/each}
  </div>

  <footer>
    {#if count > 0}
      <button type="button" onclick={openFolder}>{c.openFolder}</button>
    {:else}
      <span></span>
    {/if}
    <button type="button" class="primary" disabled={starting} onclick={newSet}>{c.startNewSet}</button>
  </footer>
</section>

<style>
  .done {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: min(56rem, 100%);
    margin: 0 auto;
    padding: var(--space-5) var(--space-5) 0;
  }
  h1 {
    margin: 0;
  }
  .browse {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
  }
  .compare {
    --compare-max-height: 50vh;
    display: flex;
    justify-content: center;
    max-width: 100%;
  }
  .nav {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .where {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 10rem;
  }
  .file {
    color: var(--text-muted);
    font-size: var(--text-small);
    overflow-wrap: anywhere;
  }
  .failed p,
  .failed ul {
    margin: 0;
  }
  .failed ul {
    padding-left: var(--space-5);
  }
  .keep {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  input {
    flex: 1;
    min-width: 12rem;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }
  input:focus-visible {
    outline: none;
    box-shadow: var(--focus);
  }
  .saved,
  .error {
    margin: 0;
  }
  .error {
    color: var(--danger);
  }
  footer {
    position: sticky;
    bottom: 0;
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    background: var(--bg);
  }
</style>
