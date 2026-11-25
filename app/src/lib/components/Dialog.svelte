<script lang="ts">
  // An in-app modal: takes the focus when it opens (and gives it back when it closes), keeps
  // Tab inside, and closes on Escape.
  import { onMount, type Snippet } from 'svelte';

  type Props = {
    title: string;
    onClose: () => void;
    children: Snippet;
    /** The buttons at the bottom. */
    actions: Snippet;
  };

  let { title, onClose, children, actions }: Props = $props();

  const id = $props.id();
  let box: HTMLDivElement;

  const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';

  onMount(() => {
    const before = document.activeElement;
    box.focus();
    return () => {
      if (before instanceof HTMLElement && before.isConnected) before.focus();
    };
  });

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key !== 'Tab') return;
    const items = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) {
      e.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const inside = box.contains(document.activeElement);
    if (e.shiftKey && (!inside || document.activeElement === first || document.activeElement === box)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (!inside || document.activeElement === last)) {
      e.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop">
  <div bind:this={box} class="dialog" role="dialog" aria-modal="true" aria-labelledby="{id}-title" tabindex="-1">
    <h2 id="{id}-title">{title}</h2>
    {@render children()}
    <div class="actions">
      {@render actions()}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4);
    background: rgb(0 0 0 / 40%);
  }
  .dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: min(30rem, 100%);
    max-height: 100%;
    overflow: auto;
    padding: var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .dialog:focus {
    outline: none;
  }
  h2 {
    margin: 0;
    font-size: var(--text-large);
    font-weight: 600;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
