<script lang="ts">
  // The editor's buttons. Every button shows its shortcut on hover. A mouse click does not move
  // the focus onto a button (so Space keeps hiding the red layer instead of pressing it);
  // keyboard users still reach every button with Tab.
  import { copy } from '../copy';
  import { keepFocus as keep } from './keys';
  import { LOUPE_STEPS } from './loupe';
  import { MAX_SIZE, MIN_SIZE, type Tool } from './tools';

  type Props = {
    tool: Tool;
    size: number;
    loupe: boolean;
    magnification: number;
    canUndo: boolean;
    canRedo: boolean;
    /** The current zoom, in percent. */
    zoom: number;
    isMac: boolean;
    onUndo: () => void;
    onRedo: () => void;
    onZoomOut: () => void;
    onZoomIn: () => void;
    onActualSize: () => void;
    onFit: () => void;
  };

  let {
    tool = $bindable(),
    size = $bindable(),
    loupe = $bindable(),
    magnification = $bindable(),
    canUndo,
    canRedo,
    zoom,
    isMac,
    onUndo,
    onRedo,
    onZoomOut,
    onZoomIn,
    onActualSize,
    onFit,
  }: Props = $props();

  const e = copy.editor;
  const k = copy.editor.keys;
  const tools: { id: Tool; name: string; key: string }[] = [
    { id: 'move', name: copy.tools.move, key: k.move },
    { id: 'brush', name: copy.tools.brush, key: k.brush },
    { id: 'eraser', name: copy.tools.eraser, key: k.eraser },
    { id: 'rect', name: copy.tools.rectangle, key: k.rectangle },
  ];
</script>

<div class="toolbar" role="group" aria-label={e.toolbar}>
  <div class="group" role="group" aria-label={e.tools}>
    {#each tools as t (t.id)}
      <button
        type="button"
        aria-pressed={tool === t.id}
        title={e.withKey(t.name, t.key)}
        onmousedown={keep}
        onclick={() => (tool = t.id)}>{t.name}</button
      >
    {/each}
  </div>

  <div class="group size">
    <label>
      <span>{e.size}</span>
      <input
        type="range"
        min={MIN_SIZE}
        max={MAX_SIZE}
        step="1"
        bind:value={size}
        aria-valuetext={e.sizeValue(size)}
        title={e.withKey(e.size, k.size)}
      />
    </label>
    <span class="value">{e.sizeValue(size)}</span>
  </div>

  <div class="group">
    <button type="button" disabled={!canUndo} title={e.withKey(e.undo, k.undo(isMac))} onmousedown={keep} onclick={onUndo}
      >{e.undo}</button
    >
    <button type="button" disabled={!canRedo} title={e.withKey(e.redo, k.redo(isMac))} onmousedown={keep} onclick={onRedo}
      >{e.redo}</button
    >
  </div>

  <div class="group">
    <button
      type="button"
      class="square"
      aria-label={e.zoomOut}
      title={e.withKey(e.zoomOut, k.zoomOut)}
      onmousedown={keep}
      onclick={onZoomOut}>{e.zoomOutGlyph}</button
    >
    <button type="button" title={e.withKey(e.actualSizeHint, k.actualSize)} onmousedown={keep} onclick={onActualSize}
      >{e.actualSize}</button
    >
    <button
      type="button"
      class="square"
      aria-label={e.zoomIn}
      title={e.withKey(e.zoomIn, k.zoomIn)}
      onmousedown={keep}
      onclick={onZoomIn}>{e.zoomInGlyph}</button
    >
    <button type="button" title={e.withKey(e.fitHint, k.fit)} onmousedown={keep} onclick={onFit}>{e.fit}</button>
    <span class="value">{e.zoom(zoom)}</span>
  </div>

  <div class="group">
    <button
      type="button"
      aria-pressed={loupe}
      title={e.withKey(e.magnifier, k.magnifier)}
      onmousedown={keep}
      onclick={() => (loupe = !loupe)}>{e.magnifier}</button
    >
    {#if loupe}
      <div class="group" role="group" aria-label={e.magnification}>
        {#each LOUPE_STEPS as n (n)}
          <button
            type="button"
            class="square"
            aria-pressed={magnification === n}
            onmousedown={keep}
            onclick={() => (magnification = n)}>{e.times(n)}</button
          >
        {/each}
      </div>
    {/if}
  </div>

  <span class="hint">{e.moveHint(isMac)}</span>
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-4);
  }
  .group {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  button {
    padding: var(--space-1) var(--space-3);
  }
  .square {
    min-width: 2.25rem;
    padding: var(--space-1) var(--space-2);
  }
  button[aria-pressed='true'] {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
  .size label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .size input {
    width: 8rem;
    accent-color: var(--accent);
  }
  .value {
    min-width: 3.5rem;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--text-small);
  }
</style>
