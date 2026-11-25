import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { PhotoInfo, PhotoState, ProgressEvent } from '../lib/types';

vi.mock('../lib/api', () => ({
  cancelJob: vi.fn(),
  matchAll: vi.fn(),
  photoStates: vi.fn(),
  listPhotos: vi.fn(),
  setIncluded: vi.fn(),
  removalBlockers: vi.fn(),
  imageUrl: vi.fn((route: string, id: number, v?: number) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`),
}));
// The single-photo editor has its own tests; here it only reports back.
vi.mock('../screens/CheckWatermark.svelte', async () => ({
  default: (await import('./CheckWatermarkStub.svelte')).default,
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import CheckAll from '../screens/CheckAll.svelte';
import { createWizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

const matchAll = vi.mocked(api.matchAll);
const photoStates = vi.mocked(api.photoStates);
const listPhotos = vi.mocked(api.listPhotos);
const setIncluded = vi.mocked(api.setIncluded);
const removalBlockers = vi.mocked(api.removalBlockers);
const cancelJob = vi.mocked(api.cancelJob);

const info = (id: number): PhotoInfo => ({ id, name: `p${id}.jpg`, folder: '/x', width: 10, height: 10 });
const st = (id: number, status: PhotoState['status'], extra: Partial<PhotoState> = {}): PhotoState => ({
  id,
  status,
  included: status === 'found' || status === 'unsure' || status === 'edited',
  reviewed: false,
  hasArea: status !== 'notFound',
  ...extra,
});

function setup(states: PhotoState[]) {
  listPhotos.mockResolvedValue(states.map((s) => info(s.id)));
  photoStates.mockResolvedValue(states);
  removalBlockers.mockResolvedValue(states.filter((s) => s.status === 'unsure' && s.included && !s.reviewed).map((s) => s.id));
  matchAll.mockResolvedValue({ found: 0, unsure: 0, notFound: 0, total: states.length });
  const wizard = createWizard();
  wizard.go('add');
  wizard.go('check');
  wizard.go('checkAll');
  render(CheckAll, { wizard });
  return wizard;
}

const primary = () => screen.findByRole('button', { name: /^Remove watermark from/ }) as Promise<HTMLButtonElement>;

describe('Check all photos', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    cancelJob.mockResolvedValue();
    vi.mocked(api.imageUrl).mockImplementation(
      (route, id, v) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`,
    );
  });

  it('summaryLine', async () => {
    const states = [...Array.from({ length: 118 }, (_, i) => st(i, 'found')), st(118, 'unsure'), st(119, 'unsure')];
    setup(states);
    expect(await screen.findByText('Found in 118 of 120 photos. 2 need a look.')).toBeTruthy();
  });

  it('foundCountsEditedOnlyWithAnArea', async () => {
    // The app's rule: an own area that is empty is "not found".
    setup([st(1, 'found'), st(2, 'edited'), st(3, 'edited', { hasArea: false, included: false })]);
    expect(await screen.findByText('Found in 2 of 3 photos.')).toBeTruthy();
  });

  it('unreadablePhotoSaysWhyAndCannotBeUsed', async () => {
    setup([st(1, 'found'), st(2, 'unreadable', { hasArea: false, included: false }), st(3, 'unreadable', { hasArea: false, included: false })]);
    listPhotos.mockResolvedValue([info(1), { ...info(2), problem: 'gone' }, { ...info(3), problem: 'damaged' }]);
    await primary();
    expect(screen.getByText(copy.photoProblem.gone)).toBeTruthy();
    expect(screen.getByText(copy.photoProblem.damaged)).toBeTruthy();
    for (const id of [2, 3]) {
      const box = screen.getByRole('checkbox', { name: `Include p${id}.jpg` }) as HTMLInputElement;
      expect(box.disabled).toBe(true);
      // Not offered for opening, and never "Not found".
      expect(screen.queryByRole('button', { name: new RegExp(`^Open p${id}\\.jpg`) })).toBeNull();
    }
    expect(screen.queryByText(copy.checkAll.badgeNotFound)).toBeNull();
    expect(screen.queryByText(copy.checkAll.noAreaTip)).toBeNull();
    expect(screen.getByText('Found in 1 of 3 photos.')).toBeTruthy();
  });

  it('summaryOmitsNeedLookAtZero', async () => {
    setup([st(1, 'found'), st(2, 'notFound')]);
    expect(await screen.findByText('Found in 1 of 2 photos.')).toBeTruthy();
  });

  it('buttonCountsIncluded', async () => {
    setup([st(1, 'found'), st(2, 'found'), st(3, 'notFound')]);
    expect((await primary()).textContent?.trim()).toBe('Remove watermark from 2 photos');
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Include p1.jpg' }));
    expect(setIncluded).toHaveBeenCalledWith(1, false);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Remove watermark from 1 photo' })).toBeTruthy());
  });

  it('routesToRemove', async () => {
    const wizard = setup([st(1, 'found')]);
    await fireEvent.click(await primary());
    expect(wizard.screen).toBe('remove');
  });

  it('disabledUntilUnsureHandled', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const button = await primary();
    expect(button.disabled).toBe(true);
    expect(screen.getByText(copy.checkAll.needLookHint)).toBeTruthy();
    removalBlockers.mockResolvedValue([]);
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Include p2.jpg' }));
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(screen.queryByText(copy.checkAll.needLookHint)).toBeNull();
    expect(button.textContent?.trim()).toBe('Remove watermark from 1 photo');
  });

  it('notFoundStartsUnticked', async () => {
    setup([st(1, 'found'), st(2, 'notFound')]);
    await primary();
    expect((screen.getByRole('checkbox', { name: 'Include p1.jpg' }) as HTMLInputElement).checked).toBe(true);
    const box = screen.getByRole('checkbox', { name: 'Include p2.jpg' }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    // No area: it cannot be ticked, and the reason is given.
    expect(box.disabled).toBe(true);
    expect(box.getAttribute('title')).toBe(copy.checkAll.noAreaTip);
    expect(screen.getByText(copy.checkAll.noAreaTip)).toBeTruthy();
    expect(screen.getByText(copy.checkAll.badgeNotFound)).toBeTruthy();
  });

  it('badges', async () => {
    setup([st(1, 'found'), st(2, 'unsure'), st(3, 'edited')]);
    await primary();
    expect(screen.getByText(copy.checkAll.badgeUnsure)).toBeTruthy();
    expect(screen.getByText(copy.checkAll.badgeEdited)).toBeTruthy();
    // The status is part of what a screen reader says for the tile.
    expect(screen.getByRole('button', { name: 'Open p1.jpg' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Open p2.jpg, not sure the watermark was found' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Open p3.jpg, checked' })).toBeTruthy();
  });

  it('failedTickIsRolledBack', async () => {
    setup([st(1, 'found')]);
    await primary();
    setIncluded.mockRejectedValue({ message: 'nope', details: '' });
    const box = screen.getByRole('checkbox', { name: 'Include p1.jpg' }) as HTMLInputElement;
    await fireEvent.click(box);
    expect(await screen.findByText('nope')).toBeTruthy();
    expect(box.checked).toBe(true);
  });

  it('openingPhotoMarksReviewed', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const button = await primary();
    expect(button.disabled).toBe(true);
    const tile = screen.getByRole('button', { name: copy.checkAll.openPhoto('p2.jpg', 'unsure') });
    expect(screen.getByAltText('p2.jpg').getAttribute('src')).toMatch(/^puw:\/\/grid\/2\?v=\d+$/);
    const before = screen.getByAltText('p2.jpg').getAttribute('src');
    await fireEvent.click(tile);
    // The editor opens the photo (which marks it reviewed), then reports back with a save.
    expect(await screen.findByTestId('stub')).toBeTruthy();
    expect(screen.getByTestId('stub').getAttribute('data-photo')).toBe('2');
    photoStates.mockResolvedValue([st(1, 'found'), st(2, 'edited', { reviewed: true })]);
    removalBlockers.mockResolvedValue([]);
    await fireEvent.click(screen.getByRole('button', { name: 'save' }));
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(screen.queryByTestId('stub')).toBeNull();
    expect(screen.getByText(copy.checkAll.badgeEdited)).toBeTruthy();
    expect(screen.getByAltText('p2.jpg').getAttribute('src')).not.toBe(before);
  });

  it('tilesAreLazy', async () => {
    setup([st(1, 'found')]);
    await primary();
    expect(screen.getByAltText('p1.jpg').getAttribute('loading')).toBe('lazy');
  });

  it('matchesFirstWithProgressAndBackCancels', async () => {
    let emit: (e: ProgressEvent) => void = () => {};
    let finish: () => void = () => {};
    listPhotos.mockResolvedValue([info(1), info(2)]);
    photoStates.mockResolvedValue([st(1, 'pending', { included: false }), st(2, 'pending', { included: false })]);
    removalBlockers.mockResolvedValue([1, 2]);
    matchAll.mockImplementation((cb) => {
      emit = cb;
      return new Promise((resolve, reject) => {
        finish = () => reject({ message: 'cancelled', details: '' });
        void resolve;
      });
    });
    const wizard = createWizard();
    wizard.go('check');
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    expect(await screen.findByText('Finding the watermark in all photos... 0 of 2')).toBeTruthy();
    emit({ phase: 'matching', done: 1, total: 2 });
    expect(await screen.findByText('Finding the watermark in all photos... 1 of 2')).toBeTruthy();
    expect(wizard.busy).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(cancelJob).toHaveBeenCalledTimes(1);
    expect(wizard.screen).toBe('check');
    finish();
    await waitFor(() => expect(wizard.busy).toBe(false));
  });

  it('skipsMatchingWhenEveryPhotoHasAResult', async () => {
    setup([st(1, 'found')]);
    await primary();
    expect(matchAll).not.toHaveBeenCalled();
  });

  it('notFoundStatusInTileName', async () => {
    setup([st(1, 'notFound')]);
    await primary();
    expect(screen.getByRole('button', { name: 'Open p1.jpg, watermark not found' })).toBeTruthy();
  });

  it('hintIsPoliteNotAnError', async () => {
    setup([st(1, 'unsure')]);
    await primary();
    const hint = screen.getByText(copy.checkAll.needLookHint);
    expect(hint.getAttribute('aria-live')).toBe('polite');
    expect(hint.getAttribute('role')).toBeNull();
  });

  it('removeWaitsForBothAnswers', async () => {
    // Nothing blocks Remove, so only the in-flight guard can disable it.
    setup([st(1, 'found'), st(2, 'found')]);
    const button = await primary();
    expect(button.disabled).toBe(false);
    let answerIncluded: () => void = () => {};
    let answerBlockers: (v: number[]) => void = () => {};
    setIncluded.mockImplementationOnce(() => new Promise<void>((r) => (answerIncluded = r)));
    removalBlockers.mockImplementationOnce(() => new Promise((r) => (answerBlockers = r)));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Include p2.jpg' }));
    expect(button.disabled).toBe(true);
    answerIncluded();
    await waitFor(() => expect(removalBlockers).toHaveBeenCalledTimes(2));
    expect(button.disabled).toBe(true);
    answerBlockers([]);
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(button.textContent?.trim()).toBe('Remove watermark from 1 photo');
  });

  it('blockersFailingAfterTickReadsEverythingAgain', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const button = await primary();
    removalBlockers.mockRejectedValueOnce({ message: 'x', details: '' });
    photoStates.mockResolvedValue([st(1, 'found'), st(2, 'unsure', { included: false })]);
    removalBlockers.mockResolvedValue([]);
    const box = screen.getByRole('checkbox', { name: 'Include p2.jpg' }) as HTMLInputElement;
    await fireEvent.click(box);
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(box.checked).toBe(false);
    expect(screen.queryByText(copy.checkAll.tickFailed)).toBeNull();
  });

  it('unmountBeforeStatesDoesNotMatch', async () => {
    let answer: (s: PhotoState[]) => void = () => {};
    photoStates.mockImplementation(() => new Promise((r) => (answer = r)));
    listPhotos.mockResolvedValue([]);
    removalBlockers.mockResolvedValue([]);
    const wizard = createWizard();
    wizard.go('checkAll');
    const view = render(CheckAll, { wizard });
    expect(screen.getByText(copy.checkAll.loading)).toBeTruthy();
    view.unmount();
    answer([st(1, 'pending', { included: false })]);
    await new Promise((r) => setTimeout(r, 0));
    expect(matchAll).not.toHaveBeenCalled();
    expect(wizard.busy).toBe(false);
  });

  it('emptySetShowsMessageAndBack', async () => {
    const wizard = setup([]);
    expect(await screen.findByText(copy.checkAll.empty)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('check');
  });

  it('matchFailureShowsItsMessage', async () => {
    photoStates.mockResolvedValue([st(1, 'pending', { included: false })]);
    matchAll.mockRejectedValue({ message: '', details: '' });
    const wizard = createWizard();
    wizard.go('check');
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    expect(await screen.findByText(copy.checkAll.findingFailed)).toBeTruthy();
    expect(screen.queryByText(copy.checkAll.loading)).toBeNull();
    expect(wizard.busy).toBe(false);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('check');
  });

  it('firstReadFailureShowsItsMessage', async () => {
    photoStates.mockRejectedValue({ message: '', details: '' });
    const wizard = createWizard();
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    expect(await screen.findByText(copy.checkAll.loadFailed)).toBeTruthy();
    expect(screen.queryByText(copy.checkAll.loading)).toBeNull();
  });

  it('refreshFailureAfterMatchHasOwnMessage', async () => {
    listPhotos.mockRejectedValue({ message: '', details: '' });
    photoStates.mockResolvedValue([st(1, 'pending', { included: false })]);
    removalBlockers.mockResolvedValue([]);
    matchAll.mockResolvedValue({ found: 1, unsure: 0, notFound: 0, total: 1 });
    const wizard = createWizard();
    wizard.go('check');
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    expect(await screen.findByText(copy.checkAll.refreshFailed)).toBeTruthy();
    expect(screen.getByRole('button', { name: copy.common.back })).toBeTruthy();
  });

  it('noBannedWordsRendered', async () => {
    setup([st(1, 'found'), st(2, 'unsure'), st(3, 'notFound'), st(4, 'edited')]);
    await primary();
    assertPlainText(document.body);
  });
});
