import { describe, expect, it } from 'vitest';
import { MAX_SIZE, MIN_SIZE, stepSize } from './tools';

describe('tools', () => {
  it('stepSizeMovesAndClamps', () => {
    expect(stepSize(2, 1)).toBe(3);
    expect(stepSize(24, 1)).toBe(30);
    expect(stepSize(30, -1)).toBe(24);
    expect(stepSize(3, -1)).toBe(2);
    expect(stepSize(MIN_SIZE, -1)).toBe(MIN_SIZE);
    expect(stepSize(190, 1)).toBe(MAX_SIZE);
    expect(stepSize(MAX_SIZE, 1)).toBe(MAX_SIZE);
  });
});
