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

const invoke = vi.fn(async (..._args: unknown[]) => null);
vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: unknown = null;
  },
  convertFileSrc: (p: string) => p,
  invoke: (...args: unknown[]) => invoke(...args),
}));

import { cancelJob, detect, matchAll, onCloseRequested, suggestSaved } from '../lib/api';

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

  it('aStopNamesTheJobStartedLast', async () => {
    invoke.mockClear();
    await suggestSaved();
    await detect(() => {});
    const ticketOf = (i: number) => (invoke.mock.calls[i][1] as { job: number }).job;
    const [suggest, find] = [ticketOf(0), ticketOf(1)];
    expect(find).toBeGreaterThan(suggest);
    await cancelJob();
    expect(invoke.mock.calls[2]).toEqual(['cancel_job', { job: find }]);
    // A stop sent before the next job starts keeps naming the job it was meant for.
    const stop = cancelJob();
    await matchAll(() => {});
    await stop;
    expect(invoke.mock.calls[3]).toEqual(['cancel_job', { job: find }]);
    expect(ticketOf(4)).toBeGreaterThan(find);
  });

  it('closeIsNotHeldWhenTheCheckFails', async () => {
    await onCloseRequested(() => {
      throw new Error('gone');
    });
    expect(close()).toBe(false);
  });
});
