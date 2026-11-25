import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DownloadProgress, UserError } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  downloadModels: vi.fn(),
  cancelJob: vi.fn(),
  listPhotos: vi.fn(async () => []),
  suggestSaved: vi.fn(async () => null),
  onDragDrop: vi.fn(async () => () => {}),
  imageUrl: vi.fn(),
}));

import App from '../App.svelte';
import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

const downloadModels = vi.mocked(api.downloadModels);
const cancelJob = vi.mocked(api.cancelJob);
const appStatus = vi.mocked(api.appStatus);

const progress = (doneBytes: number): DownloadProgress => ({ doneBytes, totalBytes: 690_000_000, file: 'a.onnx' });
const userError: UserError = { message: 'The connection was lost.', details: 'raw text' };

/** A download that stays open until the test settles it. */
function openDownload() {
  let emit: (p: DownloadProgress) => void = () => {};
  let finish: () => void = () => {};
  let fail: (e: unknown) => void = () => {};
  downloadModels.mockImplementationOnce((onEvent) => {
    emit = onEvent;
    return new Promise<void>((resolve, reject) => {
      finish = resolve;
      fail = reject;
    });
  });
  return { emit: (p: DownloadProgress) => emit(p), finish: () => finish(), fail: (e: unknown) => fail(e) };
}

async function start() {
  appStatus.mockResolvedValue({ modelsReady: false, missingBytes: 690_000_000, platform: 'macos', version: '1' });
  const wizard = createWizard();
  const view = render(App, { wizard });
  await screen.findByRole('button', { name: copy.welcome.download });
  return { wizard, ...view };
}

describe('Welcome', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.listPhotos).mockResolvedValue([]);
    vi.mocked(api.onDragDrop).mockResolvedValue(() => {});
  });

  it('showsSizeAndPrivacy', async () => {
    await start();
    expect(screen.getByText(copy.welcome.needsDownload)).toBeTruthy();
    expect(copy.welcome.needsDownload).toContain('about 690 MB');
    expect(copy.welcome.needsDownload).toContain('Your photos never leave this computer.');
  });

  it('goesStraightToAddPhotosWhenModelsAreReady', async () => {
    appStatus.mockResolvedValue({ modelsReady: true, missingBytes: 0, platform: 'macos', version: '1' });
    const wizard = createWizard();
    render(App, { wizard });
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(screen.queryByRole('button', { name: copy.welcome.download })).toBeNull();
  });

  it('progressShowsMegabytes', async () => {
    const dl = openDownload();
    const { container } = await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    dl.emit(progress(123_456_789));
    expect(await screen.findByText('123 MB of 690 MB')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('18');
    expect(screen.queryByRole('button', { name: copy.welcome.download })).toBeNull();
    assertPlainText(container);
  });

  it('progressShowsTimeLeftOnceTheSpeedIsKnown', async () => {
    const now = vi.spyOn(Date, 'now');
    now.mockReturnValue(1_000_000);
    const dl = openDownload();
    await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    // A resumed download starts from what is there already.
    dl.emit(progress(440_000_000));
    expect(await screen.findByText('440 MB of 690 MB')).toBeTruthy();
    // 10 MB in 2 s: the remaining 240 MB take about 48 s.
    now.mockReturnValue(1_002_000);
    dl.emit(progress(450_000_000));
    expect(await screen.findByText(`450 MB of 690 MB - ${copy.remove.timeLeft(48)}`)).toBeTruthy();
  });

  it('pauseStopsAndResumeContinues', async () => {
    const first = openDownload();
    await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    first.emit(progress(200_000_000));
    await screen.findByText('200 MB of 690 MB');
    cancelJob.mockImplementationOnce(async () => first.fail({ message: 'Stopped.', details: '' }));
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.pause }));
    expect(cancelJob).toHaveBeenCalledTimes(1);
    const again = await screen.findByRole('button', { name: copy.welcome.resume });
    expect(screen.queryByText('Stopped.')).toBeNull();
    expect(screen.queryByRole('button', { name: copy.welcome.tryAgain })).toBeNull();
    openDownload();
    await fireEvent.click(again);
    expect(downloadModels).toHaveBeenCalledTimes(2);
  });

  it('pausedShowsProgressAndResume', async () => {
    const first = openDownload();
    const { container } = await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    first.emit(progress(120_000_000));
    await screen.findByText('120 MB of 690 MB');
    cancelJob.mockImplementationOnce(async () => first.fail({ message: 'Stopped.', details: '' }));
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.pause }));
    expect(await screen.findByText('Paused at 120 MB of 690 MB.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.welcome.download })).toBeNull();
    assertPlainText(container);
    openDownload();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.resume }));
    expect(downloadModels).toHaveBeenCalledTimes(2);
  });

  it('doubleClickStartsOneDownload', async () => {
    openDownload();
    await start();
    const button = screen.getByRole('button', { name: copy.welcome.download });
    button.click();
    button.click();
    await screen.findByRole('button', { name: copy.welcome.pause });
    expect(downloadModels).toHaveBeenCalledTimes(1);
  });

  it('errorOffersTryAgain', async () => {
    downloadModels.mockRejectedValueOnce(userError);
    const { container } = await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    expect(await screen.findByText(userError.message)).toBeTruthy();
    assertPlainText(container);
    const dl = openDownload();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.tryAgain }));
    expect(downloadModels).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(userError.message)).toBeNull();
    dl.finish();
  });

  it('successGoesToAddPhotos', async () => {
    const dl = openDownload();
    const { wizard } = await start();
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    dl.emit(progress(690_000_000));
    dl.finish();
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(wizard.busy).toBe(false);
  });

  it('noBannedWordsRendered', async () => {
    const dl = openDownload();
    const { container } = await start();
    assertPlainText(container);
    await fireEvent.click(screen.getByRole('button', { name: copy.welcome.download }));
    dl.emit(progress(300_000_000));
    await screen.findByText('300 MB of 690 MB');
    assertPlainText(container);
    dl.fail(userError);
    await screen.findByText(userError.message);
    assertPlainText(container);
  });
});
