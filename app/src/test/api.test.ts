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
import { firstTicket, ticketOf } from '../lib/tickets';

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

  it('eachCallHasItsOwnTicketAndAStopNamesIt', async () => {
    invoke.mockClear();
    const suggest = suggestSaved();
    const find = detect(() => {});
    await Promise.all([suggest, find]);
    const ticketOfCall = (i: number) => (invoke.mock.calls[i][1] as { job: number }).job;
    expect(ticketOf(suggest)).toBe(ticketOfCall(0));
    expect(ticketOf(find)).toBe(ticketOfCall(1));
    expect(ticketOf(find)).toBeGreaterThan(ticketOf(suggest)!);
    // A stop names the ticket it is given, whatever started since.
    const later = matchAll(() => {});
    await cancelJob(ticketOf(suggest));
    expect(invoke.mock.calls.at(-1)).toEqual(['cancel_job', { job: ticketOf(suggest) }]);
    await later;
  });

  it('ticketsOfANewPageNeverRepeatOldOnes', () => {
    // A page loaded a second later starts above every ticket the first one could have used
    // within 1000 jobs per second of its life.
    expect(firstTicket(1_800_000_000_000)).toBe(1_800_000_000_000_000);
    expect(firstTicket(1_800_000_001_000)).toBeGreaterThan(firstTicket(1_800_000_000_000) + 999);
    expect(Number.isSafeInteger(firstTicket(Date.UTC(2200, 0, 1)) + 1_000_000)).toBe(true);
    // This page's tickets start from its load time.
    invoke.mockClear();
    const call = suggestSaved();
    expect(ticketOf(call)).toBeGreaterThan(firstTicket(Date.now() - 60_000));
  });

  it('closeIsNotHeldWhenTheCheckFails', async () => {
    await onCloseRequested(() => {
      throw new Error('gone');
    });
    expect(close()).toBe(false);
  });
});
