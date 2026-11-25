import { describe, expect, it } from 'vitest';
import { History } from './history';
import { fillRect, stampLine, type Area } from './paint';

const area = (w: number, h: number): Area => ({ w, h, data: new Uint8Array(w * h) });

function stroke(h: History, a: Area, x: number, y: number) {
  const r = stampLine(a, x, y, x + 5, y, 3, 255);
  h.commit(a, r);
  return r;
}

describe('History', () => {
  it('undoRedoRestoresBytes', () => {
    const a = area(60, 40);
    const h = new History();
    h.begin(a);
    const s0 = a.data.slice();
    stroke(h, a, 10, 10);
    const s1 = a.data.slice();
    stroke(h, a, 30, 20);
    const s2 = a.data.slice();
    expect(h.canUndo).toBe(true);
    expect(h.canRedo).toBe(false);
    h.undo(a);
    expect(a.data).toEqual(s1);
    h.undo(a);
    expect(a.data).toEqual(s0);
    expect(h.canUndo).toBe(false);
    expect(h.undo(a)).toBeNull();
    h.redo(a);
    expect(a.data).toEqual(s1);
    h.redo(a);
    expect(a.data).toEqual(s2);
    expect(h.redo(a)).toBeNull();
  });

  it('undo returns only the dirty rect on a 4096x2731 area', () => {
    const a = area(4096, 2731);
    const h = new History();
    h.begin(a);
    const r = stroke(h, a, 2000, 1500);
    const u = h.undo(a)!;
    expect(u).toEqual(r);
    expect(u.w * u.h).toBeLessThan(30 * 30);
    expect(a.data.some((v) => v)).toBe(false);
    expect(h.redo(a)).toEqual(r);
  });

  it('restores erased pixels over earlier strokes', () => {
    const a = area(40, 40);
    const h = new History();
    h.begin(a);
    h.commit(a, fillRect(a, { x: 5, y: 5, w: 20, h: 20 }, 255));
    const painted = a.data.slice();
    h.commit(a, stampLine(a, 15, 15, 15, 15, 4, 0));
    expect(a.data).not.toEqual(painted);
    h.undo(a);
    expect(a.data).toEqual(painted);
  });

  it('historyCapDropsOldest', () => {
    const a = area(1000, 20);
    const h = new History(100);
    h.begin(a);
    for (let i = 0; i < 105; i++) stroke(h, a, 10 + i * 8, 10);
    let n = 0;
    while (h.undo(a)) n++;
    expect(n).toBe(100);
    // The 5 oldest strokes could not be undone.
    expect(a.data[10 * 1000 + 12]).toBe(255);

    const small = new History(100, 300);
    const b = area(100, 20);
    small.begin(b);
    for (let i = 0; i < 20; i++) stroke(small, b, 5 + i * 4, 10);
    let m = 0;
    while (small.undo(b)) m++;
    expect(m).toBeLessThan(20);
    expect(m).toBeGreaterThan(0);
  });

  it('newStrokeClearsRedo', () => {
    const a = area(60, 40);
    const h = new History();
    h.begin(a);
    stroke(h, a, 10, 10);
    h.undo(a);
    expect(h.canRedo).toBe(true);
    stroke(h, a, 30, 20);
    expect(h.canRedo).toBe(false);
    expect(h.redo(a)).toBeNull();
  });

  it('ignores empty commits and requires begin', () => {
    const a = area(10, 10);
    const h = new History();
    expect(() => h.commit(a, { x: 0, y: 0, w: 1, h: 1 })).toThrow();
    h.begin(a);
    h.commit(a, { x: 0, y: 0, w: 0, h: 0 });
    expect(h.canUndo).toBe(false);
  });
});
