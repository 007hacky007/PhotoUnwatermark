import { describe, expect, it } from 'vitest';
import { fit, nextZoomStep, panBy, prevZoomStep, toImage, toScreen, wheelPan, wheelZoom, zoomAt, ZOOM_STEPS } from './view';

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

  it('steps to the next and previous zoom entry, clamped', () => {
    expect(nextZoomStep(1)).toBe(1.5);
    expect(nextZoomStep(0.3)).toBe(0.5);
    expect(nextZoomStep(16)).toBe(16);
    expect(nextZoomStep(0.01)).toBe(0.05);
    expect(prevZoomStep(1)).toBe(0.75);
    expect(prevZoomStep(0.3)).toBe(0.25);
    expect(prevZoomStep(0.05)).toBe(0.05);
    expect(prevZoomStep(40)).toBe(16);
  });

  it('wheelPanFollowsTheFingersAndShiftGoesSideways', () => {
    const page = { w: 1000, h: 800 };
    expect(wheelPan(5, 12, 0, false, page)).toEqual([-5, -12]);
    expect(wheelPan(0, 12, 0, true, page)).toEqual([-12, -0]);
    expect(wheelPan(3, 12, 0, true, page)).toEqual([-3, -12]);
    expect(wheelPan(0, 1, 1, false, page)).toEqual([-0, -16]);
    expect(wheelPan(0, 1, 2, false, page)).toEqual([-0, -800]);
    expect(panBy({ scale: 2, x: 1, y: 2 }, 3, 4)).toEqual({ scale: 2, x: 4, y: 6 });
  });

  it('oneWheelEventNeverZoomsByMoreThanAboutOnePointSix', () => {
    expect(wheelZoom(-1000, 0, true, 800)).toBeCloseTo(Math.exp(0.5));
    expect(wheelZoom(1000, 0, true, 800)).toBeCloseTo(Math.exp(-0.5));
  });

  it('wheelZoomDirectionAndUnits', () => {
    expect(wheelZoom(-100, 0, false, 800)).toBeGreaterThan(1);
    expect(wheelZoom(100, 0, false, 800)).toBeLessThan(1);
    expect(wheelZoom(100, 0, false, 800) * wheelZoom(-100, 0, false, 800)).toBeCloseTo(1);
    expect(wheelZoom(3, 1, false, 800)).toBeCloseTo(wheelZoom(48, 0, false, 800));
    expect(wheelZoom(1, 2, false, 800)).toBeCloseTo(wheelZoom(800, 0, false, 800));
    // A pinch's small deltas still zoom noticeably.
    expect(wheelZoom(-10, 0, true, 800)).toBeCloseTo(Math.exp(0.1));
  });
});
