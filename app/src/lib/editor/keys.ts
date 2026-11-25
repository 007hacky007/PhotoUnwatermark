export type Action =
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
