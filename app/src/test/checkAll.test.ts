import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DetectOutcome, GroupsState, PhotoInfo, PhotoState, ProgressEvent } from '../lib/types';

vi.mock('../lib/api', () => ({
  cancelJob: vi.fn(),
  matchAll: vi.fn(),
  photoStates: vi.fn(),
  listPhotos: vi.fn(),
  setIncluded: vi.fn(),
  removalBlockers: vi.fn(),
  groupsState: vi.fn(),
  groupSample: vi.fn(),
  suggestSaved: vi.fn(),
  detect: vi.fn(),
  useSaved: vi.fn(),
  confirmNewGroup: vi.fn(),
  confirmSetArea: vi.fn(),
  imageUrl: vi.fn((route: string, id: number, v?: number) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`),
}));
// The single-photo editor has its own tests; here it only reports back.
vi.mock('../screens/CheckWatermark.svelte', async () => ({
  default: (await import('./CheckWatermarkStub.svelte')).default,
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { tag } from '../lib/tickets';
import CheckAll from '../screens/CheckAll.svelte';
import { createWizard, type Wizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

const matchAll = vi.mocked(api.matchAll);
const photoStates = vi.mocked(api.photoStates);
const listPhotos = vi.mocked(api.listPhotos);
const setIncluded = vi.mocked(api.setIncluded);
const removalBlockers = vi.mocked(api.removalBlockers);
const groupsState = vi.mocked(api.groupsState);
const groupSample = vi.mocked(api.groupSample);
const suggestSaved = vi.mocked(api.suggestSaved);
const detect = vi.mocked(api.detect);
const useSaved = vi.mocked(api.useSaved);
const cancelJob = vi.mocked(api.cancelJob);

const outcome: DetectOutcome = { sample: [1], representative: 1 };

const info = (id: number): PhotoInfo => ({ id, name: `p${id}.jpg`, folder: '/x', width: 10, height: 10 });

/** The app's rule: ticked, readable, a non-empty area from a group, and not an unopened "?". */
function withMember(s: Omit<PhotoState, 'member'> & { member?: boolean }): PhotoState {
  const member =
    s.group !== null && s.included && s.hasArea && s.status !== 'unreadable' && !(s.status === 'unsure' && !s.reviewed);
  return { ...s, member };
}

/** A photo's state; a result belongs to group 1 unless `extra.group` says otherwise. */
const st = (id: number, status: PhotoState['status'], extra: Partial<PhotoState> = {}): PhotoState =>
  withMember({
    id,
    status,
    included: status === 'found' || status === 'unsure' || status === 'edited',
    reviewed: false,
    hasArea: status === 'found' || status === 'unsure' || status === 'edited',
    group: status === 'pending' || status === 'unreadable' ? null : 1,
    ...extra,
  });

// What the app holds; the mocks below read and change it like the app does.
let store: PhotoState[] = [];
/** The current group. */
let current = 1;
/** The current group's matching has finished; null: done once no photo waits for it. */
let matchedFlag: boolean | null = null;

function groupsOf(states: PhotoState[]): GroupsState {
  const members = new Map<number, number>();
  for (const s of states) if (s.member && s.group !== null) members.set(s.group, (members.get(s.group) ?? 0) + 1);
  return {
    groups: Array.from({ length: current }, (_, i) => ({
      number: i + 1,
      photos: members.get(i + 1) ?? 0,
      savedName: null,
      edited: false,
      canSave: true,
    })),
    current,
    matched: matchedFlag ?? !states.some((s) => s.status === 'pending'),
    leftovers: states.filter((s) => s.status !== 'unreadable' && !s.member).length,
    unreadable: states.filter((s) => s.status === 'unreadable').length,
  };
}

const blockersOf = (states: PhotoState[]) =>
  states.filter((s) => s.status === 'unsure' && s.included && !s.reviewed).map((s) => s.id);

/** The app's tick: unticking a photo of an earlier group drops it from that group. */
function tickInStore(id: number, on: boolean) {
  store = store.map((s) => {
    if (s.id !== id) return s;
    if (!on && s.group !== null && s.group !== current) return st(id, 'pending', { included: false });
    return withMember({ ...s, included: on });
  });
}

/** A wizard in step 3 of group `group`, made from a first guess. */
function wizardAt(group = 1): Wizard {
  const wizard = createWizard();
  wizard.go('add');
  for (let n = 1; n <= group; n++) {
    wizard.startDraft(outcome);
    wizard.confirmDraft();
  }
  wizard.go('check');
  wizard.go('checkAll');
  return wizard;
}

function setup(states: PhotoState[], group = 1) {
  store = states;
  current = group;
  const wizard = wizardAt(group);
  render(CheckAll, { wizard });
  return wizard;
}

const primary = () => screen.findByRole('button', { name: /^Remove watermark from/ }) as Promise<HTMLButtonElement>;
const box = (id: number) => screen.getByRole('checkbox', { name: `Include p${id}.jpg` }) as HTMLInputElement;
const tile = (id: number) => box(id).closest('li') as HTMLElement;
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const findNextOne = async () =>
  fireEvent.click(await screen.findByRole('button', { name: 'Find the next watermark in this photo' }));

describe('Check all photos', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    store = [];
    current = 1;
    matchedFlag = null;
    cancelJob.mockResolvedValue();
    vi.mocked(api.imageUrl).mockImplementation(
      (route, id, v) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`,
    );
    listPhotos.mockImplementation(async () => store.map((s) => info(s.id)));
    photoStates.mockImplementation(async () => store);
    removalBlockers.mockImplementation(async () => blockersOf(store));
    groupsState.mockImplementation(async () => groupsOf(store));
    setIncluded.mockImplementation(async (id, on) => tickInStore(id, on));
    matchAll.mockImplementation(async () => ({ found: 0, unsure: 0, notFound: 0, total: store.length }));
    suggestSaved.mockResolvedValue(null);
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
    setup([st(1, 'found'), st(2, 'unreadable', { included: false }), st(3, 'unreadable', { included: false })]);
    listPhotos.mockResolvedValue([info(1), { ...info(2), problem: 'gone' }, { ...info(3), problem: 'damaged' }]);
    await primary();
    expect(screen.getByText(copy.photoProblem.gone)).toBeTruthy();
    expect(screen.getByText(copy.photoProblem.damaged)).toBeTruthy();
    for (const id of [2, 3]) {
      expect(box(id).disabled).toBe(true);
      // Not offered for opening, never "Not found", and never "No group yet".
      expect(screen.queryByRole('button', { name: new RegExp(`^Open p${id}\\.jpg`) })).toBeNull();
      expect(within(tile(id)).queryByText(copy.groups.noGroup)).toBeNull();
    }
    expect(screen.queryByText(copy.checkAll.badgeNotFound)).toBeNull();
    expect(screen.queryByText(copy.checkAll.noAreaTip)).toBeNull();
    // Photos that cannot be read are not counted (R49 override).
    expect(screen.getByText('Found in 1 of 1 photo.')).toBeTruthy();
  });

  it('summaryOmitsNeedLookAtZero', async () => {
    setup([st(1, 'found'), st(2, 'notFound')]);
    expect(await screen.findByText('Found in 1 of 2 photos.')).toBeTruthy();
  });

  it('buttonCountsIncluded', async () => {
    setup([st(1, 'found'), st(2, 'found'), st(3, 'notFound')]);
    expect((await primary()).textContent?.trim()).toBe('Remove watermark from 2 photos');
    await fireEvent.click(box(1));
    expect(setIncluded).toHaveBeenCalledWith(1, false);
    await waitFor(() => expect(button('Remove watermark from 1 photo')).toBeTruthy());
  });

  it('routesToRemove', async () => {
    const wizard = setup([st(1, 'found')]);
    await fireEvent.click(await primary());
    expect(wizard.screen).toBe('remove');
  });

  it('disabledUntilUnsureHandled', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const remove = await primary();
    expect(remove.disabled).toBe(true);
    expect(screen.getByText(copy.checkAll.needLookHint)).toBeTruthy();
    await fireEvent.click(box(2));
    await waitFor(() => expect(remove.disabled).toBe(false));
    expect(screen.queryByText(copy.checkAll.needLookHint)).toBeNull();
    expect(remove.textContent?.trim()).toBe('Remove watermark from 1 photo');
  });

  it('notFoundStartsUnticked', async () => {
    setup([st(1, 'found'), st(2, 'notFound')]);
    await primary();
    expect(box(1).checked).toBe(true);
    expect(box(2).checked).toBe(false);
    // No area: it cannot be ticked, and the reason is given.
    expect(box(2).disabled).toBe(true);
    expect(box(2).getAttribute('title')).toBe(copy.checkAll.noAreaTip);
    expect(screen.getByText(copy.checkAll.noAreaTip)).toBeTruthy();
    expect(screen.getByText(copy.checkAll.badgeNotFound)).toBeTruthy();
  });

  it('badges', async () => {
    setup([st(1, 'found'), st(2, 'unsure'), st(3, 'edited')]);
    await primary();
    expect(screen.getByText(copy.checkAll.badgeUnsure)).toBeTruthy();
    expect(screen.getByText(copy.checkAll.badgeEdited)).toBeTruthy();
    // The status is part of what a screen reader says for the tile.
    expect(button('Open p1.jpg')).toBeTruthy();
    expect(button('Open p2.jpg, not sure the watermark was found')).toBeTruthy();
    expect(button('Open p3.jpg, checked')).toBeTruthy();
  });

  it('failedTickIsRolledBack', async () => {
    setup([st(1, 'found')]);
    await primary();
    setIncluded.mockRejectedValue({ message: 'nope', details: '' });
    await fireEvent.click(box(1));
    expect(await screen.findByText('nope')).toBeTruthy();
    expect(box(1).checked).toBe(true);
  });

  it('openingPhotoMarksReviewed', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const remove = await primary();
    expect(remove.disabled).toBe(true);
    const open = button(copy.checkAll.openPhoto('p2.jpg', 'unsure'));
    expect(screen.getByAltText('p2.jpg').getAttribute('src')).toMatch(/^puw:\/\/grid\/2\?v=\d+$/);
    const before = screen.getByAltText('p2.jpg').getAttribute('src');
    await fireEvent.click(open);
    // The editor opens the photo (which marks it reviewed), then reports back with a save.
    expect(await screen.findByTestId('stub')).toBeTruthy();
    expect(screen.getByTestId('stub').getAttribute('data-photo')).toBe('2');
    store = [st(1, 'found'), st(2, 'edited', { reviewed: true })];
    await fireEvent.click(button('save'));
    // With no photo left over, Remove is the footer's main button again.
    await waitFor(() => expect(button('Remove watermark from 2 photos').disabled).toBe(false));
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
    store = [st(1, 'pending', { included: false }), st(2, 'pending', { included: false })];
    matchAll.mockImplementation((cb) => {
      emit = cb;
      return tag(
        new Promise((_, reject) => {
          finish = () => reject({ message: 'cancelled', details: '' });
        }),
        41,
      );
    });
    const wizard = wizardAt();
    render(CheckAll, { wizard });
    expect(await screen.findByText('Finding the watermark in all photos... 0 of 2')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('0');
    emit({ phase: 'matching', done: 1, total: 2 });
    expect(await screen.findByText('Finding the watermark in all photos... 1 of 2')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('50');
    expect(wizard.busy).toBe(true);
    await fireEvent.click(button(copy.common.back));
    expect(cancelJob).toHaveBeenCalledTimes(1);
    expect(cancelJob).toHaveBeenCalledWith(41);
    await waitFor(() => expect(wizard.screen).toBe('check'));
    finish();
    await waitFor(() => expect(wizard.busy).toBe(false));
  });

  it('skipsMatchingWhenEveryPhotoHasAResult', async () => {
    setup([st(1, 'found')]);
    await primary();
    expect(matchAll).not.toHaveBeenCalled();
  });

  it('matchesOnlyWhenTheGroupIsNotMatched', async () => {
    // Photo 2 left group 1 and waits for the next group: group 2's matching has finished anyway.
    current = 2;
    matchedFlag = true;
    store = [st(1, 'found', { group: 1 }), st(2, 'pending', { included: false }), st(3, 'found', { group: 2 })];
    const view = render(CheckAll, { wizard: wizardAt(2) });
    await primary();
    expect(matchAll).not.toHaveBeenCalled();
    view.unmount();
    // A group whose matching has not run is matched, even with no photo waiting.
    matchedFlag = false;
    store = [st(1, 'found', { group: 1 }), st(3, 'notFound', { group: 2 })];
    render(CheckAll, { wizard: wizardAt(2) });
    await primary();
    expect(matchAll).toHaveBeenCalledTimes(1);
  });

  it('notFoundStatusInTileName', async () => {
    setup([st(1, 'notFound')]);
    await primary();
    expect(button('Open p1.jpg, watermark not found')).toBeTruthy();
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
    const remove = await primary();
    expect(remove.disabled).toBe(false);
    let answerIncluded: () => void = () => {};
    let answerBlockers: (v: number[]) => void = () => {};
    setIncluded.mockImplementationOnce(
      (id, on) =>
        new Promise<void>((r) => {
          answerIncluded = () => {
            tickInStore(id, on);
            r();
          };
        }),
    );
    removalBlockers.mockImplementationOnce(() => new Promise((r) => (answerBlockers = r)));
    await fireEvent.click(box(2));
    expect(remove.disabled).toBe(true);
    answerIncluded();
    await waitFor(() => expect(removalBlockers).toHaveBeenCalledTimes(2));
    expect(screen.getAllByRole('button', { name: /^Remove watermark from/ }).every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
    answerBlockers([]);
    await waitFor(() => expect(button('Remove watermark from 1 photo').disabled).toBe(false));
  });

  it('blockersFailingAfterTickReadsEverythingAgain', async () => {
    setup([st(1, 'found'), st(2, 'unsure')]);
    const remove = await primary();
    removalBlockers.mockRejectedValueOnce({ message: 'x', details: '' });
    await fireEvent.click(box(2));
    await waitFor(() => expect(remove.disabled).toBe(false));
    expect(box(2).checked).toBe(false);
    expect(screen.queryByText(copy.checkAll.tickFailed)).toBeNull();
  });

  it('unmountBeforeStatesDoesNotMatch', async () => {
    let answer: (s: PhotoState[]) => void = () => {};
    store = [st(1, 'pending', { included: false })];
    photoStates.mockImplementation(() => new Promise((r) => (answer = r)));
    const wizard = wizardAt();
    const view = render(CheckAll, { wizard });
    expect(screen.getByText(copy.checkAll.loading)).toBeTruthy();
    view.unmount();
    answer(store);
    await new Promise((r) => setTimeout(r, 0));
    expect(matchAll).not.toHaveBeenCalled();
    expect(wizard.busy).toBe(false);
  });

  it('emptySetShowsMessageAndBack', async () => {
    const wizard = setup([]);
    expect(await screen.findByText(copy.checkAll.empty)).toBeTruthy();
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('check'));
  });

  it('matchFailureShowsItsMessage', async () => {
    store = [st(1, 'pending', { included: false })];
    matchAll.mockRejectedValue({ message: '', details: '' });
    const wizard = wizardAt();
    render(CheckAll, { wizard });
    expect(await screen.findByText(copy.checkAll.findingFailed)).toBeTruthy();
    expect(screen.queryByText(copy.checkAll.loading)).toBeNull();
    expect(wizard.busy).toBe(false);
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('check'));
  });

  it('firstReadFailureShowsItsMessage', async () => {
    photoStates.mockRejectedValue({ message: '', details: '' });
    render(CheckAll, { wizard: wizardAt() });
    expect(await screen.findByText(copy.checkAll.loadFailed)).toBeTruthy();
    expect(screen.queryByText(copy.checkAll.loading)).toBeNull();
  });

  it('refreshFailureAfterMatchHasOwnMessage', async () => {
    listPhotos.mockRejectedValue({ message: '', details: '' });
    store = [st(1, 'pending', { included: false })];
    render(CheckAll, { wizard: wizardAt() });
    expect(await screen.findByText(copy.checkAll.refreshFailed)).toBeTruthy();
    expect(button(copy.common.back)).toBeTruthy();
  });

  // Groups

  it('earlierGroupTilesShowTheirGroup', async () => {
    setup([st(1, 'found', { group: 1 }), st(2, 'edited', { group: 1 }), st(3, 'found', { group: 2 })], 2);
    await primary();
    expect(within(tile(1)).getByText(copy.groups.label(1))).toBeTruthy();
    expect(within(tile(2)).getByText(copy.groups.label(1))).toBeTruthy();
    expect(within(tile(3)).queryByText(/^Group \d+$/)).toBeNull();
    expect(box(1).checked).toBe(true);
    expect(box(1).disabled).toBe(false);
    // Unticking takes it out of group 1: it waits for the next group.
    await fireEvent.click(box(1));
    expect(setIncluded).toHaveBeenCalledWith(1, false);
    await waitFor(() => expect(within(tile(1)).getByText(copy.groups.noGroup)).toBeTruthy());
    expect(within(tile(1)).queryByText(copy.groups.label(1))).toBeNull();
    expect(box(1).checked).toBe(false);
    expect(box(1).disabled).toBe(true);
    expect(within(tile(1)).getByText(copy.groups.notInGroup)).toBeTruthy();
  });

  it('earlierGroupTileOpensItsOwnArea', async () => {
    setup([st(1, 'edited', { group: 1 }), st(2, 'found', { group: 2 })], 2);
    await primary();
    await fireEvent.click(button(copy.checkAll.openPhoto('p1.jpg', 'edited')));
    const stub = await screen.findByTestId('stub');
    // Single-photo mode for that photo, which shows its own area.
    expect(stub.getAttribute('data-mode')).toBe('photo');
    expect(stub.getAttribute('data-photo')).toBe('1');
  });

  it('noGroupTilesCannotBeTicked', async () => {
    matchedFlag = true;
    setup(
      [
        st(1, 'found', { group: 2 }),
        st(2, 'pending', { included: false }),
        // Unticked in the current group: no group yet, and it can be ticked again.
        st(3, 'found', { group: 2, included: false }),
      ],
      2,
    );
    await primary();
    expect(within(tile(2)).getByText(copy.groups.noGroup)).toBeTruthy();
    expect(box(2).disabled).toBe(true);
    expect(box(2).getAttribute('title')).toBe(copy.groups.notInGroup);
    expect(within(tile(2)).getByText(copy.groups.notInGroup)).toBeTruthy();
    expect(within(tile(3)).getByText(copy.groups.noGroup)).toBeTruthy();
    expect(box(3).disabled).toBe(false);
    expect(within(tile(3)).queryByText(copy.groups.notInGroup)).toBeNull();
    expect(within(tile(1)).queryByText(copy.groups.noGroup)).toBeNull();
    await fireEvent.click(box(3));
    expect(setIncluded).toHaveBeenCalledWith(3, true);
    await waitFor(() => expect(within(tile(3)).queryByText(copy.groups.noGroup)).toBeNull());
  });

  it('groupsLineWithLeftovers', async () => {
    setup(
      [
        ...Array.from({ length: 140 }, (_, i) => st(i, 'found', { group: 1 })),
        ...Array.from({ length: 52 }, (_, i) => st(140 + i, 'found', { group: 2 })),
        ...Array.from({ length: 38 }, (_, i) => st(192 + i, 'notFound', { group: 2 })),
      ],
      2,
    );
    expect(await screen.findByText('Group 1: 140 photos. Group 2: 52 photos. 38 photos have no group yet.')).toBeTruthy();
    expect(button('Find the next watermark in these 38')).toBeTruthy();
    expect(button(copy.groups.useSaved)).toBeTruthy();
    expect(button(copy.groups.leaveOut)).toBeTruthy();
    expect(button('Remove watermark from 192 photos')).toBeTruthy();
  });

  it('groupsLineCountsUnreadablePhotos', async () => {
    // Every photo is counted on screen: in a group, with no group yet, or could not be opened.
    setup([st(1, 'found'), st(2, 'found'), st(3, 'notFound', { group: 1 }), st(4, 'unreadable', { included: false })]);
    expect(
      await screen.findByText('Group 1: 2 photos. 1 photo has no group yet. 1 photo could not be opened.'),
    ).toBeTruthy();
  });

  it('groupsLineWithOnlyUnreadableLeft', async () => {
    setup([st(1, 'found'), st(2, 'unreadable', { included: false }), st(3, 'unreadable', { included: false })]);
    expect(await screen.findByText('Group 1: 1 photo. 2 photos could not be opened.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Find the next watermark/ })).toBeNull();
  });

  it('noLeftoversKeepsTheFooter', async () => {
    setup([st(1, 'found'), st(2, 'found')]);
    await primary();
    expect(screen.queryByRole('button', { name: /^Find the next watermark/ })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.groups.leaveOut })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.groups.useSaved })).toBeNull();
    expect(screen.queryByText(/no group yet/)).toBeNull();
  });

  it('summaryCountsTheCurrentGroup', async () => {
    setup(
      [
        st(1, 'found', { group: 1 }),
        st(2, 'found', { group: 1 }),
        st(3, 'unsure', { group: 1, reviewed: true }),
        st(4, 'found', { group: 2 }),
        st(5, 'edited', { group: 2 }),
        st(6, 'unsure', { group: 2 }),
        st(7, 'notFound', { group: 2 }),
        st(8, 'found', { group: 2, included: false }),
        st(9, 'unreadable', { included: false }),
      ],
      2,
    );
    expect(await screen.findByText('Found in 2 of 5 photos. 1 needs a look.')).toBeTruthy();
  });

  it('headingFromGroupTwo', async () => {
    store = [st(1, 'found')];
    const view = render(CheckAll, { wizard: wizardAt(1) });
    await primary();
    expect(screen.queryByText(copy.groups.checkAllHeading(1))).toBeNull();
    view.unmount();
    current = 2;
    store = [st(1, 'found'), st(2, 'found', { group: 2 })];
    render(CheckAll, { wizard: wizardAt(2) });
    expect(await screen.findByRole('heading', { name: copy.groups.checkAllHeading(2) })).toBeTruthy();
  });

  it('findNextChecksSavedFirstThenFinds', async () => {
    let answer: (v: null) => void = () => {};
    suggestSaved.mockImplementation(() => new Promise((r) => (answer = r)));
    let found: (o: DetectOutcome) => void = () => {};
    detect.mockImplementation(() => new Promise((r) => (found = r)));
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await findNextOne();
    expect(await screen.findByText(copy.addPhotos.checkingSaved)).toBeTruthy();
    expect(detect).not.toHaveBeenCalled();
    expect(wizard.busy).toBe(true);
    answer(null);
    await waitFor(() => expect(detect).toHaveBeenCalledTimes(1));
    const next: DetectOutcome = { sample: [2], representative: 2 };
    found(next);
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(wizard.draft).toEqual({ group: 2, outcome: next });
    expect(wizard.draft?.outcome).toBe(next);
    expect(wizard.groups).toBe(1);
    expect(wizard.busy).toBe(false);
  });

  it('findNextOfferYesUsesSaved', async () => {
    matchedFlag = false;
    matchAll.mockImplementation(async () => {
      matchedFlag = true;
      return { found: 1, unsure: 0, notFound: 1, total: 2 };
    });
    suggestSaved.mockResolvedValue({ id: 's1', name: 'Studio' });
    useSaved.mockImplementation(async () => {
      current = 2;
      matchedFlag = false;
      return 2;
    });
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await findNextOne();
    expect(matchAll).toHaveBeenCalledTimes(1);
    expect(await screen.findByText('These look like Studio. Use it?')).toBeTruthy();
    await fireEvent.click(button(copy.addPhotos.yes));
    await waitFor(() => expect(useSaved).toHaveBeenCalledWith('s1'));
    await waitFor(() => expect(matchAll).toHaveBeenCalledTimes(2));
    expect(wizard.groups).toBe(2);
    expect(wizard.current).toEqual({ group: 2, outcome: null });
    expect(wizard.screen).toBe('checkAll');
    expect(detect).not.toHaveBeenCalled();
    await primary();
  });

  it('findNextOfferNoGoesToTheFirstGuess', async () => {
    suggestSaved.mockResolvedValue({ id: 's1', name: 'Studio' });
    detect.mockImplementation(() => new Promise(() => {}));
    setup([st(1, 'found'), st(2, 'notFound')]);
    await findNextOne();
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.no }));
    await waitFor(() => expect(detect).toHaveBeenCalledTimes(1));
    expect(useSaved).not.toHaveBeenCalled();
  });

  it('failedUseOfTheOfferStaysOnTheGrid', async () => {
    suggestSaved.mockResolvedValue({ id: 's1', name: 'Studio' });
    useSaved.mockRejectedValue({ message: '', details: '' });
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await findNextOne();
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.yes }));
    expect(await screen.findByText(copy.saved.useFailed)).toBeTruthy();
    await primary();
    expect(wizard.groups).toBe(1);
    expect(wizard.busy).toBe(false);
  });

  it('backDuringTheNextGuessKeepsTheGroups', async () => {
    suggestSaved.mockImplementation(() => tag(new Promise(() => {}), 91));
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await findNextOne();
    await screen.findByText(copy.addPhotos.checkingSaved);
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledWith(91));
    await primary();
    expect(wizard.draft).toBeNull();
    expect(wizard.busy).toBe(false);

    suggestSaved.mockResolvedValue(null);
    detect.mockImplementation(() => tag(new Promise(() => {}), 92));
    await findNextOne();
    await waitFor(() => expect(detect).toHaveBeenCalledTimes(1));
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledWith(92));
    await primary();
    expect(cancelJob).toHaveBeenCalledTimes(2);
    expect(wizard.draft).toBeNull();
    expect(wizard.groups).toBe(1);
    expect(wizard.screen).toBe('checkAll');
    expect(wizard.busy).toBe(false);
    expect(useSaved).not.toHaveBeenCalled();
    expect(api.confirmNewGroup).not.toHaveBeenCalled();
    expect(api.confirmSetArea).not.toHaveBeenCalled();
    expect(setIncluded).not.toHaveBeenCalled();
    expect(screen.getByText('Group 1: 1 photo. 1 photo has no group yet.')).toBeTruthy();
  });

  it('leaveThemOutGoesToRemove', async () => {
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await primary();
    await fireEvent.click(button(copy.groups.leaveOut));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(wizard.screen).toBe('remove');
  });

  it('removeWithLeftoversAsksOnce', async () => {
    const wizard = setup([st(1, 'found'), st(2, 'found'), st(3, 'notFound')]);
    await fireEvent.click(await primary());
    let dialog = screen.getByRole('dialog');
    expect(
      within(dialog).getByText('1 photo has no group yet. Remove from the 2 photos in groups and leave it out?'),
    ).toBeTruthy();
    expect(wizard.screen).toBe('checkAll');
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.common.back }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(wizard.screen).toBe('checkAll');
    await fireEvent.click(await primary());
    dialog = screen.getByRole('dialog');
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.groups.confirmRemove }));
    expect(wizard.screen).toBe('remove');
  });

  it('nextStepsWaitForUnsure', async () => {
    setup([st(1, 'found'), st(2, 'unsure'), st(3, 'notFound')]);
    await primary();
    const next = () => [
      button('Find the next watermark in these 2'),
      button(copy.groups.useSaved),
      button(copy.groups.leaveOut),
      button('Remove watermark from 1 photo'),
    ];
    for (const b of next()) expect(b.disabled, b.textContent ?? '').toBe(true);
    // Unticking the "?" lets every next step go, once the tick has reached the app.
    let answer: () => void = () => {};
    setIncluded.mockImplementationOnce(
      (id, on) =>
        new Promise<void>((r) => {
          answer = () => {
            tickInStore(id, on);
            r();
          };
        }),
    );
    await fireEvent.click(box(2));
    for (const b of next()) expect(b.disabled, b.textContent ?? '').toBe(true);
    answer();
    await waitFor(() => {
      for (const b of next()) expect(b.disabled, b.textContent ?? '').toBe(false);
    });
  });

  it('useSavedFromStepThreeOpensChooseMode', async () => {
    const wizard = setup([st(1, 'found'), st(2, 'notFound')]);
    await primary();
    expect(wizard.savedMode).toBe('manage');
    await fireEvent.click(button(copy.groups.useSaved));
    expect(wizard.savedMode).toBe('choose');
    expect(wizard.screen).toBe('saved');
  });

  it('backFetchesTheSampleOfASavedGroup', async () => {
    store = [st(1, 'found'), st(2, 'found')];
    const sample: DetectOutcome = { sample: [2, 1], representative: 2 };
    groupSample.mockResolvedValue(sample);
    const wizard = createWizard();
    wizard.go('add');
    wizard.useGroup(1, [1, 2]);
    wizard.go('checkAll');
    const view = render(CheckAll, { wizard });
    await primary();
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(groupSample).toHaveBeenCalledTimes(1);
    expect(wizard.current).toEqual({ group: 1, outcome: sample });
    view.unmount();
    // A group with its photos keeps them: no second read.
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    await primary();
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(groupSample).toHaveBeenCalledTimes(1);
  });

  it('stoppingTheMatchOfASavedGroupGoesToStepTwo', async () => {
    // F5: never back to Add photos with a group that was not matched.
    store = [st(1, 'pending', { included: false }), st(2, 'pending', { included: false })];
    matchAll.mockImplementation(() => tag(new Promise(() => {}), 51));
    let taken: () => void = () => {};
    cancelJob.mockImplementation(() => new Promise<void>((r) => (taken = r)));
    const sample: DetectOutcome = { sample: [1], representative: 1 };
    groupSample.mockResolvedValue(sample);
    const wizard = createWizard();
    wizard.go('add');
    wizard.useGroup(1, [1, 2]);
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    await screen.findByText('Finding the watermark in all photos... 0 of 2');
    await fireEvent.click(button(copy.common.back));
    expect(cancelJob).toHaveBeenCalledWith(51);
    // Step 2 starts jobs of its own: not before the stop is taken.
    await new Promise((r) => setTimeout(r, 0));
    expect(wizard.screen).toBe('checkAll');
    taken();
    await waitFor(() => expect(wizard.screen).toBe('check'));
    expect(wizard.current).toEqual({ group: 1, outcome: sample });
  });

  it('failedRematchShowsTheAppsStateAndHoldsTheNextSteps', async () => {
    // Review fix 1: "Yes" on the offer, then the new group's matching fails.
    suggestSaved.mockResolvedValue({ id: 's1', name: 'Studio' });
    useSaved.mockImplementation(async () => {
      // As the app does: the photos with no group are reset for group 2.
      current = 2;
      matchedFlag = false;
      store = store.map((s) => (s.member ? s : st(s.id, 'pending', { included: false })));
      return 2;
    });
    const wizard = setup([st(1, 'found'), st(2, 'found', { included: false }), st(3, 'notFound')]);
    await fireEvent.click(await screen.findByRole('button', { name: 'Find the next watermark in these 2' }));
    matchAll.mockRejectedValueOnce({ message: '', details: '' });
    await fireEvent.click(await screen.findByRole('button', { name: copy.addPhotos.yes }));
    expect(await screen.findByText(copy.checkAll.findingFailed)).toBeTruthy();
    expect(wizard.groups).toBe(2);
    // The tiles show what the app has now, not the grid from before.
    await waitFor(() => expect(within(tile(2)).getByText(copy.groups.notInGroup)).toBeTruthy());
    expect(box(2).checked).toBe(false);
    expect(box(2).disabled).toBe(true);
    expect(within(tile(1)).getByText(copy.groups.label(1))).toBeTruthy();
    // Matching is still open in the app: every next step waits for it.
    for (const name of [
      'Find the next watermark in these 2',
      copy.groups.useSaved,
      copy.groups.leaveOut,
      'Remove watermark from 1 photo',
    ]) {
      expect(button(name).disabled, name).toBe(true);
    }
    expect(button(copy.common.back).disabled).toBe(false);
  });

  it('backWhileReadingStartsNoMatch', async () => {
    // Review fix 2: Back during the first read of an unmatched saved group.
    store = [st(1, 'pending', { included: false }), st(2, 'pending', { included: false })];
    let read: () => void = () => {};
    groupsState.mockImplementationOnce(() => new Promise((r) => (read = () => r(groupsOf(store)))));
    let sampled: () => void = () => {};
    groupSample.mockImplementation(() => new Promise((r) => (sampled = () => r(outcome))));
    const wizard = createWizard();
    wizard.go('add');
    wizard.useGroup(1, [1, 2]);
    wizard.go('checkAll');
    render(CheckAll, { wizard });
    await fireEvent.click(await screen.findByRole('button', { name: copy.common.back }));
    await waitFor(() => expect(groupSample).toHaveBeenCalledTimes(1));
    // The read comes back while step 2's photos are still on their way.
    read();
    await new Promise((r) => setTimeout(r, 0));
    sampled();
    await waitFor(() => expect(wizard.screen).toBe('check'));
    await new Promise((r) => setTimeout(r, 0));
    expect(matchAll).not.toHaveBeenCalled();
    expect(wizard.busy).toBe(false);
  });

  it('noBannedWordsRendered', async () => {
    suggestSaved.mockResolvedValue({ id: 's1', name: 'Studio' });
    // Photo 5 left group 1 after group 2 was matched.
    matchedFlag = true;
    setup(
      [
        st(1, 'found', { group: 1 }),
        st(2, 'unsure', { group: 2, reviewed: true }),
        st(3, 'notFound', { group: 2 }),
        st(4, 'edited', { group: 2 }),
        st(5, 'pending', { included: false }),
        st(6, 'found', { group: 2, included: false }),
        st(7, 'unreadable', { included: false }),
      ],
      2,
    );
    await fireEvent.click(await primary());
    expect(screen.getByRole('dialog')).toBeTruthy();
    assertPlainText(document.body);
    await fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: copy.common.back }));
    await fireEvent.click(button('Find the next watermark in these 3'));
    await screen.findByText('These look like Studio. Use it?');
    assertPlainText(document.body);
    for (const text of [copy.groups.nextFailed, copy.saved.useFailed, copy.addPhotos.checkingSaved]) {
      assertPlainText(Object.assign(document.createElement('p'), { textContent: text }));
    }
  });
});
