import { describe, expect, it } from 'vitest';
import { actionFor } from './keys';

const k = (key: string, o: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }> = {}) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  ...o,
});

describe('keys', () => {
  it('keysMacAndWindows', () => {
    expect(actionFor(k('z', { metaKey: true }), true)).toBe('undo');
    expect(actionFor(k('Z', { metaKey: true, shiftKey: true }), true)).toBe('redo');
    expect(actionFor(k('z', { ctrlKey: true }), true)).toBeNull();
    expect(actionFor(k('y', { ctrlKey: true }), true)).toBeNull();
    expect(actionFor(k('z', { ctrlKey: true }), false)).toBe('undo');
    expect(actionFor(k('Z', { ctrlKey: true, shiftKey: true }), false)).toBe('redo');
    expect(actionFor(k('y', { ctrlKey: true }), false)).toBe('redo');
    expect(actionFor(k('z', { metaKey: true }), false)).toBeNull();
    expect(actionFor(k('z'), true)).toBeNull();
    expect(actionFor(k('z'), false)).toBeNull();
  });

  it('maps the tool, size, loupe and zoom keys', () => {
    const plain: [string, string][] = [
      ['b', 'brush'], ['B', 'brush'], ['e', 'eraser'], ['r', 'rect'], ['m', 'toggleLoupe'],
      ['[', 'sizeDown'], [']', 'sizeUp'], ['0', 'fit'], ['1', 'actualSize'],
      ['+', 'zoomIn'], ['=', 'zoomIn'], ['-', 'zoomOut'],
    ];
    for (const [key, action] of plain) expect(actionFor(k(key), true)).toBe(action);
    expect(actionFor(k('x'), true)).toBeNull();
    expect(actionFor(k('b', { metaKey: true }), true)).toBeNull();
    expect(actionFor(k('b', { ctrlKey: true }), false)).toBeNull();
  });
});
