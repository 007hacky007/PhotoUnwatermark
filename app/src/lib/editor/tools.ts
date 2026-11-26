export type Tool = 'move' | 'brush' | 'eraser' | 'rect';

/** Brush and eraser size: the diameter in photo pixels (at edit size). */
export const MIN_SIZE = 2;
export const MAX_SIZE = 200;
export const DEFAULT_SIZE = 24;

/** The next size up (`dir` 1) or down (-1) for the [ and ] keys: about 25 % per press, at least 1 px. */
export function stepSize(size: number, dir: 1 | -1): number {
  const next = dir > 0 ? Math.max(size + 1, Math.round(size * 1.25)) : Math.min(size - 1, Math.round(size / 1.25));
  return Math.min(MAX_SIZE, Math.max(MIN_SIZE, next));
}
