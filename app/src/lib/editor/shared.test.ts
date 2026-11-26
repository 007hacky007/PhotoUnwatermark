import { describe, expect, it } from 'vitest';
import type { StripPlacement } from '../types';
import { fillRect, stampLine, type Area } from './paint';
import { IDENTITY, SharedDrawing } from './shared';

const area = (w: number, h: number): Area => ({ w, h, data: new Uint8Array(w * h) });
const found = (scaleX: number, scaleY: number, dx: number, dy: number): StripPlacement => ({
  status: 'found',
  scaleX,
  scaleY,
  dx,
  dy,
});
const set = (a: Area, x: number, y: number) => (a.data[y * a.w + x] = 255);
const at = (a: Area, x: number, y: number) => a.data[y * a.w + x];
const count = (a: Area) => a.data.reduce((n, v) => n + (v !== 0 ? 1 : 0), 0);

describe('SharedDrawing', () => {
  it('identityViewIsACopy', () => {
    const a = area(8, 6);
    set(a, 2, 3);
    set(a, 7, 5);
    const d = new SharedDrawing(a);
    const v = d.viewOn(IDENTITY, 8, 6);
    expect(v.w).toBe(8);
    expect(v.h).toBe(6);
    expect(v.data).toEqual(a.data);
    expect(v.data).not.toBe(a.data);
    v.data.fill(255);
    expect(count(d.area)).toBe(2);
  });

  it('viewOnMapsThroughScaleAndShift', () => {
    const a = area(10, 10);
    set(a, 2, 3);
    const v = new SharedDrawing(a).viewOn(found(2, 2, 5, 1), 30, 30);
    for (let y = 0; y < 30; y++) {
      for (let x = 0; x < 30; x++) {
        const inside = x >= 9 && x <= 10 && y >= 7 && y <= 8;
        expect(at(v, x, y), `(${x}, ${y})`).toBe(inside ? 255 : 0);
      }
    }
  });

  it('viewOnIsEmptyForNotFound', () => {
    const a = area(10, 10);
    a.data.fill(255);
    const v = new SharedDrawing(a).viewOn({ status: 'notFound', scaleX: 0, scaleY: 0, dx: 0, dy: 0 }, 12, 9);
    expect(v.w).toBe(12);
    expect(v.h).toBe(9);
    expect(count(v)).toBe(0);
  });

  it('carryBackAtIdentityIsTheStroke', () => {
    const d = new SharedDrawing(area(40, 30));
    const v = d.viewOn(IDENTITY, 40, 30);
    const r = stampLine(v, 5, 5, 30, 20, 4, 255);
    expect(d.carryBack(IDENTITY, v, r)).toBe(true);
    expect(d.area.data).toEqual(v.data);
    expect(d.canUndo()).toBe(true);
    expect(d.revision).toBe(1);
    // The same stroke again changes nothing: no step, the same revision.
    expect(d.carryBack(IDENTITY, v, r)).toBe(false);
    expect(d.revision).toBe(1);
  });

  it('carryBackKeepsPartsOutsideTheView', () => {
    // A block at drawing x 30-39 lands at view x -10 to -1, just left of a view that shows
    // drawing x 40-89.
    const a = area(100, 20);
    fillRect(a, { x: 30, y: 0, w: 10, h: 20 }, 255);
    const d = new SharedDrawing(a);
    const p = found(1, 1, -40, 0);
    const outside = () => {
      for (let y = 0; y < 20; y++) for (let x = 30; x < 40; x++) expect(at(d.area, x, y), `(${x}, ${y})`).toBe(255);
    };
    let v = d.viewOn(p, 50, 20);
    expect(count(v)).toBe(0);
    expect(d.carryBack(p, v, stampLine(v, 0, 10, 10, 10, 5, 255))).toBe(true);
    outside();
    expect(at(d.area, 45, 10)).toBe(255);
    v = d.viewOn(p, 50, 20);
    expect(d.carryBack(p, v, fillRect(v, { x: 0, y: 0, w: 50, h: 20 }, 0))).toBe(true);
    outside();
    for (let x = 40; x < 90; x++) expect(at(d.area, x, 10)).toBe(0);
  });

  it('roundTripAtUnevenScale', () => {
    const d = new SharedDrawing(area(200, 200));
    const p = found(0.5, 0.75, 3, -2);
    const v = d.viewOn(p, 100, 150);
    const disc = area(100, 150);
    stampLine(disc, 50, 75, 50, 75, 10, 255);
    const r = stampLine(v, 50, 75, 50, 75, 10, 255);
    expect(d.carryBack(p, v, r)).toBe(true);
    const again = d.viewOn(p, 100, 150);
    let covered = 0;
    let extra = 0;
    for (let i = 0; i < disc.data.length; i++) {
      if (disc.data[i] && again.data[i]) covered++;
      if (!disc.data[i] && again.data[i]) extra++;
    }
    const n = count(disc);
    expect(n).toBeGreaterThan(250);
    expect(covered / n).toBeGreaterThanOrEqual(0.9);
    expect(extra / n).toBeLessThanOrEqual(0.1);
  });

  it('undoRedoAcrossPhotos', () => {
    const d = new SharedDrawing(area(20, 20));
    const original = d.area.data.slice();
    const p1 = IDENTITY;
    const p2 = found(1, 1, 5, 0);
    const revisions = [d.revision];
    const v1 = d.viewOn(p1, 20, 20);
    expect(d.carryBack(p1, v1, fillRect(v1, { x: 0, y: 0, w: 3, h: 3 }, 255))).toBe(true);
    const first = d.area.data.slice();
    revisions.push(d.revision);
    const v2 = d.viewOn(p2, 20, 20);
    expect(d.carryBack(p2, v2, fillRect(v2, { x: 10, y: 10, w: 3, h: 3 }, 255))).toBe(true);
    revisions.push(d.revision);
    // The second stroke landed 5 px to the left in the drawing.
    expect(at(d.area, 5, 10)).toBe(255);
    expect(at(d.area, 10, 10)).toBe(0);
    expect(d.undo()).toBe(true);
    revisions.push(d.revision);
    expect(d.undo()).toBe(true);
    revisions.push(d.revision);
    expect(d.area.data).toEqual(original);
    expect(d.canUndo()).toBe(false);
    expect(d.undo()).toBe(false);
    expect(d.redo()).toBe(true);
    revisions.push(d.revision);
    expect(d.area.data).toEqual(first);
    expect(d.canRedo()).toBe(true);
    expect(new Set(revisions).size).toBe(revisions.length);
  });

  it('notFoundPlacementCarriesNothing', () => {
    const d = new SharedDrawing(area(20, 20));
    for (const p of [
      { status: 'notFound', scaleX: 0, scaleY: 0, dx: 0, dy: 0 },
      { status: 'unsure', scaleX: 1, scaleY: 1, dx: 0, dy: 0 },
    ] as StripPlacement[]) {
      const v = area(20, 20);
      const r = fillRect(v, { x: 0, y: 0, w: 20, h: 20 }, 255);
      expect(d.carryBack(p, v, r)).toBe(false);
    }
    expect(count(d.area)).toBe(0);
    expect(d.revision).toBe(0);
    expect(d.canUndo()).toBe(false);
  });
});
