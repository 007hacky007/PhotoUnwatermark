import { describe, expect, it } from 'vitest';
import { fillRect, isEmpty, renderOverlay, stampLine, type Area } from './paint';

const area = (w: number, h: number): Area => ({ w, h, data: new Uint8Array(w * h) });

describe('paint', () => {
  it('stampLineCoversSegmentWithoutGaps', () => {
    const a = area(200, 200);
    stampLine(a, 10, 10, 180, 150, 3, 255);
    // Every pixel centre on the segment (and 2 px either side along x) is painted.
    for (let i = 0; i <= 1000; i++) {
      const t = i / 1000;
      const x = Math.floor(10 + 170 * t);
      const y = Math.floor(10 + 140 * t);
      expect(a.data[y * 200 + x]).toBe(255);
    }
  });

  it('paints a round disc of the right size', () => {
    const a = area(20, 20);
    const r = stampLine(a, 10, 10, 10, 10, 3, 255);
    let n = 0;
    for (const v of a.data) if (v === 255) n++;
    expect(n).toBeGreaterThanOrEqual(24);
    expect(n).toBeLessThanOrEqual(32);
    expect(a.data[10 * 20 + 10]).toBe(255);
    expect(a.data[10 * 20 + 14]).toBe(0);
    expect(r).toEqual({ x: 7, y: 7, w: 6, h: 6 });
  });

  it('stampClipsAtEdges', () => {
    const a = area(10, 10);
    const r = stampLine(a, -5, -5, 2, 2, 4, 255);
    expect(r.x).toBe(0);
    expect(r.y).toBe(0);
    expect(r.w).toBeLessThanOrEqual(10);
    expect(a.data.length).toBe(100);
    const off = stampLine(a, 50, 50, 60, 60, 2, 255);
    expect(off.w * off.h).toBe(0);
  });

  it('eraserClears', () => {
    const a = area(30, 30);
    stampLine(a, 5, 15, 25, 15, 4, 255);
    stampLine(a, 15, 15, 15, 15, 2, 0);
    expect(a.data[15 * 30 + 15]).toBe(0);
    expect(a.data[15 * 30 + 6]).toBe(255);
  });

  it('dirty rect of one stroke segment on a 4096x2731 area stays small', () => {
    const a = area(4096, 2731);
    const r = stampLine(a, 1000, 1000, 1010, 1004, 10, 255);
    expect(r.w * r.h).toBeLessThan(40 * 40);
    const changed = a.data.reduce((n, v) => n + (v ? 1 : 0), 0);
    expect(changed).toBeGreaterThan(0);
    expect(changed).toBeLessThanOrEqual(r.w * r.h);
  });

  it('fillRectNormalisesCorners', () => {
    const a = area(10, 10);
    const r = fillRect(a, { x: 6, y: 6, w: -4, h: -3 }, 255); // drag up-left
    expect(r).toEqual({ x: 2, y: 3, w: 4, h: 3 });
    for (let y = 0; y < 10; y++)
      for (let x = 0; x < 10; x++) {
        const inside = x >= 2 && x < 6 && y >= 3 && y < 6;
        expect(a.data[y * 10 + x]).toBe(inside ? 255 : 0);
      }
    expect(fillRect(a, { x: 8, y: 8, w: 10, h: 10 }, 255)).toEqual({ x: 8, y: 8, w: 2, h: 2 });
  });

  it('overlayOnlyInDirtyRect', () => {
    const a = area(6, 4);
    a.data.fill(255);
    const out = { data: new Uint8ClampedArray(6 * 4 * 4).fill(9), width: 6, height: 4 };
    renderOverlay(a, out, { x: 1, y: 1, w: 2, h: 2 });
    for (let y = 0; y < 4; y++)
      for (let x = 0; x < 6; x++) {
        const o = (y * 6 + x) * 4;
        const px = Array.from(out.data.subarray(o, o + 4));
        const inside = x >= 1 && x < 3 && y >= 1 && y < 3;
        expect(px).toEqual(inside ? [255, 59, 48, 115] : [9, 9, 9, 9]);
      }
    a.data[1 * 6 + 1] = 0;
    renderOverlay(a, out, { x: 1, y: 1, w: 1, h: 1 });
    expect(Array.from(out.data.subarray(28, 32))).toEqual([0, 0, 0, 0]);
  });

  it('isEmpty', () => {
    const a = area(5, 5);
    expect(isEmpty(a)).toBe(true);
    a.data[24] = 255;
    expect(isEmpty(a)).toBe(false);
  });
});
