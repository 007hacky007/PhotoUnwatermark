import { describe, expect, it } from 'vitest';
import { actionFor, isTextField, repeats } from './keys';

const k = (key: string, o: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
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

  it('treats AltGr (ctrl+alt) as a plain key and ignores plain Alt', () => {
    expect(actionFor(k('[', { ctrlKey: true, altKey: true }), false)).toBe('sizeDown');
    expect(actionFor(k(']', { ctrlKey: true, altKey: true }), false)).toBe('sizeUp');
    expect(actionFor(k('z', { ctrlKey: true, altKey: true }), false)).toBeNull();
    expect(actionFor(k('b', { altKey: true }), false)).toBeNull();
    expect(actionFor(k('b', { altKey: true }), true)).toBeNull();
    expect(actionFor(k('[', { altKey: true }), true)).toBeNull();
  });
});

describe('key helpers', () => {
  it('onlyStepActionsRepeat', () => {
    for (const a of ['sizeDown', 'sizeUp', 'zoomIn', 'zoomOut', 'undo', 'redo'] as const) expect(repeats(a), a).toBe(true);
    for (const a of ['brush', 'eraser', 'rect', 'toggleLoupe', 'fit', 'actualSize'] as const) expect(repeats(a), a).toBe(false);
  });

  it('textFieldsKeepTheirKeys', () => {
    const make = (html: string) => {
      const box = document.createElement('div');
      box.innerHTML = html;
      return box.firstElementChild;
    };
    expect(isTextField(make('<input type="text">'))).toBe(true);
    expect(isTextField(make('<textarea></textarea>'))).toBe(true);
    expect(isTextField(make('<input type="range">'))).toBe(false);
    expect(isTextField(make('<button>B</button>'))).toBe(false);
    expect(isTextField(window)).toBe(false);
    expect(isTextField(null)).toBe(false);
  });
});
