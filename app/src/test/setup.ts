import { afterEach, beforeEach, expect, vi } from 'vitest';

// Svelte reports misuse (and the tests' own mistakes) through console warnings and errors:
// any of them fails the test that caused it, so the test output stays clean.
beforeEach(() => {
  vi.spyOn(console, 'warn');
  vi.spyOn(console, 'error');
});

afterEach(() => {
  expect(console.warn, 'console.warn was called').not.toHaveBeenCalled();
  expect(console.error, 'console.error was called').not.toHaveBeenCalled();
  vi.restoreAllMocks();
});

// jsdom has no canvas. The editor's components draw through this 2D context, which accepts
// their calls and draws nothing; the pixel logic is tested on its own (src/lib/editor).
const contexts = new WeakMap<HTMLCanvasElement, object>();
const noop = () => {};

function context2d(canvas: HTMLCanvasElement): object {
  return {
    canvas,
    lineWidth: 1,
    strokeStyle: '',
    fillStyle: '',
    globalAlpha: 1,
    imageSmoothingEnabled: true,
    save: noop,
    restore: noop,
    setTransform: noop,
    clearRect: noop,
    fillRect: noop,
    beginPath: noop,
    moveTo: noop,
    lineTo: noop,
    arc: noop,
    rect: noop,
    stroke: noop,
    drawImage: noop,
    putImageData: noop,
    createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4), width: w, height: h }),
  };
}

HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, kind: string) {
  if (kind !== '2d') return null;
  let ctx = contexts.get(this);
  if (!ctx) contexts.set(this, (ctx = context2d(this)));
  return ctx;
} as unknown as HTMLCanvasElement['getContext'];
