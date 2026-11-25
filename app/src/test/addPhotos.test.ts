import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DetectOutcome, PhotoInfo, ProgressEvent } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  downloadModels: vi.fn(),
  cancelJob: vi.fn(),
  addPaths: vi.fn(),
  removePhoto: vi.fn(),
  listPhotos: vi.fn(),
  suggestSaved: vi.fn(),
  detect: vi.fn(),
  useSaved: vi.fn(),
  imageUrl: vi.fn((route: string, id: number) => `puw://${route}/${id}`),
  onDragDrop: vi.fn(),
  listSaved: vi.fn(),
  choosePhotos: vi.fn(),
  chooseFolder: vi.fn(),
}));

import App from '../App.svelte';
import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

const addPaths = vi.mocked(api.addPaths);
const listPhotos = vi.mocked(api.listPhotos);
const suggestSaved = vi.mocked(api.suggestSaved);
const detect = vi.mocked(api.detect);
const useSaved = vi.mocked(api.useSaved);
const cancelJob = vi.mocked(api.cancelJob);
const onDragDrop = vi.mocked(api.onDragDrop);
const choosePhotos = vi.mocked(api.choosePhotos);
const chooseFolder = vi.mocked(api.chooseFolder);
const removePhoto = vi.mocked(api.removePhoto);

const photo = (id: number, name = `p${id}.jpg`): PhotoInfo => ({ id, name, folder: '/x', width: 10, height: 10 });
const outcome: DetectOutcome = { sample: [1], representative: 1 };

/** A job that stays open until the test settles it. */
function openJob() {
  let emit: (e: ProgressEvent) => void = () => {};
  let finish: (o: DetectOutcome) => void = () => {};
  let fail: (e: unknown) => void = () => {};
  const impl = (...args: unknown[]) => {
    emit = args[args.length - 1] as (e: ProgressEvent) => void;
    return new Promise<DetectOutcome>((resolve, reject) => {
      finish = resolve;
      fail = reject;
    });
  };
  return { impl, emit: (e: ProgressEvent) => emit(e), finish: (o = outcome) => finish(o), fail: (e: unknown) => fail(e) };
}

async function start(photos: PhotoInfo[] = []) {
  listPhotos.mockResolvedValue(photos);
  const wizard = createWizard();
  wizard.go('add');
  const view = render(App, { wizard });
  await screen.findByText(copy.addPhotos.drop);
  return { wizard, ...view };
}

describe('Add photos', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.imageUrl).mockImplementation((route, id) => `puw://${route}/${id}`);
    onDragDrop.mockResolvedValue(() => {});
    suggestSaved.mockResolvedValue(null);
  });

  it('savedWatermarksEntry', async () => {
    const { wizard } = await start();
    vi.mocked(api.listSaved).mockResolvedValueOnce([]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.savedWatermarks }));
    expect(wizard.screen).toBe('saved');
    expect(await screen.findByText(copy.saved.empty)).toBeTruthy();
  });

  it('primaryDisabledWithoutPhotos', async () => {
    await start();
    const primary = screen.getByRole('button', { name: copy.addPhotos.find }) as HTMLButtonElement;
    expect(primary.disabled).toBe(true);
  });

  it('showsThumbnailsAndCount', async () => {
    await start([photo(1), photo(2)]);
    const img = (await screen.findByAltText('p1.jpg')) as HTMLImageElement;
    expect(img.getAttribute('src')).toBe('puw://thumb/1');
    expect(img.getAttribute('loading')).toBe('lazy');
    expect(screen.getByText('2 photos')).toBeTruthy();
    expect((screen.getByRole('button', { name: copy.addPhotos.find }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('chooseButtonsAddPaths', async () => {
    await start();
    choosePhotos.mockResolvedValueOnce(['/a/1.jpg']);
    addPaths.mockResolvedValueOnce({ added: [photo(1)], skipped: [] });
    listPhotos.mockResolvedValue([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await screen.findByAltText('p1.jpg');
    expect(addPaths).toHaveBeenCalledWith(['/a/1.jpg']);

    chooseFolder.mockResolvedValueOnce(null);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.chooseFolder }));
    await waitFor(() => expect(chooseFolder).toHaveBeenCalled());
    expect(addPaths).toHaveBeenCalledTimes(1);
  });

  it('dropAddsPaths', async () => {
    let handler: (e: { kind: string; paths: string[] }) => void = () => {};
    onDragDrop.mockImplementation(async (cb) => {
      handler = cb as typeof handler;
      return () => {};
    });
    await start();
    await waitFor(() => expect(onDragDrop).toHaveBeenCalled());
    addPaths.mockResolvedValueOnce({ added: [photo(3)], skipped: [] });
    listPhotos.mockResolvedValue([photo(3)]);
    handler({ kind: 'drop', paths: ['/a/folder'] });
    await screen.findByAltText('p3.jpg');
    expect(addPaths).toHaveBeenCalledWith(['/a/folder']);
  });

  it('showsSkippedReasons', async () => {
    const { container } = await start();
    choosePhotos.mockResolvedValueOnce(['/a/1.jpg', '/a/2.heic']);
    addPaths.mockResolvedValueOnce({
      added: [photo(1)],
      skipped: [{ name: '2.heic', reason: 'This kind of photo is not supported yet.' }],
    });
    listPhotos.mockResolvedValue([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    expect(await screen.findByText(/2\.heic/)).toBeTruthy();
    expect(screen.getByText(/This kind of photo is not supported yet\./)).toBeTruthy();
    assertPlainText(container);
  });

  it('removeTakesPhotoOut', async () => {
    await start([photo(1), photo(2)]);
    removePhoto.mockResolvedValueOnce();
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.remove('p1.jpg') }));
    await waitFor(() => expect(screen.queryByAltText('p1.jpg')).toBeNull());
    expect(removePhoto).toHaveBeenCalledWith(1);
    expect(screen.getByText('1 photo')).toBeTruthy();
  });

  it('offerSavedWatermark', async () => {
    await start();
    suggestSaved.mockResolvedValueOnce({ id: 's1', name: 'Studio' });
    choosePhotos.mockResolvedValueOnce(['/a/1.jpg']);
    addPaths.mockResolvedValueOnce({ added: [photo(1)], skipped: [] });
    listPhotos.mockResolvedValue([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    expect(await screen.findByText('These look like Studio. Use it?')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.no }));
    expect(screen.queryByText('These look like Studio. Use it?')).toBeNull();
  });

  async function offered() {
    const view = await start([photo(1)]);
    suggestSaved.mockResolvedValueOnce({ id: 's1', name: 'Studio' });
    choosePhotos.mockResolvedValueOnce(['/a/2.jpg']);
    addPaths.mockResolvedValueOnce({ added: [photo(2)], skipped: [] });
    listPhotos.mockResolvedValue([photo(1), photo(2)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await screen.findByText('These look like Studio. Use it?');
    return view;
  }

  it('yesUsesSaved', async () => {
    const job = openJob();
    useSaved.mockImplementation(job.impl);
    const { wizard } = await offered();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.yes }));
    expect(useSaved).toHaveBeenCalledWith('s1', expect.any(Function));
    expect(detect).not.toHaveBeenCalled();
    expect(wizard.busy).toBe(true);
    job.finish();
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(wizard.outcome).toEqual(outcome);
    expect(wizard.busy).toBe(false);
  });

  it('removeRefreshesOffer', async () => {
    await offered();
    let answer: (v: { id: string; name: string } | null) => void = () => {};
    suggestSaved.mockImplementationOnce(() => new Promise((r) => (answer = r)));
    removePhoto.mockResolvedValueOnce();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.remove('p2.jpg') }));
    await waitFor(() => expect(suggestSaved).toHaveBeenCalledTimes(2));
    expect(screen.queryByText('These look like Studio. Use it?')).toBeNull();
    answer({ id: 's1', name: 'Studio' });
    expect(await screen.findByText('These look like Studio. Use it?')).toBeTruthy();
  });

  it('removingLastPhotoClearsOffer', async () => {
    await start([photo(1)]);
    suggestSaved.mockResolvedValueOnce({ id: 's1', name: 'Studio' });
    choosePhotos.mockResolvedValueOnce(['/a/1.jpg']);
    addPaths.mockResolvedValueOnce({ added: [photo(1)], skipped: [] });
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await screen.findByText('These look like Studio. Use it?');
    removePhoto.mockResolvedValueOnce();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.remove('p1.jpg') }));
    await waitFor(() => expect(screen.queryByText('1 photo')).toBeNull());
    expect(screen.queryByText('These look like Studio. Use it?')).toBeNull();
    expect(suggestSaved).toHaveBeenCalledTimes(1);
  });

  it('failedRemoveSaysSo', async () => {
    await start([photo(1)]);
    removePhoto.mockRejectedValueOnce({ message: '', details: '' });
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.remove('p1.jpg') }));
    expect(await screen.findByText(copy.addPhotos.removeFailed)).toBeTruthy();
  });

  it('savedPathStartsNeutral', async () => {
    useSaved.mockImplementation(openJob().impl);
    await offered();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.yes }));
    expect(await screen.findByText(copy.finding.neutral)).toBeTruthy();
    expect(screen.queryByText(copy.loading.finder)).toBeNull();
  });

  it('findingSentencesFollowEvents', async () => {
    const job = openJob();
    detect.mockImplementation(job.impl);
    const { wizard, container } = await start([photo(1), photo(2), photo(3)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await screen.findByText(copy.loading.finder);

    job.emit({ phase: 'loadingModels', fraction: 0.5, which: 'finder' });
    expect((await screen.findByRole('progressbar')).getAttribute('aria-valuenow')).toBe('50');
    expect(screen.getByText(copy.loading.finder)).toBeTruthy();
    assertPlainText(container);

    job.emit({ phase: 'reading', fraction: 0.25 });
    await screen.findByText('Reading your photos...');
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('25');

    job.emit({ phase: 'looking', done: 1, total: 3 });
    await screen.findByText('Looking for the watermark... photo 2 of 3');
    assertPlainText(container);

    job.finish();
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(wizard.outcome).toEqual(outcome);
  });

  it('backCancels', async () => {
    const job = openJob();
    detect.mockImplementation(job.impl);
    cancelJob.mockImplementationOnce(async () => job.fail({ message: 'Stopped.', details: '' }));
    const { wizard } = await start([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await fireEvent.click(await screen.findByRole('button', { name: copy.common.back }));
    expect(cancelJob).toHaveBeenCalledTimes(1);
    await screen.findByText(copy.addPhotos.drop);
    expect(screen.queryByText('Stopped.')).toBeNull();
    expect(wizard.screen).toBe('add');
    expect(wizard.busy).toBe(false);
  });

  it('failureShowsMessageAndBack', async () => {
    detect.mockRejectedValueOnce({ message: 'Something went wrong.', details: 'raw' });
    await start([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    expect(await screen.findByText('Something went wrong.')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    await screen.findByText(copy.addPhotos.drop);
  });

  /** Adds photo 2 to photo 1 with the saved-watermark check left running. */
  async function checkRunning() {
    const view = await start([photo(1)]);
    let answer: (v: { id: string; name: string } | null) => void = () => {};
    suggestSaved.mockImplementationOnce(() => new Promise((r) => (answer = r)));
    choosePhotos.mockResolvedValueOnce(['/a/2.jpg']);
    addPaths.mockResolvedValueOnce({ added: [photo(2)], skipped: [] });
    listPhotos.mockResolvedValue([photo(1), photo(2)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await screen.findByText(copy.addPhotos.checkingSaved);
    return { ...view, answer: (v: { id: string; name: string } | null) => answer(v) };
  }

  it('savedCheckDoesNotBlockTheButtons', async () => {
    const { container, answer } = await checkRunning();
    for (const name of [copy.addPhotos.choosePhotos, copy.addPhotos.chooseFolder, copy.addPhotos.find]) {
      expect((screen.getByRole('button', { name }) as HTMLButtonElement).disabled, name).toBe(false);
    }
    assertPlainText(container);
    answer(null);
    await waitFor(() => expect(screen.queryByText(copy.addPhotos.checkingSaved)).toBeNull());
    expect(cancelJob).not.toHaveBeenCalled();
  });

  it('findStopsTheSavedCheck', async () => {
    const job = openJob();
    detect.mockImplementation(job.impl);
    cancelJob.mockResolvedValue();
    const { answer } = await checkRunning();
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledTimes(1));
    expect(detect).toHaveBeenCalledTimes(1);
    // A late answer of the stopped check offers nothing.
    answer({ id: 's1', name: 'Studio' });
    await screen.findByText(copy.loading.finder);
    expect(screen.queryByText('These look like Studio. Use it?')).toBeNull();
  });

  it('addingStopsTheSavedCheck', async () => {
    cancelJob.mockResolvedValue();
    await checkRunning();
    chooseFolder.mockResolvedValueOnce('/b');
    addPaths.mockResolvedValueOnce({ added: [], skipped: [] });
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.chooseFolder }));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByText(copy.addPhotos.checkingSaved)).toBeNull());
  });

  it('removeWaitsForAdding', async () => {
    await start([photo(1), photo(2)]);
    let added: (v: { added: PhotoInfo[]; skipped: [] }) => void = () => {};
    addPaths.mockImplementationOnce(() => new Promise((r) => (added = r)));
    choosePhotos.mockResolvedValueOnce(['/a/3.jpg']);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.choosePhotos }));
    await waitFor(() => expect(addPaths).toHaveBeenCalled());
    const x = screen.getByRole('button', { name: copy.addPhotos.remove('p1.jpg') }) as HTMLButtonElement;
    expect(x.disabled).toBe(true);
    await fireEvent.click(x);
    expect(removePhoto).not.toHaveBeenCalled();
    added({ added: [], skipped: [] });
    await waitFor(() => expect(x.disabled).toBe(false));
  });

  it('sameListGoesBackToStepTwo', async () => {
    const job = openJob();
    detect.mockImplementation(job.impl);
    const { wizard } = await start([photo(1), photo(2)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    job.finish();
    await waitFor(() => expect(wizard.screen).toBe('check'));
    wizard.back();
    await screen.findByText(copy.addPhotos.drop);
    await screen.findByAltText('p2.jpg');
    // The same photos: straight back to step 2 with what was found, no new search.
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    expect(wizard.screen).toBe('check');
    expect(detect).toHaveBeenCalledTimes(1);
    wizard.back();
    await screen.findByText(copy.addPhotos.drop);
    // A changed list looks again.
    removePhoto.mockResolvedValueOnce();
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.remove('p2.jpg') }));
    await waitFor(() => expect(screen.queryByAltText('p2.jpg')).toBeNull());
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    expect(detect).toHaveBeenCalledTimes(2);
  });

  it('unreadablePhotoSaysWhy', async () => {
    const { container } = await start([photo(1), photo(2)]);
    const img = (await screen.findByAltText('p2.jpg')) as HTMLImageElement;
    listPhotos.mockResolvedValue([photo(1), { ...photo(2), problem: 'damaged' }]);
    await fireEvent.error(img);
    expect(await screen.findByText(copy.photoProblem.damaged)).toBeTruthy();
    expect(screen.queryByAltText('p2.jpg')).toBeNull();
    // It can still be taken out of the list.
    expect(screen.getByRole('button', { name: copy.addPhotos.remove('p2.jpg') })).toBeTruthy();
    assertPlainText(container);
  });

  it('missingDataOffersTheDownload', async () => {
    detect.mockRejectedValueOnce({ message: 'Some files are missing.', details: 'raw', missingData: true });
    const { wizard } = await start([photo(1)]);
    await fireEvent.click(screen.getByRole('button', { name: copy.addPhotos.find }));
    await fireEvent.click(await screen.findByRole('button', { name: copy.common.downloadAgain }));
    expect(wizard.screen).toBe('welcome');
    expect(wizard.busy).toBe(false);
  });

  it('noBannedWordsRendered', async () => {
    const { container } = await start([photo(1)]);
    assertPlainText(container);
    for (const text of Object.values(copy.addPhotos)) {
      if (typeof text === 'string') assertPlainText(Object.assign(document.createElement('p'), { textContent: text }));
    }
  });
});
