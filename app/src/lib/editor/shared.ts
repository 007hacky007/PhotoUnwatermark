// Step 2's one drawing per group. It is kept at the reference photo's edit size; every strip
// photo shows it through that photo's placement, and a stroke on a strip photo is carried back
// into it through the same placement.

import type { StripPlacement } from '../types';
import { History } from './history';
import type { Area, Rect } from './paint';

/** The placement of the reference photo on itself. */
export const IDENTITY: StripPlacement = { status: 'found', scaleX: 1, scaleY: 1, dx: 0, dy: 0 };

/** Undo and redo of a drawing kept outside the editor. */
export interface OuterHistory {
  canUndo(): boolean;
  canRedo(): boolean;
  undo(): void;
  redo(): void;
}

/** True when `p` maps the drawing somewhere (not found, or a broken transform, maps it nowhere). */
function maps(p: StripPlacement): boolean {
  return (
    p.status !== 'notFound' &&
    p.scaleX > 0 &&
    p.scaleY > 0 &&
    Number.isFinite(p.scaleX) &&
    Number.isFinite(p.scaleY) &&
    Number.isFinite(p.dx) &&
    Number.isFinite(p.dy)
  );
}

/** The group's one drawing, at the reference photo's edit size, with its own undo history. */
export class SharedDrawing {
  readonly area: Area;
  private readonly history = new History();
  private changes = 0;

  constructor(area: Area) {
    this.area = area;
    this.history.reset(area);
  }

  /** Grows with every change, undo and redo. */
  get revision(): number {
    return this.changes;
  }

  canUndo(): boolean {
    return this.history.canUndo;
  }

  canRedo(): boolean {
    return this.history.canRedo;
  }

  /**
   * The drawing as it shows on a `w` x `h` photo placed with `p`: each pixel centre of the view
   * is mapped back into the drawing and takes the pixel it lands in (0 outside the drawing).
   * Empty for a placement that was not found.
   */
  viewOn(p: StripPlacement, w: number, h: number): Area {
    const data = new Uint8Array(w * h);
    if (!maps(p)) return { w, h, data };
    const a = this.area;
    const cols = new Int32Array(w);
    for (let u = 0; u < w; u++) {
      const x = Math.floor((u + 0.5 - p.dx) / p.scaleX);
      cols[u] = x >= 0 && x < a.w ? x : -1;
    }
    for (let v = 0; v < h; v++) {
      const y = Math.floor((v + 0.5 - p.dy) / p.scaleY);
      if (y < 0 || y >= a.h) continue;
      const from = y * a.w;
      const to = v * w;
      for (let u = 0; u < w; u++) if (cols[u] >= 0) data[to + u] = a.data[from + cols[u]];
    }
    return { w, h, data };
  }

  /**
   * Carries the part `dirty` of `view` (shown with placement `p`) back into the drawing as one
   * undo step: each drawing pixel whose centre lands inside `dirty` takes the view's value
   * there, and nothing else changes. False when nothing changed or `p` is not found.
   */
  carryBack(p: StripPlacement, view: Area, dirty: Rect): boolean {
    if (p.status !== 'found' || !maps(p)) return false;
    const l = Math.max(0, dirty.x);
    const t = Math.max(0, dirty.y);
    const r = Math.min(view.w, dirty.x + dirty.w);
    const b = Math.min(view.h, dirty.y + dirty.h);
    if (r <= l || b <= t) return false;
    const a = this.area;
    // The corners of `dirty` in the drawing, clipped to it and grown by a pixel.
    const x0 = Math.max(0, Math.floor((l - p.dx) / p.scaleX) - 1);
    const y0 = Math.max(0, Math.floor((t - p.dy) / p.scaleY) - 1);
    const x1 = Math.min(a.w, Math.ceil((r - p.dx) / p.scaleX) + 1);
    const y1 = Math.min(a.h, Math.ceil((b - p.dy) / p.scaleY) + 1);
    if (x1 <= x0 || y1 <= y0) return false;
    const cols = new Int32Array(x1 - x0);
    for (let i = 0; i < cols.length; i++) {
      const u = Math.floor((x0 + i + 0.5) * p.scaleX + p.dx);
      cols[i] = u >= l && u < r ? u : -1;
    }
    this.history.begin(a);
    let changed = false;
    for (let y = y0; y < y1; y++) {
      const v = Math.floor((y + 0.5) * p.scaleY + p.dy);
      if (v < t || v >= b) continue;
      const from = v * view.w;
      const to = y * a.w + x0;
      for (let i = 0; i < cols.length; i++) {
        if (cols[i] < 0) continue;
        const value = view.data[from + cols[i]] !== 0 ? 255 : 0;
        if (a.data[to + i] !== value) {
          a.data[to + i] = value;
          changed = true;
        }
      }
    }
    if (!changed) return false;
    this.history.commit(a, { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
    this.changes++;
    return true;
  }

  /** Takes back the last step; false when there was none. */
  undo(): boolean {
    if (!this.history.undo(this.area)) return false;
    this.changes++;
    return true;
  }

  /** Does the last undone step again; false when there was none. */
  redo(): boolean {
    if (!this.history.redo(this.area)) return false;
    this.changes++;
    return true;
  }
}
