<script lang="ts">
  import { onMount } from 'svelte';
  import { deleteSaved, imageUrl, listSaved, renameSaved } from '../lib/api';
  import Dialog from '../lib/components/Dialog.svelte';
  import { copy } from '../lib/copy';
  import type { SavedInfo, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let list: SavedInfo[] | null = $state(null);
  let error: string | null = $state(null);
  /** The watermark being renamed, and the name typed so far. */
  let editing: string | null = $state(null);
  let draft = $state('');
  let renameError: string | null = $state(null);
  let deleting: SavedInfo | null = $state(null);
  let deleteError: string | null = $state(null);
  let working = $state(false);

  const messageOf = (e: unknown, fallback: string) => (e as Partial<UserError> | null)?.message || fallback;

  async function refresh() {
    try {
      list = await listSaved();
    } catch (e) {
      error = messageOf(e, copy.saved.loadFailed);
    }
  }

  onMount(refresh);

  function date(created: string): string {
    const d = new Date(created);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(undefined, { dateStyle: 'medium' });
  }

  function startRename(w: SavedInfo) {
    editing = w.id;
    draft = w.name;
    renameError = null;
  }

  function stopRename() {
    editing = null;
    renameError = null;
  }

  async function rename(w: SavedInfo) {
    const name = draft.trim();
    if (name === '' || name === w.name) {
      stopRename();
      return;
    }
    working = true;
    try {
      await renameSaved(w.id, name);
    } catch (e) {
      renameError = messageOf(e, copy.saved.renameFailed);
      return;
    } finally {
      working = false;
    }
    stopRename();
    await refresh();
  }

  function keydown(e: KeyboardEvent, w: SavedInfo) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void rename(w);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      stopRename();
    }
  }

  async function remove() {
    if (deleting === null) return;
    working = true;
    deleteError = null;
    try {
      await deleteSaved(deleting.id);
    } catch (e) {
      deleteError = messageOf(e, copy.saved.deleteFailed);
      return;
    } finally {
      working = false;
    }
    deleting = null;
    await refresh();
  }

  function closeDelete() {
    if (working) return;
    deleting = null;
    deleteError = null;
  }

  const focus = (el: HTMLElement) => el.focus();
</script>

<section class="saved">
  <div class="top">
    <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
    <h1>{copy.saved.title}</h1>
  </div>

  {#if error !== null}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if list !== null && list.length === 0}
    <p class="empty">{copy.saved.empty}</p>
  {:else if list !== null}
    <ul>
      {#each list as w (w.id)}
        <li>
          <img src={imageUrl('saved', w.id)} alt={w.name} />
          <div class="text">
            {#if editing === w.id}
              <div class="rename">
                <input
                  type="text"
                  aria-label={copy.saved.newName(w.name)}
                  bind:value={draft}
                  onkeydown={(e) => keydown(e, w)}
                  disabled={working}
                  {@attach focus}
                />
                <button type="button" class="primary" disabled={working} onclick={() => rename(w)}>
                  {copy.saved.save}
                </button>
                <button type="button" disabled={working} onclick={stopRename}>{copy.common.cancel}</button>
              </div>
              {#if renameError !== null}
                <p class="error" role="alert">{renameError}</p>
              {/if}
            {:else}
              <span class="name">{w.name}</span>
            {/if}
            <span class="date">{copy.saved.savedOn(date(w.created))}</span>
          </div>
          {#if editing !== w.id}
            <div class="buttons">
              <button type="button" aria-label={copy.saved.renameOne(w.name)} onclick={() => startRename(w)}>
                {copy.saved.rename}
              </button>
              <button
                type="button"
                aria-label={copy.saved.deleteOne(w.name)}
                aria-haspopup="dialog"
                onclick={() => {
                  deleteError = null;
                  deleting = w;
                }}
              >
                {copy.saved.delete}
              </button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if deleting !== null}
  <Dialog title={copy.saved.deleteTitle(deleting.name)} onClose={closeDelete}>
    <p>{copy.saved.deleteBody}</p>
    {#if deleteError !== null}
      <p class="error" role="alert">{deleteError}</p>
    {/if}
    {#snippet actions()}
      <button type="button" class="danger" disabled={working} onclick={remove}>{copy.saved.delete}</button>
      <button type="button" disabled={working} onclick={closeDelete}>{copy.common.cancel}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .saved {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 44rem;
    padding: var(--space-5);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .top h1 {
    margin: 0;
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  img {
    flex: none;
    width: 96px;
    height: 64px;
    object-fit: contain;
    border-radius: var(--radius);
    background: var(--track);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .date,
  .empty {
    color: var(--text-muted);
  }
  .date {
    font-size: var(--text-small);
  }
  .rename,
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  input {
    flex: 1;
    min-width: 10rem;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  p {
    margin: 0;
  }
  .error {
    color: var(--danger);
  }
</style>
