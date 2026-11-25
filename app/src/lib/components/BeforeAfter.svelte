<script lang="ts">
  // Two pictures of the same size on top of each other, split by a handle: "before" on the left,
  // "after" on the right. Drag anywhere on the pictures, or focus the handle and use the arrow
  // keys (Shift for bigger steps, Home and End for the ends).
  import { copy } from '../copy';

  type Props = {
    before: string;
    after: string;
    beforeLabel?: string;
    afterLabel?: string;
  };

  let { before, after, beforeLabel = copy.beforeAfter.before, afterLabel = copy.beforeAfter.after }: Props = $props();

  /** Where the handle is, in percent from the left. */
  let position = $state(50);
  let box: HTMLDivElement;
  let handle: HTMLDivElement;
  let dragging = false;

  const clamp = (v: number) => Math.min(100, Math.max(0, v));

  function follow(e: PointerEvent) {
    const r = box.getBoundingClientRect();
    if (r.width > 0) position = clamp(((e.clientX - r.left) / r.width) * 100);
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    box.setPointerCapture?.(e.pointerId);
    handle.focus({ preventScroll: true });
    follow(e);
  }

  function move(e: PointerEvent) {
    if (dragging) follow(e);
  }

  function keydown(e: KeyboardEvent) {
    const step = e.shiftKey ? 10 : 2;
    const next: Record<string, number> = {
      ArrowLeft: position - step,
      ArrowDown: position - step,
      ArrowRight: position + step,
      ArrowUp: position + step,
      Home: 0,
      End: 100,
    };
    if (!(e.key in next)) return;
    e.preventDefault();
    // The handle has the keys; a page that also uses arrows (browsing photos) must not see them.
    e.stopPropagation();
    position = clamp(next[e.key]);
  }
</script>

<!-- Dragging on the pictures is a shortcut for the handle, which keyboard users focus instead. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={box}
  class="compare"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={() => (dragging = false)}
  onpointercancel={() => (dragging = false)}
>
  <img class="after" src={after} alt={afterLabel} draggable="false" />
  <img class="before" src={before} alt={beforeLabel} draggable="false" style:clip-path="inset(0 {100 - position}% 0 0)" />
  <span class="label left" aria-hidden="true">{beforeLabel}</span>
  <span class="label right" aria-hidden="true">{afterLabel}</span>
  <div
    bind:this={handle}
    class="handle"
    role="slider"
    tabindex="0"
    aria-label={copy.beforeAfter.divider}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(position)}
    style:left="{position}%"
    onkeydown={keydown}
  >
    <span class="knob"></span>
  </div>
</div>

<style>
  .compare {
    position: relative;
    display: inline-block;
    max-width: 100%;
    overflow: hidden;
    border-radius: var(--radius);
    cursor: ew-resize;
    touch-action: none;
    user-select: none;
  }
  img {
    display: block;
  }
  .after {
    max-width: 100%;
    max-height: var(--compare-max-height, 60vh);
  }
  .before {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .label {
    position: absolute;
    top: var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius);
    background: rgb(0 0 0 / 55%);
    color: #fff;
    font-size: var(--text-small);
    pointer-events: none;
  }
  .left {
    left: var(--space-2);
  }
  .right {
    right: var(--space-2);
  }
  .handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: #fff;
    box-shadow: 0 0 0 1px rgb(0 0 0 / 40%);
    transform: translateX(-50%);
  }
  .handle:focus-visible {
    outline: none;
    box-shadow: var(--focus);
  }
  .knob {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 28px;
    height: 28px;
    border: 2px solid #fff;
    border-radius: 50%;
    background: rgb(0 0 0 / 45%);
    transform: translate(-50%, -50%);
  }
</style>
