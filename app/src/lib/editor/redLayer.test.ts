import { describe, expect, it, vi } from 'vitest';
import { stampLine, type Area } from './paint';
import { RedLayer } from './redLayer';

function layer(w = 40, h = 30) {
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d')!;
  const put = vi.spyOn(ctx, 'putImageData');
  const onDrawn = vi.fn();
  const red = new RedLayer(canvas, onDrawn);
  const area: Area = { w, h, data: new Uint8Array(w * h) };
  red.set(area);
  return { canvas, put, onDrawn, red, area };
}

describe('RedLayer', () => {
  it('setDrawsTheWholeLayerOnce', () => {
    const { canvas, put, onDrawn } = layer();
    expect([canvas.width, canvas.height]).toEqual([40, 30]);
    expect(put).toHaveBeenCalledOnce();
    expect(put.mock.calls[0].slice(1)).toEqual([0, 0, 0, 0, 40, 30]);
    expect(onDrawn).toHaveBeenCalledOnce();
  });

  it('redrawsOnlyTheDirtyRect', () => {
    const { put, red, area } = layer();
    put.mockClear();
    red.begin();
    const a = stampLine(area, 5, 5, 5, 5, 2, 255);
    const b = stampLine(area, 12, 8, 12, 8, 2, 255);
    red.queue(a);
    red.queue(b);
    expect(put).not.toHaveBeenCalled();
    red.commit({ x: 3, y: 3, w: 12, h: 8 });
    expect(put).toHaveBeenCalledOnce();
    // One draw of the union of the two stamps, nowhere near the whole 40 x 30 layer.
    expect(put.mock.calls[0].slice(1)).toEqual([0, 0, a.x, a.y, b.x + b.w - a.x, b.y + b.h - a.y]);
    const image = put.mock.calls[0][0];
    expect(image.data[(5 * 40 + 5) * 4 + 3]).toBe(115);
    expect(image.data[(20 * 40 + 30) * 4 + 3]).toBe(0);
  });

  it('undoAndRedoRedrawTheStepOnly', () => {
    const { put, red, area } = layer();
    red.begin();
    const r = stampLine(area, 20, 15, 22, 15, 2, 255);
    red.commit(r);
    put.mockClear();
    expect(red.undo()).toBe(true);
    expect(area.data.every((v) => v === 0)).toBe(true);
    expect(put.mock.calls[0].slice(3)).toEqual([r.x, r.y, r.w, r.h]);
    expect(red.redo()).toBe(true);
    expect(area.data[15 * 40 + 21]).toBe(255);
    expect(red.redo()).toBe(false);
  });

  it('setStartsTheHistoryAfresh', () => {
    const { red, area } = layer();
    red.begin();
    red.commit(stampLine(area, 5, 5, 5, 5, 2, 255));
    expect(red.history.canUndo).toBe(true);
    red.set({ w: 10, h: 10, data: new Uint8Array(100) });
    expect(red.history.canUndo).toBe(false);
    expect(red.undo()).toBe(false);
  });
});
