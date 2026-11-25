<script lang="ts">
  // The magnifier: an enlarged view of the photo, the red layer and the brush outline around
  // the cursor. The editor calls show() and hide() as the pointer moves; they redraw only this
  // small canvas and move it, so nothing else on the page renders again.
  import { onMount } from 'svelte';
  import { LOUPE_SIZE, loupeClip, loupePlacement, loupeSource, type LoupeFrame } from './loupe';
  import { outlineBox, outlineCircle, outlineCross } from './outline';

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let dpr = 1;

  onMount(() => {
    dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(LOUPE_SIZE * dpr);
    canvas.height = Math.round(LOUPE_SIZE * dpr);
    ctx = canvas.getContext('2d');
  });

  export function hide(): void {
    if (canvas) canvas.style.display = 'none';
  }

  export function show(f: LoupeFrame): void {
    if (!ctx) return;
    const src = loupeSource(f.ix, f.iy, f.viewScale, f.mag);
    const k = LOUPE_SIZE / src.sw;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, LOUPE_SIZE, LOUPE_SIZE);
    const part = loupeClip(src, f.imgW, f.imgH);
    if (part) {
      ctx.imageSmoothingEnabled = false;
      for (const layer of [f.photo, f.overlay]) {
        // A canvas without pixels yet (photo still loading) cannot be drawn from.
        if (!layer || layer.width === 0 || layer.height === 0) continue;
        const sx = layer.width / f.imgW;
        const sy = layer.height / f.imgH;
        ctx.drawImage(layer, part.sx * sx, part.sy * sy, part.sw * sx, part.sh * sy, part.dx, part.dy, part.dw, part.dh);
      }
    }
    const c = LOUPE_SIZE / 2;
    if (f.rect) {
      const x = (f.rect.x - src.sx) * k;
      const y = (f.rect.y - src.sy) * k;
      outlineBox(ctx, x, y, x + f.rect.w * k, y + f.rect.h * k);
    }
    if (f.brush !== null) outlineCircle(ctx, c, c, f.brush * k);
    else outlineCross(ctx, c, c, 8);
    const at = loupePlacement(f.cx, f.cy, f.boxW, f.boxH);
    canvas.style.transform = `translate(${at.x}px, ${at.y}px)`;
    canvas.style.display = 'block';
  }
</script>

<canvas bind:this={canvas} class="loupe" style:width="{LOUPE_SIZE}px" style:height="{LOUPE_SIZE}px" aria-hidden="true"
></canvas>

<style>
  .loupe {
    position: absolute;
    top: 0;
    left: 0;
    display: none;
    border: 2px solid var(--surface);
    border-radius: var(--radius);
    background: var(--track);
    box-shadow: 0 4px 16px rgb(0 0 0 / 35%);
    pointer-events: none;
  }
</style>
