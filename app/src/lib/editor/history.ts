import type { Area, Rect } from './paint';

type Step = { rect: Rect; before: Uint8Array; after: Uint8Array };

const MAX_STEPS = 100;
const MAX_BYTES = 256 * 1024 * 1024;

function read(a: Area, r: Rect): Uint8Array {
  const out = new Uint8Array(r.w * r.h);
  for (let y = 0; y < r.h; y++) {
    const s = (r.y + y) * a.w + r.x;
    out.set(a.data.subarray(s, s + r.w), y * r.w);
  }
  return out;
}

function write(a: Area, r: Rect, patch: Uint8Array): void {
  for (let y = 0; y < r.h; y++) {
    a.data.set(patch.subarray(y * r.w, (y + 1) * r.w), (r.y + y) * a.w + r.x);
  }
}

/**
 * Undo/redo by dirty-rectangle patches. A shadow copy of the area (taken once in begin)
 * supplies the "before" bytes, so a step costs only its dirty rectangle.
 */
export class History {
  private shadow: Uint8Array | null = null;
  private undoStack: Step[] = [];
  private redoStack: Step[] = [];
  private bytes = 0;

  constructor(
    private readonly maxSteps = MAX_STEPS,
    private readonly maxBytes = MAX_BYTES,
  ) {}

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  /** Call before the first edit (and before each stroke; later calls are cheap). */
  begin(a: Area): void {
    if (!this.shadow || this.shadow.length !== a.data.length) {
      this.shadow = a.data.slice();
      this.undoStack = [];
      this.redoStack = [];
      this.bytes = 0;
    }
  }

  /** Record the edit that changed rect `dirty` of `a` since the last commit. */
  commit(a: Area, dirty: Rect): void {
    if (!this.shadow) throw new Error('History.begin must be called before commit');
    if (dirty.w <= 0 || dirty.h <= 0) return;
    const shadowArea: Area = { w: a.w, h: a.h, data: this.shadow };
    const step: Step = { rect: { ...dirty }, before: read(shadowArea, dirty), after: read(a, dirty) };
    write(shadowArea, dirty, step.after);
    this.redoStack = [];
    this.undoStack.push(step);
    this.bytes += step.before.length + step.after.length;
    while (this.undoStack.length > this.maxSteps || (this.bytes > this.maxBytes && this.undoStack.length > 1)) {
      const old = this.undoStack.shift()!;
      this.bytes -= old.before.length + old.after.length;
    }
  }

  undo(a: Area): Rect | null {
    const step = this.undoStack.pop();
    if (!step) return null;
    this.apply(a, step, step.before);
    this.redoStack.push(step);
    return { ...step.rect };
  }

  redo(a: Area): Rect | null {
    const step = this.redoStack.pop();
    if (!step) return null;
    this.apply(a, step, step.after);
    this.undoStack.push(step);
    return { ...step.rect };
  }

  private apply(a: Area, step: Step, patch: Uint8Array): void {
    write(a, step.rect, patch);
    write({ w: a.w, h: a.h, data: this.shadow! }, step.rect, patch);
  }
}
