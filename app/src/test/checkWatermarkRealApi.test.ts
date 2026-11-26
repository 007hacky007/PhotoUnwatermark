// Step 2 over the real api.ts, with only Tauri replaced by a stand-in for the app's one job
// slot: a job sent while another runs that has not been stopped is refused as busy, and a job
// sent after a stop waits for the stopped one to end. The app takes a stop only when the test
// says so, as cancel_job and the next command may arrive in either order.

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

type Call = { cmd: string; args: Record<string, unknown> };
const calls: Call[] = [];
type Running = { ticket: number; stopped: boolean; end: () => void };
let running: Running | null = null;
/** Stops the app has received but not taken yet. */
let stops: (() => void)[] = [];

const W = 4;
const H = 3;

/** Runs `body` as a job with ticket `ticket`, like Jobs::start in the app. */
async function asJob<T>(ticket: number, body: (self: Running) => Promise<T>): Promise<T> {
  if (running && !running.stopped) throw { message: 'Another job is running.', details: '' };
  while (running) await new Promise((r) => setTimeout(r, 0));
  const self: Running = { ticket, stopped: false, end: () => {} };
  running = self;
  try {
    return await body(self);
  } finally {
    running = null;
  }
}

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: unknown = null;
  },
  convertFileSrc: (p: string) => `puw://${p}`,
  invoke: vi.fn(async (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case 'edit_size':
        return { width: W, height: H };
      case 'start_area': {
        const a = new Uint8Array(W * H);
        a[5] = 255;
        return a.buffer;
      }
      case 'place_on':
        return asJob(args.job as number, (self) =>
          args.id === 2
            ? // Photo 2 is slow: it ends only when stopped.
              new Promise((_, reject) => (self.end = () => reject({ message: 'Stopped.', details: '' })))
            : Promise.resolve({ status: 'found', scaleX: 1, scaleY: 1, dx: 0, dy: 0 }),
        );
      case 'clear_groups':
      case 'confirm_new_group':
      case 'confirm_set_area':
        return asJob(args.job as number, async () => null);
      case 'cancel_job':
        return new Promise<null>((resolve) =>
          stops.push(() => {
            const job = running;
            if (job && job.ticket === args.job) {
              job.stopped = true;
              job.end();
            }
            resolve(null);
          }),
        );
      default:
        return null;
    }
  }),
}));

import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import CheckWatermark from '../screens/CheckWatermark.svelte';
import { assertPlainText } from './plainText';

const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const sent = (cmd: string) => calls.filter((c) => c.cmd === cmd);
const settle = () => new Promise((r) => setTimeout(r, 20));
/** The app takes every stop it has received. */
function takeStops() {
  for (const take of stops.splice(0)) take();
}

async function ready() {
  await waitFor(() => expect(button(copy.checkWatermark.looksRight).disabled).toBe(false));
}

/** Group 1 confirmed and opened again in step 2, on a sample of three photos. */
async function open() {
  const wizard = createWizard();
  wizard.go('add');
  wizard.startDraft({ sample: [1, 2, 3], representative: 1 });
  wizard.confirmDraft();
  wizard.go('checkAll');
  wizard.go('check');
  render(CheckWatermark, { wizard });
  await ready();
  return wizard;
}

/** Shows photo 2, whose placement keeps running. */
async function placingOnTwo() {
  await fireEvent.click(button(copy.checkWatermark.stripPhoto(2, 3)));
  await waitFor(() => {
    expect(sent('place_on').at(-1)?.args.id).toBe(2);
    expect(running?.ticket).toBe(sent('place_on').at(-1)?.args.job);
  });
}

describe('Step 2 over the real api', () => {
  beforeEach(() => {
    calls.length = 0;
    running = null;
    stops = [];
  });

  it('theNextPlacementWaitsUntilTheStopIsTaken', async () => {
    await open();
    await placingOnTwo();
    const slow = sent('place_on')[0].args.job;
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(3, 3)));
    await waitFor(() => expect(sent('cancel_job')).toHaveLength(1));
    expect(sent('cancel_job')[0].args.job).toBe(slow);
    await settle();
    expect(sent('place_on')).toHaveLength(1);
    takeStops();
    await ready();
    expect(sent('place_on').at(-1)?.args.id).toBe(3);
    expect(screen.queryByText(copy.checkWatermark.placeFailed)).toBeNull();
    expect(screen.queryByText('Another job is running.')).toBeNull();
  });

  it('backClearsTheGroupsOnlyAfterTheStopIsTaken', async () => {
    const wizard = await open();
    await placingOnTwo();
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(sent('cancel_job')).toHaveLength(1));
    await settle();
    expect(sent('clear_groups')).toHaveLength(0);
    takeStops();
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(sent('clear_groups')).toHaveLength(1);
    expect(wizard.groups).toBe(0);
  });

  it('noBannedWordsRendered', async () => {
    await open();
    await placingOnTwo();
    expect(screen.getByText(copy.checkWatermark.placing)).toBeTruthy();
    assertPlainText(document.body);
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(3, 3)));
    takeStops();
    await ready();
    assertPlainText(document.body);
  });
});
