export type Action =
  | 'move'
  | 'brush'
  | 'eraser'
  | 'rect'
  | 'sizeDown'
  | 'sizeUp'
  | 'toggleLoupe'
  | 'undo'
  | 'redo'
  | 'fit'
  | 'actualSize'
  | 'zoomIn'
  | 'zoomOut';

type KeyEvent = { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean };

export function actionFor(e: KeyEvent, isMac: boolean): Action | null {
  const key = e.key.toLowerCase();
  // AltGr reports ctrlKey + altKey on Windows: such a key is a plain typed character.
  const altGr = e.altKey && e.ctrlKey;
  if (e.altKey && !altGr) return null;
  const ctrl = e.ctrlKey && !altGr;
  const mod = isMac ? e.metaKey : ctrl;
  const other = isMac ? ctrl : e.metaKey;
  if (other) return null;
  if (mod) {
    if (key === 'z') return e.shiftKey ? 'redo' : 'undo';
    if (key === 'y' && !isMac && !e.shiftKey) return 'redo';
    return null;
  }
  switch (key) {
    case 'h': return 'move';
    case 'b': return 'brush';
    case 'e': return 'eraser';
    case 'r': return 'rect';
    case 'm': return 'toggleLoupe';
    case '[': return 'sizeDown';
    case ']': return 'sizeUp';
    case '0': return 'fit';
    case '1': return 'actualSize';
    case '+':
    case '=': return 'zoomIn';
    case '-': return 'zoomOut';
    default: return null;
  }
}

const REPEATING = new Set<Action>(['sizeDown', 'sizeUp', 'zoomIn', 'zoomOut', 'undo', 'redo']);

/** Whether holding the key down keeps acting; the others act once per press. */
export function repeats(action: Action): boolean {
  return REPEATING.has(action);
}

/** True when `target` takes typed text, so letter shortcuts must leave the keys to it. */
export function isTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT') return true;
  return target instanceof HTMLInputElement && target.type !== 'range' && target.type !== 'checkbox';
}

/**
 * For `onmousedown` on the editor screen's buttons: a mouse click then leaves the focus where
 * it was, so Space keeps hiding the red layer instead of pressing the last button clicked.
 * Keyboard users still reach the buttons with Tab, and Space presses a button focused that way.
 */
export function keepFocus(e: MouseEvent): void {
  e.preventDefault();
}
