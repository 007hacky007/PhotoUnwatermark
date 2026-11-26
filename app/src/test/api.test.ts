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

import {
  cancelJob,
  clearGroups,
  confirmNewGroup,
  detect,
  getStartArea,
  groupSample,
  groupsState,
  matchAll,
  onCloseRequested,
  placeOn,
  saveWatermark,
  suggestSaved,
  useSaved,
} from '../lib/api';
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

  it('groupCallsHaveTheirOwnTickets', async () => {
    invoke.mockClear();
    const calls = [placeOn(1, 2), confirmNewGroup(3), clearGroups(), useSaved('s1')];
    await Promise.all(calls);
    const tickets = calls.map((c) => ticketOf(c));
    expect(new Set(tickets).size).toBe(4);
    for (const t of tickets) expect(typeof t).toBe('number');
    expect(invoke.mock.calls).toEqual([
      ['place_on', { reference: 1, id: 2, job: tickets[0] }],
      ['confirm_new_group', { id: 3, job: tickets[1] }],
      ['clear_groups', { job: tickets[2] }],
      ['use_saved', { id: 's1', job: tickets[3] }],
    ]);
  });

  it('groupReadsArePlainCalls', async () => {
    invoke.mockClear();
    const groups = groupsState();
    const sample = groupSample();
    const save = saveWatermark(2, 'Studio');
    await Promise.all([groups, sample, save]);
    expect(ticketOf(groups)).toBeUndefined();
    expect(ticketOf(sample)).toBeUndefined();
    expect(invoke.mock.calls).toEqual([['groups'], ['group_sample'], ['save_watermark', { group: 2, name: 'Studio' }]]);
    invoke.mockResolvedValueOnce(new Uint8Array([0, 255]).buffer as never);
    expect(await getStartArea(4)).toEqual(new Uint8Array([0, 255]));
    expect(invoke.mock.calls.at(-1)).toEqual(['start_area', { id: 4 }]);
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
