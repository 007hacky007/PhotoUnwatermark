import { describe, expect, it } from 'vitest';
import { LOUPE_SIZE, loupePlacement, loupeSource } from './loupe';

describe('loupe', () => {
  it('loupeSourceScalesWithMag', () => {
    const a = loupeSource(500, 300, 1, 4);
    expect(a).toEqual({ sx: 500 - 27.5, sy: 300 - 27.5, sw: 55, sh: 55 });
    const b = loupeSource(500, 300, 1, 8);
    expect(b.sw).toBeCloseTo(27.5);
    const c = loupeSource(500, 300, 0.5, 2);
    expect(c.sw).toBeCloseTo(220);
  });

  it('loupeStaysInViewport', () => {
    const vw = 800;
    const vh = 600;
    for (const [cx, cy] of [[0, 0], [vw, 0], [0, vh], [vw, vh], [400, 300], [790, 10]]) {
      const p = loupePlacement(cx, cy, vw, vh);
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.x + LOUPE_SIZE).toBeLessThanOrEqual(vw);
      expect(p.y + LOUPE_SIZE).toBeLessThanOrEqual(vh);
    }
  });

  it('sits up-right of the cursor when there is room', () => {
    expect(loupePlacement(300, 400, 800, 600)).toEqual({ x: 324, y: 400 - 24 - LOUPE_SIZE });
  });

  it('flips left and below at the top-right corner', () => {
    expect(loupePlacement(780, 10, 800, 600)).toEqual({ x: 780 - 24 - LOUPE_SIZE, y: 34 });
  });
});
