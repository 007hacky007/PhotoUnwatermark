import { describe, expect, it } from 'vitest';
import { LOUPE_SIZE, loupeClip, loupePlacement, loupeSource } from './loupe';

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

  it('clipsTheSourceAtAllFourEdges', () => {
    // 55 x 55 source on a 100 x 80 image: k = 220 / 55 = 4 loupe pixels per photo pixel.
    expect(loupeClip({ sx: 20, sy: 10, sw: 55, sh: 55 }, 100, 80)).toEqual({ sx: 20, sy: 10, sw: 55, sh: 55, dx: 0, dy: 0, dw: 220, dh: 220 });
    expect(loupeClip({ sx: -10, sy: -5, sw: 55, sh: 55 }, 100, 80)).toEqual({ sx: 0, sy: 0, sw: 45, sh: 50, dx: 40, dy: 20, dw: 180, dh: 200 });
    expect(loupeClip({ sx: 70, sy: 60, sw: 55, sh: 55 }, 100, 80)).toEqual({ sx: 70, sy: 60, sw: 30, sh: 20, dx: 0, dy: 0, dw: 120, dh: 80 });
    expect(loupeClip({ sx: 100, sy: 0, sw: 55, sh: 55 }, 100, 80)).toBeNull();
    expect(loupeClip({ sx: -60, sy: -60, sw: 55, sh: 55 }, 100, 80)).toBeNull();
  });
});
