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

/** The next larger ZOOM_STEPS entry (the largest one when already at the top). */
export function nextZoomStep(scale: number): number {
  return ZOOM_STEPS.find((s) => s > scale * (1 + 1e-9)) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1];
}

/** The next smaller ZOOM_STEPS entry (the smallest one when already at the bottom). */
export function prevZoomStep(scale: number): number {
  return [...ZOOM_STEPS].reverse().find((s) => s < scale * (1 - 1e-9)) ?? ZOOM_STEPS[0];
}

/**
 * Zoom factor for one wheel event. `deltaMode` 1 counts lines, 2 pages (`pageHeight` pixels).
 * A pinch on a Mac trackpad arrives as a wheel event with ctrlKey and small deltas, so it
 * zooms faster per unit.
 */
export function wheelZoom(deltaY: number, deltaMode: number, ctrlKey: boolean, pageHeight: number): number {
  const unit = deltaMode === 1 ? 16 : deltaMode === 2 ? pageHeight : 1;
  // One event never zooms by more than about 1.6 times, so a mouse wheel notch with Ctrl is not a jump.
  // Small pixel deltas with Ctrl are a trackpad pinch (faster per unit); a wheel notch, with Ctrl or Cmd, steps like a plain wheel did.
  const pinch = ctrlKey && deltaMode === 0 && Math.abs(deltaY) < 50;
  const exponent = Math.max(-0.5, Math.min(0.5, -deltaY * unit * (pinch ? 0.01 : 0.002)));
  return Math.exp(exponent);
}

/** Moves the view by (dx, dy) screen pixels. */
export function panBy(v: View, dx: number, dy: number): View {
  return { scale: v.scale, x: v.x + dx, y: v.y + dy };
}

/**
 * How far one wheel event moves the photo, in screen pixels: the photo follows the fingers on a
 * trackpad. Shift turns a mouse wheel's up and down into sideways.
 */
export function wheelPan(
  deltaX: number,
  deltaY: number,
  deltaMode: number,
  shiftKey: boolean,
  pageSize: { w: number; h: number },
): [number, number] {
  const ux = deltaMode === 1 ? 16 : deltaMode === 2 ? pageSize.w : 1;
  const uy = deltaMode === 1 ? 16 : deltaMode === 2 ? pageSize.h : 1;
  let dx = deltaX * ux;
  let dy = deltaY * uy;
  if (shiftKey && dx === 0) [dx, dy] = [dy, 0];
  return [-dx, -dy];
}

/** Safari's gesturechange `scale` counts from the start of the pinch; this is the factor since the last event. */
export function gestureRatio(previous: number, scale: number): number {
  return previous > 0 && scale > 0 ? scale / previous : 1;
}
