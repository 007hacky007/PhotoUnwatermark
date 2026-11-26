import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProgressEvent, RemoveReport } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  deviceStatus: vi.fn(),
  deviceFix: vi.fn(),
  checkOutput: vi.fn(),
  setOutputOverride: vi.fn(),
  chooseFolder: vi.fn(),
  remove: vi.fn(),
  cancelJob: vi.fn(),
  saveWatermark: vi.fn(),
  defaultWatermarkName: vi.fn(),
  clearPhotos: vi.fn(),
  revealItemInDir: vi.fn(),
  onCloseRequested: vi.fn(),
  onQuitRequested: vi.fn(),
  closeWindow: vi.fn(),
  imageUrl: vi.fn(),
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { createWizard, type Wizard } from '../lib/wizard.svelte';
import Done from '../screens/Done.svelte';
import Remove from '../screens/Remove.svelte';
import { assertPlainText } from './plainText';

const m = {
  appStatus: vi.mocked(api.appStatus),
  deviceStatus: vi.mocked(api.deviceStatus),
  checkOutput: vi.mocked(api.checkOutput),
  setOutputOverride: vi.mocked(api.setOutputOverride),
  chooseFolder: vi.mocked(api.chooseFolder),
  remove: vi.mocked(api.remove),
  cancelJob: vi.mocked(api.cancelJob),
  saveWatermark: vi.mocked(api.saveWatermark),
  defaultWatermarkName: vi.mocked(api.defaultWatermarkName),
  clearPhotos: vi.mocked(api.clearPhotos),
  revealItemInDir: vi.mocked(api.revealItemInDir),
  onCloseRequested: vi.mocked(api.onCloseRequested),
  onQuitRequested: vi.mocked(api.onQuitRequested),
  closeWindow: vi.mocked(api.closeWindow),
  imageUrl: vi.mocked(api.imageUrl),
};

/** The removal the screen started: its events, and how it ends. */
interface Job {
  emit: (e: ProgressEvent) => void;
  finish: (report: RemoveReport) => void;
  fail: (message: string) => void;
}

function pendingRemove(): Job {
  const job: Job = { emit: () => {}, finish: () => {}, fail: () => {} };
  m.remove.mockImplementation((cb) => {
    job.emit = cb;
    return new Promise<RemoveReport>((resolve, reject) => {
      job.finish = resolve;
      job.fail = (message) => reject({ message, details: '' });
    });
  });
  return job;
}

/** The close handler the screen registered (true holds the window open), and its stop function. */
let closeHandler: (() => boolean) | null;
let unlisten: ReturnType<typeof vi.fn>;
/** The handler for a request to quit the app (Cmd+Q), and its stop function. */
let quitHandler: (() => void) | null;
let unlistenQuit: ReturnType<typeof vi.fn>;

const status = (platform: string) => ({ modelsReady: true, missingBytes: 0, platform, version: '0.1.0' });

function stepFour(platform = 'windows'): Wizard {
  m.appStatus.mockResolvedValue(status(platform));
  const wizard = createWizard();
  wizard.go('add');
  wizard.go('check');
  wizard.go('checkAll');
  wizard.go('remove');
  render(Remove, { wizard });
  return wizard;
}

const report = (r: Partial<RemoveReport> = {}): RemoveReport => ({ done: [], failed: [], leftOut: [], cancelled: false, ...r });
const done = (id: number, output = `/photos/Without watermark/p${id}.jpg`) => ({ id, output });

function wizardWith(r: RemoveReport): Wizard {
  const wizard = createWizard();
  wizard.report = r;
  wizard.go('done');
  return wizard;
}

function stepFive(r: RemoveReport): Wizard {
  const wizard = createWizard();
  wizard.go('add');
  wizard.go('check');
  wizard.go('checkAll');
  wizard.go('remove');
  wizard.report = r;
  wizard.go('done');
  render(Done, { wizard });
  return wizard;
}

const removing = (done: number, total: number, photo: number, photoFraction: number, secondsLeft: number | null): ProgressEvent => ({
  phase: 'removing',
  done,
  total,
  photo,
  photoFraction,
  secondsLeft,
});

beforeEach(() => {
  vi.resetAllMocks();
  closeHandler = null;
  unlisten = vi.fn();
  quitHandler = null;
  unlistenQuit = vi.fn();
  m.appStatus.mockResolvedValue(status('windows'));
  m.deviceStatus.mockResolvedValue({ state: 'gpu', reason: null, details: null, fixes: [] });
  m.checkOutput.mockResolvedValue({ ok: true, unwritable: [] });
  m.setOutputOverride.mockResolvedValue();
  m.cancelJob.mockResolvedValue();
  m.clearPhotos.mockResolvedValue();
  m.revealItemInDir.mockResolvedValue();
  m.closeWindow.mockResolvedValue();
  m.defaultWatermarkName.mockResolvedValue('Watermark from 2026-11-25');
  m.saveWatermark.mockResolvedValue('w1');
  m.onCloseRequested.mockImplementation(async (h) => {
    closeHandler = h;
    return unlisten as unknown as () => void;
  });
  m.onQuitRequested.mockImplementation(async (h) => {
    quitHandler = h;
    return unlistenQuit as unknown as () => void;
  });
  m.imageUrl.mockImplementation((route, id, v) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('Remove', () => {
  it('asksForFolderWhenUnwritable', async () => {
    m.checkOutput.mockResolvedValueOnce({ ok: false, unwritable: ['/cards/A', '/cards/B'] });
    m.chooseFolder.mockResolvedValue('/Users/me/Results');
    pendingRemove();
    stepFour();
    expect(
      await screen.findByText(
        'The app cannot save next to these photos: /cards/A, /cards/B. Choose a folder for the results.',
      ),
    ).toBeTruthy();
    expect(m.remove).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Choose a folder...' }));
    await waitFor(() => expect(m.remove).toHaveBeenCalledTimes(1));
    expect(m.setOutputOverride).toHaveBeenCalledWith('/Users/me/Results');
    expect(m.checkOutput).toHaveBeenCalledTimes(2);
  });

  it('cancelledFolderPickerKeepsAsking', async () => {
    m.checkOutput.mockResolvedValue({ ok: false, unwritable: ['/cards/A'] });
    m.chooseFolder.mockResolvedValue(null);
    const wizard = stepFour();
    await fireEvent.click(await screen.findByRole('button', { name: copy.remove.chooseFolder }));
    await tick();
    expect(m.setOutputOverride).not.toHaveBeenCalled();
    expect(m.remove).not.toHaveBeenCalled();
    expect(m.onCloseRequested).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('checkAll');
  });

  it('etaTexts', () => {
    expect(copy.remove.timeLeft(null)).toBe('Working out the time left...');
    expect(copy.remove.timeLeft(90)).toBe('About 2 minutes left');
    expect(copy.remove.timeLeft(30)).toBe('Less than a minute left');
    // Minutes are rounded up; under a minute is "less than a minute".
    expect(copy.remove.timeLeft(0)).toBe('Less than a minute left');
    expect(copy.remove.timeLeft(59)).toBe('Less than a minute left');
    expect(copy.remove.timeLeft(60)).toBe('About 1 minute left');
    expect(copy.remove.timeLeft(61)).toBe('About 2 minutes left');
    expect(copy.remove.timeLeft(120)).toBe('About 2 minutes left');
    expect(copy.remove.timeLeft(121)).toBe('About 3 minutes left');
  });

  it('showsProgressTimeLeftAndThePhoto', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    expect(wizard.busy).toBe(true);
    job.emit({ phase: 'loadingModels', fraction: 0.4, which: 'remover' });
    expect(await screen.findByText(copy.loading.remover)).toBeTruthy();
    job.emit(removing(0, 4, 7, 0, null));
    const bar = await screen.findByRole('progressbar', { name: 'Photo 1 of 4' });
    expect(screen.getByText('Working out the time left...')).toBeTruthy();
    expect(bar.getAttribute('aria-valuenow')).toBe('0');
    expect(screen.getByAltText(copy.remove.currentPhoto).getAttribute('src')).toBe('puw://thumb/7');
    job.emit(removing(0, 4, 7, 0.5, null));
    await tick();
    expect(bar.getAttribute('aria-valuenow')).toBe('13');
    // The photo is finished: the bar counts it whole and the picture shows the result.
    job.emit(removing(1, 4, 7, 1, 90));
    await tick();
    expect(screen.getByRole('progressbar', { name: 'Photo 1 of 4' })).toBe(bar);
    expect(screen.getByText('About 2 minutes left')).toBeTruthy();
    expect(bar.getAttribute('aria-valuenow')).toBe('25');
    const finished = screen.getByAltText(copy.remove.finishedPhoto);
    expect(finished.getAttribute('src')).toMatch(/^puw:\/\/result\/7\?v=\d+$/);
    job.emit(removing(1, 4, 8, 0, 30));
    await tick();
    expect(screen.getByRole('progressbar', { name: 'Photo 2 of 4' })).toBe(bar);
    expect(screen.getByText('Less than a minute left')).toBeTruthy();
    expect(screen.getByAltText(copy.remove.currentPhoto).getAttribute('src')).toBe('puw://thumb/8');
  });

  it('brokenResultFallsBackToThumb', async () => {
    const job = pendingRemove();
    stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    job.emit(removing(0, 1, 3, 0, null));
    job.emit(removing(1, 1, 3, 1, 0));
    const img = await screen.findByAltText(copy.remove.finishedPhoto);
    await fireEvent.error(img);
    expect(screen.getByAltText(copy.remove.currentPhoto).getAttribute('src')).toBe('puw://thumb/3');
  });

  it('announcesPhotosThrottled', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(0);
    const job = pendingRemove();
    const { container } = render(Remove, { wizard: createWizard() });
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    const live = () => container.querySelector('[data-testid="announce"]');
    job.emit(removing(0, 9, 1, 0, null));
    await tick();
    expect(live()?.getAttribute('aria-live')).toBe('polite');
    expect(live()?.textContent).toBe('Photo 1 of 9');
    vi.setSystemTime(3000);
    job.emit(removing(1, 9, 2, 0, null));
    await tick();
    expect(live()?.textContent).toBe('Photo 1 of 9');
    vi.setSystemTime(11000);
    job.emit(removing(2, 9, 3, 0, null));
    await tick();
    expect(live()?.textContent).toBe('Photo 3 of 9');
  });

  it('stopNeedsConfirmation', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    job.emit(removing(0, 3, 1, 0.2, null));
    await fireEvent.click(await screen.findByRole('button', { name: 'Stop' }));
    const dialog = screen.getByRole('dialog', { name: 'Stop now? The photo being worked on is left unfinished.' });
    expect(dialog.textContent).toContain('Finished photos are kept.');
    expect(m.cancelJob).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Keep going' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(m.cancelJob).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Stop' }));
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByText(copy.remove.stopping)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull();
    const stopped = report({ done: [done(1)], cancelled: true });
    job.finish(stopped);
    await waitFor(() => expect(wizard.screen).toBe('done'));
    expect(wizard.report).toEqual(stopped);
    expect(wizard.busy).toBe(false);
  });

  it('fallbackNoticeShown', async () => {
    const job = pendingRemove();
    stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    job.emit(removing(0, 2, 1, 0, null));
    m.deviceStatus.mockResolvedValue({ state: 'cpu', reason: 'It stopped.', details: 'raw', fixes: [] });
    job.emit({ phase: 'fellBackToProcessor', reason: 'DirectML device removed 0x887A0005' });
    expect(
      await screen.findByText('The graphics card stopped working, so the app continues on the processor.'),
    ).toBeTruthy();
    // The chip follows the processor too.
    expect(await screen.findByText(copy.device.cpu)).toBeTruthy();
    expect(screen.queryByText('DirectML device removed 0x887A0005')).toBeNull();
    const toggle = screen.getByRole('button', { name: copy.device.showDetails });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    await fireEvent.click(toggle);
    expect(screen.getByText('DirectML device removed 0x887A0005')).toBeTruthy();
    expect(screen.getByRole('button', { name: copy.device.hideDetails }).getAttribute('aria-expanded')).toBe('true');
    // It stays for the rest of the run.
    job.emit(removing(1, 2, 2, 0, 20));
    await tick();
    expect(screen.getByText('The graphics card stopped working, so the app continues on the processor.')).toBeTruthy();
  });

  it('fallbackNoticeNamesTheNeuralEngineOnMacs', async () => {
    const job = pendingRemove();
    stepFour('macos');
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    await waitFor(() => expect(m.appStatus).toHaveBeenCalled());
    await tick();
    job.emit({ phase: 'fellBackToProcessor', reason: 'x' });
    expect(
      await screen.findByText('The Neural Engine stopped working, so the app continues on the processor.'),
    ).toBeTruthy();
  });

  it('closeWhileRunningAsksThenStopsAndQuits', async () => {
    m.checkOutput.mockResolvedValueOnce({ ok: false, unwritable: ['/x'] });
    m.chooseFolder.mockResolvedValue('/out');
    const job = pendingRemove();
    const wizard = stepFour();
    await screen.findByRole('button', { name: copy.remove.chooseFolder });
    // Nothing runs yet, so closing is not held up.
    expect(m.onCloseRequested).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.chooseFolder }));
    await waitFor(() => expect(closeHandler).not.toBeNull());
    expect(closeHandler!()).toBe(true);
    await tick();
    expect(screen.getByRole('dialog', { name: 'Removal is still running. Stop and quit?' })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Keep going' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(m.cancelJob).not.toHaveBeenCalled();

    expect(closeHandler!()).toBe(true);
    await tick();
    await fireEvent.click(screen.getByRole('button', { name: 'Stop and quit' }));
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    await tick();
    expect(
      screen.getByRole('progressbar', { name: 'Stopping... the app will close in a moment.' }),
    ).toBeTruthy();
    // The window closes only after the run has ended.
    expect(m.closeWindow).not.toHaveBeenCalled();
    job.finish(report({ cancelled: true }));
    await waitFor(() => expect(m.closeWindow).toHaveBeenCalledTimes(1));
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(unlistenQuit).toHaveBeenCalledTimes(1);
    expect(wizard.screen).toBe('remove');
  });

  it('secondCloseWhileQuittingClosesAtOnce', async () => {
    pendingRemove();
    stepFour();
    await waitFor(() => expect(closeHandler).not.toBeNull());
    expect(closeHandler!()).toBe(true);
    await tick();
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.stopAndQuit }));
    // Still running, but the person asked twice: the window is not held again.
    expect(closeHandler!()).toBe(false);
    await tick();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closeAfterRunEndedIsNotHeld', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(closeHandler).not.toBeNull());
    job.finish(report({ done: [done(1)] }));
    await waitFor(() => expect(wizard.screen).toBe('done'));
    // Even if stopping the guard had failed, it lets the window close.
    expect(closeHandler!()).toBe(false);
    await tick();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closeAfterStopSaysTheAppWillClose', async () => {
    const job = pendingRemove();
    stepFour();
    await waitFor(() => expect(closeHandler).not.toBeNull());
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.stop }));
    await fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: copy.remove.stop }));
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    expect(closeHandler!()).toBe(true);
    await tick();
    const dialog = screen.getByRole('dialog', {
      name: 'Stopping... the app will close in a moment.',
    });
    expect(within(dialog).queryByRole('button', { name: copy.remove.keepGoing })).toBeNull();
    expect(within(dialog).getAllByRole('button').map((b) => b.textContent?.trim())).toEqual([copy.remove.ok]);
    assertPlainText(document.body);
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.remove.ok }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    job.finish(report({ cancelled: true }));
    await waitFor(() => expect(m.closeWindow).toHaveBeenCalledTimes(1));
  });

  it('quitRequestWhileRunningAsksTheSame', async () => {
    const job = pendingRemove();
    stepFour();
    await waitFor(() => expect(quitHandler).not.toBeNull());
    quitHandler!();
    await tick();
    expect(screen.getByRole('dialog', { name: copy.remove.quitTitle })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.stopAndQuit }));
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    expect(m.closeWindow).not.toHaveBeenCalled();
    // Asked again while quitting: quit at once.
    quitHandler!();
    expect(m.closeWindow).toHaveBeenCalledTimes(1);
    job.finish(report({ cancelled: true }));
    await tick();
  });

  it('quitRequestAfterRunEndedQuits', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(quitHandler).not.toBeNull());
    job.finish(report({ done: [done(1)] }));
    await waitFor(() => expect(wizard.screen).toBe('done'));
    quitHandler!();
    expect(m.closeWindow).toHaveBeenCalledTimes(1);
  });

  it('closeGuardEndsWithTheRun', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(closeHandler).not.toBeNull());
    expect(unlisten).not.toHaveBeenCalled();
    job.finish(report({ done: [done(1)] }));
    await waitFor(() => expect(wizard.screen).toBe('done'));
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(unlistenQuit).toHaveBeenCalledTimes(1);
  });

  it('unmountStopsTheRunAndTheCloseGuard', async () => {
    pendingRemove();
    const view = render(Remove, { wizard: createWizard() });
    await waitFor(() => expect(closeHandler).not.toBeNull());
    await waitFor(() => expect(quitHandler).not.toBeNull());
    view.unmount();
    expect(m.cancelJob).toHaveBeenCalledTimes(1);
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(unlistenQuit).toHaveBeenCalledTimes(1);
  });

  it('closeGuardArrivingAfterUnmountIsRemoved', async () => {
    let answer: (off: () => void) => void = () => {};
    m.onCloseRequested.mockImplementation(() => new Promise((r) => (answer = r)));
    pendingRemove();
    const view = render(Remove, { wizard: createWizard() });
    await waitFor(() => expect(m.onCloseRequested).toHaveBeenCalled());
    view.unmount();
    answer(unlisten as unknown as () => void);
    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(1));
  });

  it('refusalShowsItsMessageAndBack', async () => {
    const job = pendingRemove();
    const wizard = stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    job.fail('Open or untick the photos with a ? first.');
    expect(await screen.findByText('Open or untick the photos with a ? first.')).toBeTruthy();
    expect(wizard.busy).toBe(false);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('checkAll');
  });

  it('quitDialogIsPlainText', async () => {
    pendingRemove();
    stepFour();
    await waitFor(() => expect(closeHandler).not.toBeNull());
    closeHandler!();
    await tick();
    assertPlainText(document.body);
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.stopAndQuit }));
    assertPlainText(document.body);
  });

  it('noBannedWordsRendered', async () => {
    const job = pendingRemove();
    stepFour();
    await waitFor(() => expect(m.remove).toHaveBeenCalled());
    job.emit(removing(0, 2, 1, 0.3, 125));
    job.emit({ phase: 'fellBackToProcessor', reason: 'device lost' });
    await fireEvent.click(await screen.findByRole('button', { name: copy.device.showDetails }));
    await fireEvent.click(screen.getByRole('button', { name: copy.remove.stop }));
    assertPlainText(document.body);
  });
});

describe('Done', () => {
  it('countsSavedPhotosAndTheirFolder', async () => {
    stepFive(report({ done: [done(1), done(2), done(3)] }));
    expect(screen.getByText("3 photos saved in 'Without watermark'")).toBeTruthy();
  });

  it('namesTheChosenFolderAndOnePhoto', async () => {
    stepFive(report({ done: [done(4, 'C:\\Users\\me\\Results\\p4.jpg')] }));
    expect(screen.getByText("1 photo saved in 'Results'")).toBeTruthy();
  });

  it('doneListsFailures', async () => {
    stepFive(
      report({
        done: [done(1)],
        failed: [
          { id: 2, name: 'beach.jpg', reason: 'The photo could not be read.' },
          { id: 3, name: 'dune.jpg', reason: 'The photo was moved or deleted.' },
        ],
      }),
    );
    expect(screen.getByText('2 photos could not be finished:')).toBeTruthy();
    expect(screen.getByText('beach.jpg - The photo could not be read.')).toBeTruthy();
    expect(screen.getByText('dune.jpg - The photo was moved or deleted.')).toBeTruthy();
  });

  it('stoppedRunSaysHowManyWereFinished', async () => {
    stepFive(report({ done: [done(1), done(2)], cancelled: true }));
    expect(screen.getByText('Stopped. 2 photos were finished.')).toBeTruthy();
    expect(copy.done.stopped(1)).toBe('Stopped. 1 photo was finished.');
  });

  it('nothingSavedHidesOpenFolderAndTheComparison', async () => {
    stepFive(report({ failed: [{ id: 2, name: 'a.jpg', reason: 'Broken.' }] }));
    expect(screen.getByText(copy.done.nothingSaved)).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.done.openFolder })).toBeNull();
    expect(screen.queryByRole('slider')).toBeNull();
  });

  it('openFolderRevealsTheFirstResult', async () => {
    stepFive(report({ done: [done(1, '/a/Without watermark/p1.jpg'), done(2, '/b/Without watermark/p2.jpg')] }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open folder' }));
    expect(m.revealItemInDir).toHaveBeenCalledWith('/a/Without watermark/p1.jpg');
  });

  it('browsesBeforeAndAfterWithButtonsAndKeys', async () => {
    stepFive(report({ done: [done(1), done(2), done(3)] }));
    const after = () => screen.getByAltText(copy.beforeAfter.after).getAttribute('src');
    const before = () => screen.getByAltText(copy.beforeAfter.before).getAttribute('src');
    expect(before()).toBe('puw://original/1');
    expect(after()).toMatch(/^puw:\/\/result\/1\?v=\d+$/);
    expect(screen.getByText('Photo 1 of 3')).toBeTruthy();
    expect(screen.getByText('p1.jpg')).toBeTruthy();
    const previous = screen.getByRole('button', { name: copy.done.previous }) as HTMLButtonElement;
    expect(previous.disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: copy.done.next }));
    expect(before()).toBe('puw://original/2');
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    expect(before()).toBe('puw://original/3');
    expect((screen.getByRole('button', { name: copy.done.next }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    expect(before()).toBe('puw://original/3');
    await fireEvent.keyDown(window, { key: 'ArrowLeft' });
    expect(before()).toBe('puw://original/2');
    // The handle keeps its own arrows, and typing a name does not browse.
    await fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowLeft' });
    expect(before()).toBe('puw://original/2');
    await fireEvent.keyDown(await screen.findByRole('textbox'), { key: 'ArrowLeft' });
    expect(before()).toBe('puw://original/2');
  });

  it('announcesBrowsing', async () => {
    const { container } = render(Done, { wizard: wizardWith(report({ done: [done(1), done(2), done(3)] })) });
    const live = container.querySelector('[data-testid="announce"]')!;
    expect(live.getAttribute('aria-live')).toBe('polite');
    expect(live.textContent).toBe('');
    await fireEvent.click(screen.getByRole('button', { name: copy.done.next }));
    expect(live.textContent).toBe('Photo 2 of 3');
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    expect(live.textContent).toBe('Photo 3 of 3');
  });

  it('keepsSavedAndPositionWhenComingBack', async () => {
    const wizard = wizardWith(report({ done: [done(1), done(2), done(3)] }));
    const first = render(Done, { wizard });
    await screen.findByDisplayValue('Watermark from 2026-11-25');
    await fireEvent.click(screen.getByRole('button', { name: copy.done.next }));
    await fireEvent.click(screen.getByRole('button', { name: copy.done.saveWatermark }));
    expect(await screen.findByText(copy.done.savedWatermark)).toBeTruthy();
    first.unmount();
    // Done -> Settings -> Back shows the step again.
    render(Done, { wizard });
    expect(screen.getByText(copy.done.savedWatermark)).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.done.saveWatermark })).toBeNull();
    expect(screen.getByText('Photo 2 of 3')).toBeTruthy();
    expect(screen.getByAltText(copy.beforeAfter.before).getAttribute('src')).toBe('puw://original/2');
    expect(m.saveWatermark).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole('button', { name: copy.done.startNewSet }));
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(wizard.savedGroups).toEqual([]);
    expect(wizard.resultIndex).toBe(0);
  });

  it('saveWatermarkUsesDefaultName', async () => {
    stepFive(report({ done: [done(1)] }));
    const field = (await screen.findByDisplayValue('Watermark from 2026-11-25')) as HTMLInputElement;
    expect(screen.getByLabelText(copy.done.name)).toBe(field);
    await fireEvent.click(screen.getByRole('button', { name: 'Save this watermark for next time' }));
    expect(m.saveWatermark).toHaveBeenCalledWith(1, 'Watermark from 2026-11-25');
    expect(await screen.findByText('Saved')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Save this watermark for next time' })).toBeNull();
  });

  it('saveWatermarkSavesTheCurrentGroup', async () => {
    const wizard = wizardWith(report({ done: [done(1)] }));
    wizard.useGroup(2);
    render(Done, { wizard });
    await screen.findByDisplayValue('Watermark from 2026-11-25');
    await fireEvent.click(screen.getByRole('button', { name: copy.done.saveWatermark }));
    expect(await screen.findByText(copy.done.savedWatermark)).toBeTruthy();
    expect(m.saveWatermark).toHaveBeenCalledWith(2, 'Watermark from 2026-11-25');
    expect(wizard.savedGroups).toEqual([2]);
  });

  it('saveWatermarkTakesTheTypedNameAndNeedsOne', async () => {
    stepFive(report({ done: [done(1)] }));
    const field = (await screen.findByDisplayValue('Watermark from 2026-11-25')) as HTMLInputElement;
    await fireEvent.input(field, { target: { value: '   ' } });
    await fireEvent.click(screen.getByRole('button', { name: copy.done.saveWatermark }));
    expect(m.saveWatermark).not.toHaveBeenCalled();
    expect(screen.getByText(copy.saved.emptyName)).toBeTruthy();
    await fireEvent.input(field, { target: { value: ' Studio ' } });
    m.saveWatermark.mockRejectedValueOnce({ message: '', details: '' });
    await fireEvent.click(screen.getByRole('button', { name: copy.done.saveWatermark }));
    expect(await screen.findByText(copy.done.saveFailed)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.done.saveWatermark }));
    expect(m.saveWatermark).toHaveBeenLastCalledWith(1, 'Studio');
    expect(await screen.findByText(copy.done.savedWatermark)).toBeTruthy();
  });

  it('newSetClearsAndGoesToAdd', async () => {
    const wizard = stepFive(report({ done: [done(1)] }));
    wizard.startDraft({ sample: [1], representative: 1 }, [1]);
    wizard.confirmDraft();
    await fireEvent.click(screen.getByRole('button', { name: 'Start a new set' }));
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(m.clearPhotos).toHaveBeenCalledTimes(1);
    expect(wizard.report).toBeNull();
    expect(wizard.drawing).toBeNull();
    expect(wizard.groups).toBe(0);
    expect(wizard.canGoBack).toBe(false);
  });

  it('newSetFailureStays', async () => {
    const wizard = stepFive(report({ done: [done(1)] }));
    m.clearPhotos.mockRejectedValueOnce({ message: '', details: '' });
    await fireEvent.click(screen.getByRole('button', { name: copy.done.startNewSet }));
    expect(await screen.findByText(copy.done.newSetFailed)).toBeTruthy();
    expect(wizard.screen).toBe('done');
  });

  it('noBannedWordsRendered', async () => {
    stepFive(
      report({
        done: [done(1), done(2)],
        failed: [{ id: 3, name: 'x.jpg', reason: 'The photo could not be read.' }],
        cancelled: true,
      }),
    );
    await screen.findByDisplayValue('Watermark from 2026-11-25');
    assertPlainText(document.body);
  });
});
