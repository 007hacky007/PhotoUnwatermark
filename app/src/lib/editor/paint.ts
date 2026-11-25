/** Binary watermark area at edit size: one byte per pixel, 0 or 255. */
export type Area = { w: number; h: number; data: Uint8Array };

/** Pixel rectangle. A width or height of 0 means "nothing". */
export type Rect = { x: number; y: number; w: number; h: number };

/** The part of ImageData the overlay needs, so it works without a canvas implementation. */
export type OverlayBuffer = { data: Uint8ClampedArray; width: number; height: number };

const EMPTY: Rect = { x: 0, y: 0, w: 0, h: 0 };

function clipRect(a: Area, x0: number, y0: number, x1: number, y1: number): Rect {
  const l = Math.max(0, Math.min(x0, x1));
  const t = Math.max(0, Math.min(y0, y1));
  const r = Math.min(a.w, Math.max(x0, x1));
  const b = Math.min(a.h, Math.max(y0, y1));
  if (r <= l || b <= t) return { ...EMPTY };
  return { x: l, y: t, w: r - l, h: b - t };
}

function stamp(a: Area, cx: number, cy: number, radius: number, value: number): void {
  const top = Math.max(0, Math.ceil(cy - radius - 0.5));
  const bottom = Math.min(a.h - 1, Math.floor(cy + radius - 0.5));
  const r2 = radius * radius;
  for (let y = top; y <= bottom; y++) {
    const dy = y + 0.5 - cy;
    const span = Math.sqrt(r2 - dy * dy);
    const from = Math.max(0, Math.ceil(cx - span - 0.5));
    const to = Math.min(a.w - 1, Math.floor(cx + span - 0.5));
    if (to >= from) a.data.fill(value, y * a.w + from, y * a.w + to + 1);
  }
}

/** Paint filled circles along the segment; returns the dirty rectangle, clipped to the area. */
export function stampLine(
  a: Area,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  radius: number,
  value: 0 | 255,
): Rect {
  radius = Math.max(0.5, radius);
  const step = radius / 2;
  const n = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) / step));
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    stamp(a, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, radius, value);
  }
  return clipRect(
    a,
    Math.floor(Math.min(x0, x1) - radius),
    Math.floor(Math.min(y0, y1) - radius),
    Math.ceil(Math.max(x0, x1) + radius),
    Math.ceil(Math.max(y0, y1) + radius),
  );
}

/** Fill a rectangle (corners in any order, so a drag up-left works); returns the clipped rect. */
export function fillRect(a: Area, r: Rect, value: 0 | 255): Rect {
  const c = clipRect(a, Math.round(r.x), Math.round(r.y), Math.round(r.x + r.w), Math.round(r.y + r.h));
  for (let y = c.y; y < c.y + c.h; y++) a.data.fill(value, y * a.w + c.x, y * a.w + c.x + c.w);
  return c;
}

/** Write the red overlay (255,59,48,115 where set, transparent elsewhere) for rect r only. */
export function renderOverlay(a: Area, out: OverlayBuffer, r: Rect): void {
  const c = clipRect(a, r.x, r.y, r.x + r.w, r.y + r.h);
  for (let y = c.y; y < c.y + c.h; y++) {
    for (let x = c.x; x < c.x + c.w; x++) {
      const set = a.data[y * a.w + x] === 255;
      const o = (y * a.w + x) * 4;
      out.data[o] = set ? 255 : 0;
      out.data[o + 1] = set ? 59 : 0;
      out.data[o + 2] = set ? 48 : 0;
      out.data[o + 3] = set ? 115 : 0;
    }
  }
}

export function isEmpty(a: Area): boolean {
  return !a.data.some((v) => v !== 0);
}

/** Smallest rectangle containing both; an empty rectangle contributes nothing. */
export function unionRect(a: Rect | null, b: Rect): Rect {
  if (!a || a.w <= 0 || a.h <= 0) return { ...b };
  if (b.w <= 0 || b.h <= 0) return { ...a };
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y };
}
