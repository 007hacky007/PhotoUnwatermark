import type { Rect } from './paint';

export const LOUPE_SIZE = 220;
export const LOUPE_STEPS = [2, 4, 8];
export const LOUPE_DEFAULT = 4;
const OFFSET = 24;

/** Image rectangle the loupe shows: centred on (ix, iy), magnified `mag` times the view scale. */
export function loupeSource(ix: number, iy: number, viewScale: number, mag: number) {
  const size = LOUPE_SIZE / (viewScale * mag);
  return { sx: ix - size / 2, sy: iy - size / 2, sw: size, sh: size };
}

/** Top-left of the loupe: up-right of the cursor, flipped to stay inside the viewport. */
export function loupePlacement(cx: number, cy: number, vw: number, vh: number): { x: number; y: number } {
  let x = cx + OFFSET;
  let y = cy - OFFSET - LOUPE_SIZE;
  if (x + LOUPE_SIZE > vw) x = cx - OFFSET - LOUPE_SIZE;
  if (y < 0) y = cy + OFFSET;
  return {
    x: Math.max(0, Math.min(x, vw - LOUPE_SIZE)),
    y: Math.max(0, Math.min(y, vh - LOUPE_SIZE)),
  };
}

/** Where a loupe source rectangle lies on a w x h image, and where that part goes on the loupe. */
export type LoupeClip = { sx: number; sy: number; sw: number; sh: number; dx: number; dy: number; dw: number; dh: number };

/**
 * Clips a `loupeSource` rectangle to the image, so drawing never reads outside it (near the
 * edges the rest of the loupe stays empty). Null when none of the rectangle is on the image.
 */
export function loupeClip(src: { sx: number; sy: number; sw: number; sh: number }, w: number, h: number): LoupeClip | null {
  const x0 = Math.max(0, src.sx);
  const y0 = Math.max(0, src.sy);
  const x1 = Math.min(w, src.sx + src.sw);
  const y1 = Math.min(h, src.sy + src.sh);
  if (x1 <= x0 || y1 <= y0) return null;
  const k = LOUPE_SIZE / src.sw;
  return { sx: x0, sy: y0, sw: x1 - x0, sh: y1 - y0, dx: (x0 - src.sx) * k, dy: (y0 - src.sy) * k, dw: (x1 - x0) * k, dh: (y1 - y0) * k };
}

/** What the magnifier draws for one pointer position. */
export type LoupeFrame = {
  photo: HTMLCanvasElement;
  /** The red layer, or null while it is hidden. */
  overlay: HTMLCanvasElement | null;
  /** Photo size (edit size) and the cursor on it. */
  imgW: number;
  imgH: number;
  ix: number;
  iy: number;
  /** The cursor and the size of the box the loupe must stay inside, in CSS pixels. */
  cx: number;
  cy: number;
  boxW: number;
  boxH: number;
  viewScale: number;
  mag: number;
  /** Brush radius in photo pixels, or null for the Rectangle tool. */
  brush: number | null;
  /** The rectangle being dragged, in photo pixels. */
  rect: Rect | null;
};
