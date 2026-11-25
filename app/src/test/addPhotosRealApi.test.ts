// Add photos over the real api.ts, with only Tauri replaced: what reaches the app, in what order
// and with which ticket. (addPhotos.test.ts replaces api.ts and cannot see tickets.)

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { PhotoInfo } from '../lib/types';

type Call = { cmd: string; args: Record<string, unknown> };
const calls: Call[] = [];
let photos: PhotoInfo[] = [];
/** Settles the pending saved-watermark check (as the app does when it is stopped). */
let endCheck: (() => void) | null = null;
let picked: string[] = [];
/** When false, a stop is taken but the check ends only when the test says so. */
let stopEndsAtOnce = true;
let checkOver: Promise<void> = Promise.resolve();
let checkDone: () => void = () => {};

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: unknown = null;
  },
  convertFileSrc: (p: string) => `puw://${p}`,
  invoke: vi.fn(async (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case 'list_photos':
        return photos;
      case 'suggest_saved':
        checkOver = new Promise((done) => (checkDone = done));
        return new Promise((_, reject) => {
          endCheck = () => {
            reject({ message: 'Stopped.', details: '' });
            checkDone();
          };
        });
      case 'cancel_job':
        if (stopEndsAtOnce) endCheck?.();
        return null;
      case 'add_paths': {
        // Like the app: a job started after a stop waits for the stopped one to end.
        await checkOver;
        const added = (args.paths as string[]).map((_, i) => photo(photos.length + i + 1));
        photos = [...photos, ...added];
        return { added, skipped: [] };
      }
      case 'detect':
        return new Promise(() => {});
      default:
        return null;
    }
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn(async () => () => {}) }),
}));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => picked) }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn(), revealItemInDir: vi.fn() }));

import App from '../App.svelte';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';

function photo(id: number): PhotoInfo {
  return { id, name: `p${id}.jpg`, folder: '/x', width: 10, height: 10 };
}

const tickets = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args.job as number);
const order = (cmd: string) => calls.findIndex((c) => c.cmd === cmd);

/** One photo in the list, a second one added, and the saved-watermark check still running. */
async function checkRunning() {
  calls.length = 0;
  photos = [photo(1)];
  endCheck = null;
  checkOver = Promise.resolve();
  const wizard = createWizard();
  wizard.go('add');
  render(App, { wizard });
  await screen.findByText('1 photo');
  picked = ['/a/2.jpg'];
  await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
  await screen.findByText(copy.addPhotos.checkingSaved);
  const [check] = tickets('suggest_saved');
  expect(check).toBeTypeOf('number');
  calls.length = 0;
  return { wizard, check };
}

describe('Add photos, real tickets', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    stopEndsAtOnce = true;
  });

  it('addingStopsTheCheckByItsTicketFirst', async () => {
    const { check } = await checkRunning();
    picked = ['/a/3.jpg'];
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await screen.findByText('3 photos');
    const [add] = tickets('add_paths');
    // The stop names the check, and reaches the app before the new job starts.
    expect(tickets('cancel_job')).toEqual([check]);
    expect(order('cancel_job')).toBeLessThan(order('add_paths'));
    expect(add).not.toBe(check);
    // A new check runs for the new list; nothing stopped the add.
    expect(tickets('suggest_saved')).toHaveLength(1);
    expect(tickets('cancel_job')).not.toContain(add);
  });

  it('findStopsTheCheckByItsTicketFirst', async () => {
    const { check } = await checkRunning();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await waitFor(() => expect(tickets('detect')).toHaveLength(1));
    const [find] = tickets('detect');
    expect(tickets('cancel_job')).toEqual([check]);
    expect(order('cancel_job')).toBeLessThan(order('detect'));
    expect(find).not.toBe(check);
    await screen.findByText(copy.loading.finder);
    expect(tickets('cancel_job')).toEqual([check]);
  });

  it('backFromFindingStopsTheSearchByItsTicket', async () => {
    const { check } = await checkRunning();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await waitFor(() => expect(tickets('detect')).toHaveLength(1));
    const [find] = tickets('detect');
    await fireEvent.click(await screen.findByRole('button', { name: copy.common.back }));
    await screen.findByText(copy.addPhotos.drop);
    expect(tickets('cancel_job')).toEqual([check, find]);
  });

  it('waitingForAStoppedJobSaysSo', async () => {
    await checkRunning();
    stopEndsAtOnce = false;
    picked = ['/a/3.jpg'];
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    // The app finishes the stopped check before it adds (it waits, like the real app).
    expect(await screen.findByText(copy.addPhotos.waitingForStop)).toBeTruthy();
    endCheck?.();
    await screen.findByText('3 photos');
    expect(screen.queryByText(copy.addPhotos.waitingForStop)).toBeNull();
  });
});
