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
