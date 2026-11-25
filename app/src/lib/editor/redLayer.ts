import { History } from './history';
import { renderOverlay, unionRect, type Area, type Rect } from './paint';

/**
 * The red layer: the area being edited, its undo history, and the canvas that shows it (at
 * edit size). Changes are queued as rectangles and drawn once per animation frame, and only
 * the changed rectangle is drawn again, never the whole layer.
 */
export class RedLayer {
  readonly history = new History();
  private current: Area | null = null;
  private buffer: ImageData | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private pending: Rect | null = null;
  private frame = 0;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    /** Called after each redraw (the magnifier shows the layer too). */
    private readonly onDrawn: () => void = () => {},
  ) {}

  /** The area being edited (the object given to `set`, changed in place), or null before. */
  get area(): Area | null {
    return this.current;
  }

  /** Shows `a`, drawing all of it; the undo history starts afresh. */
  set(a: Area): void {
    this.cancel();
    this.current = a;
    this.history.reset(a);
    this.canvas.width = a.w;
    this.canvas.height = a.h;
    this.ctx = this.canvas.getContext('2d');
    if (!this.ctx) return;
    if (!this.buffer || this.buffer.width !== a.w || this.buffer.height !== a.h) {
      this.buffer = this.ctx.createImageData(a.w, a.h);
    }
    this.queue({ x: 0, y: 0, w: a.w, h: a.h });
    this.flush();
  }

  /** Call before changing the area in a stroke (see History.begin). */
  begin(): void {
    if (this.current) this.history.begin(this.current);
  }

  /** Draws rectangle `r` with the next animation frame. */
  queue(r: Rect): void {
    if (r.w <= 0 || r.h <= 0) return;
    this.pending = unionRect(this.pending, r);
    if (!this.frame) this.frame = requestAnimationFrame(() => this.flush());
  }

  /** Draws what is queued now. */
  flush(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    const r = this.pending;
    this.pending = null;
    if (!r || !this.current || !this.buffer || !this.ctx) return;
    renderOverlay(this.current, this.buffer, r);
    this.ctx.putImageData(this.buffer, 0, 0, r.x, r.y, r.w, r.h);
    this.onDrawn();
  }

  /** Draws what is queued and records the change inside `r` as one undo step. */
  commit(r: Rect): void {
    this.flush();
    if (this.current) this.history.commit(this.current, r);
  }

  /** Takes back the last step; false when there was none. */
  undo(): boolean {
    return this.step(this.current && this.history.undo(this.current));
  }

  /** Does the last undone step again; false when there was none. */
  redo(): boolean {
    return this.step(this.current && this.history.redo(this.current));
  }

  /** Drops a queued redraw (when the editor goes away). */
  cancel(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.pending = null;
  }

  private step(r: Rect | null): boolean {
    if (!r) return false;
    this.queue(r);
    this.flush();
    return true;
  }
}
