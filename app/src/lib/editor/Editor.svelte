<script lang="ts">
  // The watermark area editor. Layers, bottom to top: the photo drawn into a canvas, the red
  // layer (a canvas at edit size), both moved by one CSS transform for zoom and pan; then a
  // canvas the size of the box for the brush outline and the rectangle being dragged; then the
  // magnifier. While painting, only the changed rectangle of the red layer is redrawn (once per
  // frame) and nothing reactive changes, so the page never renders again mid-stroke.
  import { onMount, untrack } from 'svelte';
  import { imageUrl } from '../api';
  import { copy } from '../copy';
  import type { PhotoId } from '../types';
  import { actionFor, isTextField, repeats, type Action } from './keys';
  import Loupe from './Loupe.svelte';
  import { LOUPE_DEFAULT } from './loupe';
  import { outlineBox, outlineCircle } from './outline';
  import { fillRect, stampLine, unionRect, type Area, type Rect } from './paint';
  import { RedLayer } from './redLayer';
  import type { OuterHistory } from './shared';
  import Toolbar from './Toolbar.svelte';
  import { DEFAULT_SIZE, stepSize, type Tool } from './tools';
  import { fit, nextZoomStep, gestureRatio, panBy, prevZoomStep, toImage, toScreen, wheelPan, wheelZoom, zoomAt, type View } from './view';

  type Props = {
    photoId: PhotoId;
    /** Called after each change to the area (a stroke, a rectangle, undo, redo). */
    onChange?: () => void;
    /** False while the editor is covered or busy: its shortcuts and pointer input then do nothing. */
    active?: boolean;
    /** When given, the undo and redo buttons and keys act on this history instead of the editor's own. */
    outer?: OuterHistory;
    /** Called after each stroke or rectangle with the part of the shown area it changed. */
    onCommit?: (dirty: Rect) => void;
    /** False: brush, eraser and rectangle do nothing; zoom, pan, Space and the magnifier still work. */
    paintable?: boolean;
  };

  let { photoId, onChange, active = true, outer, onCommit, paintable = true }: Props = $props();

  const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);

  let tool: Tool = $state('brush');
  let size = $state(DEFAULT_SIZE);
  let loupeOn = $state(false);
  let loupeMag = $state(LOUPE_DEFAULT);
  let view: View = $state({ scale: 1, x: 0, y: 0 });
  let boxW = $state(0);
  let boxH = $state(0);
  let dims: { w: number; h: number } | null = $state(null);
  let steps = $state(0);
  let space = $state(false);
  let panning = $state(false);
  /** True while a brush, eraser or rectangle stroke runs: Space then changes nothing. */
  let stroking = $state(false);
  let photoFailed = $state(false);
  /** The photo the red layer belongs to. Until setArea brings the new photo's area, the old one is not shown or edited. */
  let shownFor: PhotoId | null = $state(null);
  const stale = $derived(shownFor !== photoId);
  const hideRed = $derived(space && !panning && !stroking);
  const canUndo = $derived.by(() => (void steps, outer ? outer.canUndo() : (red?.history.canUndo ?? false)));
  const canRedo = $derived.by(() => (void steps, outer ? outer.canRedo() : (red?.history.canRedo ?? false)));

  // Not reactive: these change on every pointer move.
  let red: RedLayer | null = null;
  /** True until the person zooms or pans; resizing the window then fits the photo again. */
  let fitted = true;
  type Stroke =
    | { kind: 'paint'; value: 0 | 255; last: [number, number]; dirty: Rect | null }
    | { kind: 'rect'; start: [number, number]; end: [number, number] }
    | { kind: 'pan'; sx: number; sy: number; from: View };
  let stroke: Stroke | null = null;
  let pointerId = -1;
  /** The pointer in box coordinates, or null when it is outside the box. */
  let pointer: { x: number; y: number } | null = null;
  /** The part of the outline canvas drawn last (CSS pixels), cleared before the next draw. */
  let drawn: Rect | null = null;

  let stage: HTMLDivElement;
  let photoCanvas: HTMLCanvasElement;
  let overlayCanvas: HTMLCanvasElement;
  let cursorCanvas: HTMLCanvasElement;
  let loupe: Loupe;
  let cursorCtx: CanvasRenderingContext2D | null = null;

  /** The area being edited (the same object it was given, changed in place), or null before setArea. */
  export function getArea(): Area | null {
    return stale ? null : (red?.area ?? null);
  }

  /** Shows `a` (at edit size) as the red layer of the current photo; undo starts afresh. */
  export function setArea(a: Area): void {
    stroke = null;
    panning = false;
    stroking = false;
    red?.set(a);
    shownFor = photoId;
    steps++;
    dims = { w: a.w, h: a.h };
    fitView();
  }

  /** Like setArea, but keeps the zoom and pan (the same photo shows a changed area). */
  export function showArea(a: Area): void {
    if (!dims || dims.w !== a.w || dims.h !== a.h || stale) return setArea(a);
    stroke = null;
    panning = false;
    stroking = false;
    red?.set(a);
    steps++;
    redraw();
  }

  // The photo, drawn into its canvas once loaded.
  $effect(() => {
    const id = photoId;
    photoFailed = false;
    photoCanvas.width = 0;
    const img = new Image();
    img.onload = () => {
      photoCanvas.width = img.naturalWidth;
      photoCanvas.height = img.naturalHeight;
      photoCanvas.getContext('2d')?.drawImage(img, 0, 0);
      redraw();
    };
    img.onerror = () => (photoFailed = true);
    img.src = imageUrl('edit', id);
    return () => {
      img.onload = null;
      img.onerror = null;
    };
  });

  onMount(() => {
    red = new RedLayer(overlayCanvas, drawLoupe);
    cursorCtx = cursorCanvas.getContext('2d');
    const measure = () => {
      boxW = stage.clientWidth;
      boxH = stage.clientHeight;
    };
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(stage);
    stage.addEventListener('wheel', wheel, { passive: false });
    // Safari (the Mac app's web view) may report a pinch as gestures instead of ctrl wheel events.
    for (const t of GESTURES) stage.addEventListener(t, gesture as EventListener, { passive: false });
    return () => {
      observer?.disconnect();
      stage.removeEventListener('wheel', wheel);
      for (const t of GESTURES) stage.removeEventListener(t, gesture as EventListener);
      red?.cancel();
    };
  });

  // A new box size: size the outline canvas to it, and fit the photo again unless zoomed.
  $effect(() => {
    const w = boxW;
    const h = boxH;
    if (w <= 0 || h <= 0) return;
    untrack(() => {
      const dpr = window.devicePixelRatio || 1;
      cursorCanvas.width = Math.round(w * dpr);
      cursorCanvas.height = Math.round(h * dpr);
      cursorCtx?.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawn = null;
      if (fitted) fitView();
      redraw();
    });
  });

  // Anything that changes what the outline or the magnifier shows.
  $effect(() => {
    void [view, tool, size, loupeOn, loupeMag, hideRed, panning, stale, paintable];
    untrack(redraw);
  });

  // Covered (preview, saving): let go of Space so the red layer is not left hidden.
  $effect(() => {
    if (!active) untrack(() => (space = false));
  });

  function fitView() {
    if (!dims) return;
    view = fit(dims.w, dims.h, Math.max(1, boxW), Math.max(1, boxH));
    fitted = true;
  }

  function zoomTo(scale: number, px = boxW / 2, py = boxH / 2) {
    view = zoomAt(view, scale / view.scale, px, py);
    fitted = false;
  }

  function local(e: { clientX: number; clientY: number }, box = stage.getBoundingClientRect()): [number, number] {
    return [e.clientX - box.left, e.clientY - box.top];
  }

  const GESTURES = ['gesturestart', 'gesturechange', 'gestureend'];
  /** The scale of the pinch gesture last applied, or null when none runs. While one runs, ctrl wheel events are the same pinch and are ignored. */
  let gestureScale: number | null = null;

  function gesture(e: Event & { scale?: number; clientX?: number; clientY?: number }) {
    e.preventDefault();
    if (e.type === 'gestureend') {
      gestureScale = null;
      return;
    }
    if (e.type === 'gesturestart') {
      gestureScale = e.scale ?? 1;
      return;
    }
    if (!dims || stroke || gestureScale === null) return;
    const scale = e.scale ?? 1;
    const [px, py] = local({ clientX: e.clientX ?? 0, clientY: e.clientY ?? 0 });
    zoomTo(view.scale * gestureRatio(gestureScale, scale), px, py);
    gestureScale = scale;
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    if (!dims || stroke) return;
    const [px, py] = local(e);
    pointer = { x: px, y: py };
    if (e.ctrlKey || e.metaKey) {
      if (gestureScale !== null) return;
      // A pinch on a trackpad arrives as a wheel with ctrlKey; Ctrl or Cmd with a wheel zooms too.
      zoomTo(view.scale * wheelZoom(e.deltaY, e.deltaMode, e.ctrlKey, boxH), px, py);
    } else {
      // Two fingers on a trackpad, or a wheel: the photo moves.
      const [dx, dy] = wheelPan(e.deltaX, e.deltaY, e.deltaMode, e.shiftKey, { w: boxW, h: boxH });
      view = panBy(view, dx, dy);
      fitted = false;
    }
  }

  /** One undo step was added or taken back. */
  function changed() {
    steps++;
    onChange?.();
  }

  function paintTo(ix: number, iy: number) {
    const area = red?.area;
    if (!area || stroke?.kind !== 'paint') return;
    const r = stampLine(area, stroke.last[0], stroke.last[1], ix, iy, size / 2, stroke.value);
    stroke.last = [ix, iy];
    stroke.dirty = unionRect(stroke.dirty, r);
    red?.queue(r);
  }

  function down(e: PointerEvent) {
    if (stroke || !red?.area || !active || stale) return;
    const [px, py] = local(e);
    pointer = { x: px, y: py };
    if (e.button === 1 || (e.button === 0 && (space || tool === 'move'))) {
      stroke = { kind: 'pan', sx: px, sy: py, from: { ...view } };
      panning = true;
    } else if (e.button === 0) {
      if (!paintable) return;
      const [ix, iy] = toImage(view, px, py);
      if (tool === 'rect') {
        stroke = { kind: 'rect', start: [ix, iy], end: [ix, iy] };
        stroking = true;
      } else {
        red.begin();
        stroke = { kind: 'paint', value: tool === 'brush' ? 255 : 0, last: [ix, iy], dirty: null };
        stroking = true;
        paintTo(ix, iy);
      }
    } else {
      return;
    }
    e.preventDefault();
    stage.focus({ preventScroll: true });
    pointerId = e.pointerId;
    stage.setPointerCapture?.(e.pointerId);
    redraw();
  }

  function move(e: PointerEvent) {
    const box = stage.getBoundingClientRect();
    const [px, py] = local(e, box);
    pointer = { x: px, y: py };
    if (stroke && e.pointerId === pointerId) {
      if (stroke.kind === 'pan') {
        view = { scale: stroke.from.scale, x: stroke.from.x + px - stroke.sx, y: stroke.from.y + py - stroke.sy };
        fitted = false;
        return;
      }
      if (stroke.kind === 'rect') stroke.end = toImage(view, px, py);
      else for (const c of e.getCoalescedEvents?.() ?? [e]) paintTo(...toImage(view, ...local(c, box)));
    }
    redraw();
  }

  /** Pointer up commits the stroke to the undo history; a cancelled rectangle is dropped. */
  function up(e: PointerEvent, cancelled = false) {
    if (!stroke || e.pointerId !== pointerId) return;
    const s = stroke;
    stroke = null;
    stroking = false;
    pointerId = -1;
    if (s.kind === 'pan') {
      panning = false;
    } else if (s.kind === 'paint') {
      if (s.dirty) committed(s.dirty);
    } else if (red?.area && !cancelled) {
      const [x0, y0] = s.start;
      const [x1, y1] = toImage(view, ...local(e));
      red.begin();
      const r = fillRect(red.area, { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, 255);
      red.queue(r);
      committed(r);
    }
    redraw();
  }

  /** A stroke or rectangle that changed rectangle `r` ended. */
  function committed(r: Rect) {
    // With an outer history, the owner of that history records the change (through onCommit).
    if (outer) red?.flush();
    else red?.commit(r);
    if (r.w > 0 && r.h > 0) onCommit?.(r);
    changed();
  }

  function leave() {
    if (stroke) return;
    pointer = null;
    redraw();
  }

  function undo() {
    if (stroke || stale) return;
    if (outer) {
      if (!outer.canUndo()) return;
      outer.undo();
      changed();
    } else if (red?.undo()) changed();
  }

  function redo() {
    if (stroke || stale) return;
    if (outer) {
      if (!outer.canRedo()) return;
      outer.redo();
      changed();
    } else if (red?.redo()) changed();
  }

  function redraw() {
    drawOutline();
    drawLoupe();
  }

  function drawOutline() {
    const ctx = cursorCtx;
    if (!ctx) return;
    if (drawn) ctx.clearRect(drawn.x, drawn.y, drawn.w, drawn.h);
    drawn = null;
    if (!pointer || !red?.area || stale || panning || !paintable || tool === 'move' || (space && !stroke)) return;
    if (stroke?.kind === 'rect') {
      const [ax, ay] = toScreen(view, ...stroke.start);
      const [bx, by] = toScreen(view, ...stroke.end);
      outlineBox(ctx, ax, ay, bx, by);
      drawn = { x: Math.min(ax, bx) - 3, y: Math.min(ay, by) - 3, w: Math.abs(bx - ax) + 6, h: Math.abs(by - ay) + 6 };
    } else if (tool === 'brush' || tool === 'eraser') {
      const r = Math.max(1, (size / 2) * view.scale);
      outlineCircle(ctx, pointer.x, pointer.y, r);
      drawn = { x: pointer.x - r - 3, y: pointer.y - r - 3, w: 2 * r + 6, h: 2 * r + 6 };
    }
  }

  /** The magnifier follows the pointer over the photo; it hides outside the photo and while panning. */
  function drawLoupe() {
    const area = red?.area;
    if (!loupe) return;
    if (!loupeOn || !area || stale || !pointer || panning) return loupe.hide();
    const [ix, iy] = toImage(view, pointer.x, pointer.y);
    if (ix < 0 || iy < 0 || ix >= area.w || iy >= area.h) return loupe.hide();
    const rect = stroke?.kind === 'rect' ? stroke : null;
    loupe.show({
      photo: photoCanvas,
      overlay: hideRed ? null : overlayCanvas,
      imgW: area.w,
      imgH: area.h,
      ix,
      iy,
      cx: pointer.x,
      cy: pointer.y,
      boxW,
      boxH,
      viewScale: view.scale,
      mag: loupeMag,
      brush: tool === 'rect' || tool === 'move' ? null : size / 2,
      rect: rect && { x: rect.start[0], y: rect.start[1], w: rect.end[0] - rect.start[0], h: rect.end[1] - rect.start[1] },
    });
  }

  function keydown(e: KeyboardEvent) {
    if (!active || isTextField(e.target)) return;
    if (e.key === ' ') {
      // A button the keyboard focused is pressed with Space, as everywhere else (the screen's
      // buttons do not take the focus on a mouse click, see keepFocus).
      if (e.target instanceof HTMLButtonElement) return;
      e.preventDefault();
      // Mid-stroke Space neither hides the red layer nor turns the stroke into a pan.
      if (!stroking) space = true;
      return;
    }
    const action = actionFor(e, isMac);
    if (!action) return;
    e.preventDefault();
    if (e.repeat && !repeats(action)) return;
    run(action);
  }

  function keyup(e: KeyboardEvent) {
    if (e.key === ' ') space = false;
  }

  function run(action: Action) {
    switch (action) {
      case 'move':
      case 'brush':
      case 'eraser':
      case 'rect':
        tool = action;
        break;
      case 'sizeDown':
        size = stepSize(size, -1);
        break;
      case 'sizeUp':
        size = stepSize(size, 1);
        break;
      case 'toggleLoupe':
        loupeOn = !loupeOn;
        break;
      case 'undo':
        undo();
        break;
      case 'redo':
        redo();
        break;
      case 'fit':
        fitView();
        break;
      case 'actualSize':
        zoomTo(1);
        break;
      case 'zoomIn':
        zoomTo(nextZoomStep(view.scale));
        break;
      case 'zoomOut':
        zoomTo(prevZoomStep(view.scale));
        break;
    }
  }
</script>

<svelte:window onkeydown={keydown} onkeyup={keyup} onblur={() => (space = false)} />

<div class="editor">
  <Toolbar
    bind:tool
    bind:size
    bind:loupe={loupeOn}
    bind:magnification={loupeMag}
    {canUndo}
    {canRedo}
    zoom={Math.round(view.scale * 100)}
    {isMac}
    onUndo={undo}
    onRedo={redo}
    onZoomOut={() => run('zoomOut')}
    onZoomIn={() => run('zoomIn')}
    onActualSize={() => run('actualSize')}
    onFit={fitView}
  />

  <!-- The painting surface is one widget: it takes the focus on click so the shortcuts and
       Space act on it rather than on the last button pressed. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    bind:this={stage}
    class="stage"
    class:rect={tool === 'rect'}
    class:grab={space || tool === 'move'}
    class:panning
    role="application"
    aria-label={copy.editor.photo}
    tabindex="0"
    onpointerdown={down}
    onpointermove={move}
    onpointerup={(e) => up(e)}
    onpointercancel={(e) => up(e, true)}
    onpointerleave={leave}
  >
    <div
      class="sheet"
      data-layer="sheet"
      class:pixels={view.scale >= 2}
      style:width="{dims?.w ?? 0}px"
      style:height="{dims?.h ?? 0}px"
      style:transform="translate({view.x}px, {view.y}px) scale({view.scale})"
    >
      <canvas bind:this={photoCanvas}></canvas>
      <canvas bind:this={overlayCanvas} data-layer="red" style:visibility={hideRed || stale ? 'hidden' : 'visible'}
      ></canvas>
    </div>
    <canvas bind:this={cursorCanvas} class="outline" style:width="{boxW}px" style:height="{boxH}px"></canvas>
    <Loupe bind:this={loupe} />
    {#if photoFailed}
      <p class="error" role="alert">{copy.checkWatermark.photoFailed}</p>
    {/if}
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
    min-height: 0;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 240px;
    overflow: hidden;
    border-radius: var(--radius);
    background: var(--track);
    cursor: crosshair;
    touch-action: none;
    user-select: none;
  }
  .stage:focus-visible {
    outline: none;
    box-shadow: var(--focus);
  }
  .stage.grab {
    cursor: grab;
  }
  .stage.panning {
    cursor: grabbing;
  }
  .sheet {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }
  .sheet canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  /* Zoomed in, each photo pixel shows as a square, so edges can be painted exactly. */
  .sheet.pixels canvas {
    image-rendering: pixelated;
  }
  .outline {
    position: absolute;
    top: 0;
    left: 0;
    pointer-events: none;
  }
  .error {
    position: absolute;
    top: var(--space-3);
    left: var(--space-3);
    margin: 0;
    color: var(--danger);
  }
</style>
