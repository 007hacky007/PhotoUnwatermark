import { afterEach, beforeEach, expect, vi } from 'vitest';

// Svelte reports misuse (and the tests' own mistakes) through console warnings and errors:
// any of them fails the test that caused it, so the test output stays clean.
beforeEach(() => {
  vi.spyOn(console, 'warn');
  vi.spyOn(console, 'error');
});

afterEach(() => {
  expect(console.warn, 'console.warn was called').not.toHaveBeenCalled();
  expect(console.error, 'console.error was called').not.toHaveBeenCalled();
  vi.restoreAllMocks();
});
