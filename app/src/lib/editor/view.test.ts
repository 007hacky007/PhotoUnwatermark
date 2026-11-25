import { describe, expect, it } from 'vitest';
import { fit, toImage, toScreen, zoomAt, ZOOM_STEPS } from './view';

describe('view', () => {
  it('fitCentres', () => {
    const v = fit(4000, 2000, 1000, 1000);
    expect(v.scale).toBeCloseTo(0.25);
    expect(v.x).toBeCloseTo(0);
    expect(v.y).toBeCloseTo(250);
    const t = fit(100, 400, 1000, 800);
    expect(t.scale).toBeCloseTo(2);
    expect(t.x).toBeCloseTo(400);
    expect(t.y).toBeCloseTo(0);
  });

  it('zoomKeepsPointUnderCursor', () => {
    const v = { scale: 0.5, x: 30, y: -20 };
    const [ix, iy] = toImage(v, 300, 200);
    const z = zoomAt(v, 3, 300, 200);
    expect(z.scale).toBeCloseTo(1.5);
    const [sx, sy] = toScreen(z, ix, iy);
    expect(sx).toBeCloseTo(300);
    expect(sy).toBeCloseTo(200);
  });

  it('zoomClamped', () => {
    expect(zoomAt({ scale: 10, x: 0, y: 0 }, 100, 5, 5).scale).toBe(16);
    expect(zoomAt({ scale: 0.1, x: 0, y: 0 }, 0.001, 5, 5).scale).toBe(0.05);
    const z = zoomAt({ scale: 16, x: 7, y: 9 }, 2, 50, 50);
    expect(z).toEqual({ scale: 16, x: 7, y: 9 });
  });

  it('toImage and toScreen are inverses; steps are ascending', () => {
    const v = { scale: 2.5, x: 11, y: -4 };
    const [ix, iy] = toImage(v, 100, 50);
    expect(toScreen(v, ix, iy)).toEqual([100, 50]);
    expect([...ZOOM_STEPS].sort((a, b) => a - b)).toEqual(ZOOM_STEPS);
  });
});
