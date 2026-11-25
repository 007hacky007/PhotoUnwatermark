/** screen = image * scale + (x, y) */
export type View = { scale: number; x: number; y: number };

export const MIN_SCALE = 0.05;
export const MAX_SCALE = 16;
export const ZOOM_STEPS = [0.05, 0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8, 12, 16];

const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Largest scale that shows the whole image in the box, centred. */
export function fit(imgW: number, imgH: number, boxW: number, boxH: number): View {
  const scale = clampScale(Math.min(boxW / imgW, boxH / imgH));
  return { scale, x: (boxW - imgW * scale) / 2, y: (boxH - imgH * scale) / 2 };
}

/** Zoom by `factor` keeping the image point under (px, py) where it is. */
export function zoomAt(v: View, factor: number, px: number, py: number): View {
  const scale = clampScale(v.scale * factor);
  const k = scale / v.scale;
  return { scale, x: px - (px - v.x) * k, y: py - (py - v.y) * k };
}

export function toImage(v: View, px: number, py: number): [number, number] {
  return [(px - v.x) / v.scale, (py - v.y) / v.scale];
}

export function toScreen(v: View, ix: number, iy: number): [number, number] {
  return [ix * v.scale + v.x, iy * v.scale + v.y];
}
