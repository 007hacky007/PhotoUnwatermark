import { describe, expect, it, vi } from 'vitest';

type Handler = (event: { preventDefault: () => void }) => void;
let handler: Handler = () => {};
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: vi.fn(async (h: Handler) => {
      handler = h;
      return () => {};
    }),
  }),
}));

import { onCloseRequested } from '../lib/api';

function close(): boolean {
  const preventDefault = vi.fn();
  handler({ preventDefault });
  return preventDefault.mock.calls.length > 0;
}

describe('api', () => {
  it('closeIsHeldOnlyWhenAsked', async () => {
    let hold = true;
    await onCloseRequested(() => hold);
    expect(close()).toBe(true);
    hold = false;
    expect(close()).toBe(false);
  });

  it('closeIsNotHeldWhenTheCheckFails', async () => {
    await onCloseRequested(() => {
      throw new Error('gone');
    });
    expect(close()).toBe(false);
  });
});
