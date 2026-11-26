import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DetectOutcome, DeviceStatus, Placed, ProgressEvent, StripPlacement } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  deviceStatus: vi.fn(),
  deviceFix: vi.fn(),
  cancelJob: vi.fn(),
  editSize: vi.fn(),
  getArea: vi.fn(),
  getStartArea: vi.fn(),
  putDraft: vi.fn(),
  dropDraft: vi.fn(),
  preview: vi.fn(),
  confirmSetArea: vi.fn(),
  confirmNewGroup: vi.fn(),
  clearGroups: vi.fn(),
  placeOn: vi.fn(),
  openPhoto: vi.fn(),
  savePhotoArea: vi.fn(),
  imageUrl: vi.fn(),
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { SharedDrawing } from '../lib/editor/shared';
import { tag } from '../lib/tickets';
import { createWizard, type Wizard } from '../lib/wizard.svelte';
import CheckWatermark from '../screens/CheckWatermark.svelte';
import { assertPlainText } from './plainText';

const editSize = vi.mocked(api.editSize);
const getArea = vi.mocked(api.getArea);
const getStartArea = vi.mocked(api.getStartArea);
const putDraft = vi.mocked(api.putDraft);
const preview = vi.mocked(api.preview);
const confirmSetArea = vi.mocked(api.confirmSetArea);
const confirmNewGroup = vi.mocked(api.confirmNewGroup);
const clearGroups = vi.mocked(api.clearGroups);
const placeOn = vi.mocked(api.placeOn);
const openPhoto = vi.mocked(api.openPhoto);
const savePhotoArea = vi.mocked(api.savePhotoArea);
const cancelJob = vi.mocked(api.cancelJob);

const W = 4;
const H = 3;
const empty = () => new Uint8Array(W * H);
const someArea = () => {
  const a = empty();
  a[5] = 255;
  a[6] = 255;
  return a;
};
const gpu: DeviceStatus = { state: 'gpu', reason: null, details: null, fixes: [] };

const full = () => new Uint8Array(W * H).fill(255);
/** Where the shared drawing lands on a strip photo: shifted by (dx, dy) at scale 1. */
const placed = (status: Placed, dx = 0, dy = 0): StripPlacement => ({ status, scaleX: 1, scaleY: 1, dx, dy });
/** The view of `drawing` (reference size) on a W x H photo placed with `p`. */
const viewOf = (drawing: Uint8Array, p: StripPlacement) =>
  new SharedDrawing({ w: W, h: H, data: drawing.slice() }).viewOn(p, W, H).data;

/**
 * Step 2 for a set whose sample is `outcome`, with `areas` as each photo's stored area. `as`
 * says which drawing it shows: group 1's draft, group 2's draft (group 1 made from another
 * outcome), or group 1 opened again after it was confirmed.
 */
async function openSet(
  areas: Record<number, Uint8Array>,
  outcome: DetectOutcome = { sample: [1, 2], representative: 1 },
  as: 'first' | 'second' | 'reopened' = 'first',
) {
  getStartArea.mockImplementation(async (id) => (areas[id] ?? empty()).slice());
  const wizard = createWizard();
  wizard.go('add');
  if (as === 'second') {
    wizard.startDraft({ sample: [9], representative: 9 });
    wizard.confirmDraft();
    wizard.go('checkAll');
  }
  wizard.startDraft(outcome);
  if (as === 'reopened') {
    wizard.confirmDraft();
    wizard.go('checkAll');
  }
  wizard.go('check');
  const view = render(CheckWatermark, { wizard });
  await ready();
  return { wizard, ...view };
}

/** Shows the same wizard's step 2 again (a new mount). */
async function openAgain(wizard: Wizard) {
  const view = render(CheckWatermark, { wizard });
  await ready();
  return view;
}

/** Waits until the editor shows an area (the primary button is enabled then). */
async function ready(name: string = copy.checkWatermark.looksRight) {
  await waitFor(() => expect((screen.getByRole('button', { name }) as HTMLButtonElement).disabled).toBe(false));
}

/** The red layer's canvas and the transformed sheet that holds it. */
const redLayer = () => document.querySelector<HTMLCanvasElement>('canvas[data-layer="red"]')!;
const sheet = () => document.querySelector<HTMLElement>('[data-layer="sheet"]')!;

/** Clicks like a browser: a mousedown that is not prevented moves the focus to the element. */
async function click(el: HTMLElement) {
  if (await fireEvent.mouseDown(el)) el.focus();
  await fireEvent.mouseUp(el);
  await fireEvent.click(el);
}

/** Space pressed and released where the focus is, as the keyboard does. */
const spaceDown = () => fireEvent.keyDown(document.activeElement ?? window, { key: ' ' });
const spaceUp = () => fireEvent.keyUp(document.activeElement ?? window, { key: ' ' });

const pressed = (name: string) => screen.getByRole('button', { name }).getAttribute('aria-pressed');
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;

/** What the red layer shows: 255 where it is red, else 0. */
function redPixels(): Uint8Array {
  const image = (redLayer().getContext('2d') as unknown as { image: ImageData | null }).image;
  const out = new Uint8Array(W * H);
  if (image) for (let i = 0; i < out.length; i++) out[i] = image.data[i * 4 + 3] !== 0 ? 255 : 0;
  return out;
}

/** Shows strip photo `n` of `total` and waits until it is shown. */
async function stripTo(n: number, total = 2) {
  await fireEvent.click(button(copy.checkWatermark.stripPhoto(n, total)));
  await ready();
}

const looksRight = () => fireEvent.click(button(copy.checkWatermark.looksRight));

/** Lets every answer that is already on its way arrive. */
const settle = () => new Promise((r) => setTimeout(r, 20));

/** Paints a short stroke at the top-left of the editor; with the default brush it covers the tiny test photo. */
async function paint() {
  const stage = screen.getByRole('application');
  await fireEvent.pointerDown(stage, { pointerId: 1, button: 0, clientX: 0, clientY: 0, pointerType: 'mouse' });
  await fireEvent.pointerMove(stage, { pointerId: 1, button: 0, clientX: 1, clientY: 1, pointerType: 'mouse' });
  await fireEvent.pointerUp(stage, { pointerId: 1, button: 0, clientX: 1, clientY: 1, pointerType: 'mouse' });
}

describe('Check the watermark', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.imageUrl).mockImplementation((route, id, v) => `puw://${route}/${id}${v === undefined ? '' : `?v=${v}`}`);
    vi.mocked(api.appStatus).mockResolvedValue({ modelsReady: true, missingBytes: 0, platform: 'windows', version: '0.1.0' });
    vi.mocked(api.deviceStatus).mockResolvedValue(gpu);
    editSize.mockResolvedValue({ width: W, height: H });
    putDraft.mockResolvedValue();
    confirmSetArea.mockResolvedValue();
    confirmNewGroup.mockResolvedValue();
    clearGroups.mockResolvedValue();
    placeOn.mockResolvedValue(placed('found'));
    openPhoto.mockResolvedValue();
    savePhotoArea.mockResolvedValue();
    cancelJob.mockResolvedValue();
  });

  it('toolShortcutsSwitchTool', async () => {
    await openSet({ 1: someArea() });
    expect(pressed(copy.tools.brush)).toBe('true');
    await fireEvent.keyDown(window, { key: 'e' });
    expect(pressed(copy.tools.eraser)).toBe('true');
    expect(pressed(copy.tools.brush)).toBe('false');
    await fireEvent.keyDown(window, { key: 'R' });
    expect(pressed(copy.tools.rectangle)).toBe('true');
    await fireEvent.keyDown(window, { key: 'b' });
    expect(pressed(copy.tools.brush)).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: copy.tools.eraser }));
    expect(pressed(copy.tools.eraser)).toBe('true');
    // Size keys move the slider, within 2-200.
    const size = screen.getByRole('slider', { name: copy.editor.size }) as HTMLInputElement;
    expect(size.min).toBe('2');
    expect(size.max).toBe('200');
    const before = Number(size.value);
    await fireEvent.keyDown(window, { key: ']' });
    expect(Number(size.value)).toBeGreaterThan(before);
    await fireEvent.keyDown(window, { key: '[' });
    await fireEvent.keyDown(window, { key: '[' });
    expect(Number(size.value)).toBeLessThan(before);
  });

  it('magnifierToggleWithM', async () => {
    await openSet({ 1: someArea() });
    expect(pressed(copy.editor.magnifier)).toBe('false');
    expect(screen.queryByRole('button', { name: copy.editor.times(4) })).toBeNull();
    await fireEvent.keyDown(window, { key: 'm' });
    expect(pressed(copy.editor.magnifier)).toBe('true');
    expect(pressed(copy.editor.times(4))).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: copy.editor.times(8) }));
    expect(pressed(copy.editor.times(8))).toBe('true');
    expect(pressed(copy.editor.times(4))).toBe('false');
    expect(screen.getByRole('button', { name: copy.editor.times(2) })).toBeTruthy();
    await fireEvent.keyDown(window, { key: 'M' });
    expect(pressed(copy.editor.magnifier)).toBe('false');
    // A held key repeats: the toggle happens once per press.
    await fireEvent.keyDown(window, { key: 'm', repeat: true });
    expect(pressed(copy.editor.magnifier)).toBe('false');
  });

  it('looksRightSendsAreaThenConfirms', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(putDraft).toHaveBeenCalledWith(1, W, H, someArea());
    // A drawing not confirmed yet makes the next group.
    expect(confirmNewGroup).toHaveBeenCalledWith(1);
    expect(confirmSetArea).not.toHaveBeenCalled();
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(confirmNewGroup.mock.invocationCallOrder[0]);
    expect(wizard.groups).toBe(1);
    expect(wizard.current?.group).toBe(1);
    expect(wizard.draft).toBeNull();
  });

  it('confirmFailureStaysWithMessage', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    confirmNewGroup.mockRejectedValueOnce({ message: 'Another job is running.', details: '' });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText('Another job is running.')).toBeTruthy();
    expect(wizard.screen).toBe('check');
    expect(wizard.groups).toBe(0);
  });

  it('emptyAreaShowsHint', async () => {
    const { wizard } = await openSet({ 1: empty() });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText(copy.checkWatermark.emptyArea)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    expect(putDraft).not.toHaveBeenCalled();
    expect(confirmNewGroup).not.toHaveBeenCalled();
    expect(preview).not.toHaveBeenCalled();
    expect(wizard.screen).toBe('check');
    // Painting something clears the hint.
    await paint();
    expect(screen.queryByText(copy.checkWatermark.emptyArea)).toBeNull();
  });

  it('paintingChangesTheAreaAndUndoTakesItBack', async () => {
    await openSet({ 1: empty() });
    const undo = screen.getByRole('button', { name: copy.editor.undo }) as HTMLButtonElement;
    const redo = screen.getByRole('button', { name: copy.editor.redo }) as HTMLButtonElement;
    expect(undo.disabled).toBe(true);
    await paint();
    expect(undo.disabled).toBe(false);
    await fireEvent.click(undo);
    expect(redo.disabled).toBe(false);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText(copy.checkWatermark.emptyArea)).toBeTruthy();
    await fireEvent.click(redo);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalled());
    expect(putDraft).toHaveBeenCalledWith(1, W, H, new Uint8Array(W * H).fill(255));
  });

  it('stripShowsTheSharedDrawingOnEachPhoto', async () => {
    const p = placed('found', 1, 0);
    placeOn.mockResolvedValue(p);
    await openSet({ 1: someArea(), 2: empty() });
    const strip = screen.getByRole('group', { name: copy.checkWatermark.strip });
    const first = button(copy.checkWatermark.stripPhoto(1, 2));
    expect(strip.contains(first)).toBe(true);
    expect(first.getAttribute('aria-pressed')).toBe('true');
    expect(redPixels()).toEqual(someArea());
    await stripTo(2);
    expect(button(copy.checkWatermark.stripPhoto(2, 2)).getAttribute('aria-pressed')).toBe('true');
    // The app gets the drawing first, then places it; photo 2's own guess is never read.
    expect(putDraft).toHaveBeenCalledWith(1, W, H, someArea());
    expect(placeOn).toHaveBeenCalledWith(1, 2);
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(placeOn.mock.invocationCallOrder[0]);
    expect(getStartArea).toHaveBeenCalledTimes(1);
    expect(getArea).not.toHaveBeenCalled();
    const want = viewOf(someArea(), p);
    expect(want).not.toEqual(someArea());
    expect(redPixels()).toEqual(want);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('paintingOnAStripPhotoChangesTheSharedDrawing', async () => {
    placeOn.mockResolvedValue(placed('found', 1, 0));
    await openSet({ 1: someArea() });
    await stripTo(2);
    await paint();
    // The view is all red, and the drawing behind it took the part that lies on this photo.
    expect(redPixels()).toEqual(viewOf(full(), placed('found', 1, 0)));
    await looksRight();
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalledWith(1));
    const want = someArea();
    for (let y = 0; y < H; y++) for (let x = 0; x < W - 1; x++) want[y * W + x] = 255;
    expect(putDraft).toHaveBeenLastCalledWith(1, W, H, want);
  });

  it('looksRightFromAStripPhotoUsesTheSharedDrawing', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    placeOn.mockResolvedValue(placed('found', 1, 1));
    await stripTo(2);
    await looksRight();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(confirmNewGroup).toHaveBeenCalledWith(1);
    expect(confirmSetArea).not.toHaveBeenCalled();
    for (const call of putDraft.mock.calls) expect(call).toEqual([1, W, H, someArea()]);
    expect(putDraft.mock.invocationCallOrder.at(-1)).toBeLessThan(confirmNewGroup.mock.invocationCallOrder[0]);
  });

  it('unsureStripPhotoShowsTheDrawingWithPaintingOff', async () => {
    const p = placed('unsure', 1, 0);
    placeOn.mockResolvedValue(p);
    await openSet({ 1: someArea() });
    await stripTo(2);
    expect(screen.getByRole('status').textContent).toBe(copy.checkWatermark.unsureHere);
    expect(redPixels()).toEqual(viewOf(someArea(), p));
    await paint();
    expect(redPixels()).toEqual(viewOf(someArea(), p));
    expect(button(copy.editor.undo).disabled).toBe(true);
    expect(button(copy.checkWatermark.preview).disabled).toBe(false);
    // Looking around still works: Space and drag pans.
    const before = sheet().style.transform;
    const stage = screen.getByRole('application');
    await spaceDown();
    await fireEvent.pointerDown(stage, { pointerId: 2, button: 0, clientX: 0, clientY: 0, pointerType: 'mouse' });
    await fireEvent.pointerMove(stage, { pointerId: 2, button: 0, clientX: 30, clientY: 20, pointerType: 'mouse' });
    await fireEvent.pointerUp(stage, { pointerId: 2, button: 0, clientX: 30, clientY: 20, pointerType: 'mouse' });
    await spaceUp();
    expect(sheet().style.transform).not.toBe(before);
    await looksRight();
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalledWith(1));
    expect(putDraft).toHaveBeenLastCalledWith(1, W, H, someArea());
  });

  it('notFoundStripPhotoShowsTheNote', async () => {
    placeOn.mockResolvedValue({ status: 'notFound', scaleX: 0, scaleY: 0, dx: 0, dy: 0 });
    await openSet({ 1: someArea() });
    await stripTo(2);
    expect(screen.getByRole('status').textContent).toBe(copy.checkWatermark.notFoundHere);
    expect(redPixels()).toEqual(empty());
    expect(button(copy.checkWatermark.preview).disabled).toBe(true);
    await paint();
    expect(redPixels()).toEqual(empty());
    expect(button(copy.editor.undo).disabled).toBe(true);
    // The drawing is still there for the group.
    expect(button(copy.checkWatermark.looksRight).disabled).toBe(false);
    await stripTo(1);
    expect(screen.queryByRole('status')).toBeNull();
    expect(redPixels()).toEqual(someArea());
  });

  it('undoCoversEditsOnAnyStripPhoto', async () => {
    placeOn.mockResolvedValue(placed('found', 1, 0));
    await openSet({ 1: someArea() });
    await stripTo(2);
    await paint();
    await stripTo(1);
    const changed = someArea();
    for (let y = 0; y < H; y++) for (let x = 0; x < W - 1; x++) changed[y * W + x] = 255;
    expect(redPixels()).toEqual(changed);
    expect(button(copy.editor.undo).disabled).toBe(false);
    await fireEvent.keyDown(window, { key: 'z', ctrlKey: true });
    expect(redPixels()).toEqual(someArea());
    expect(button(copy.editor.undo).disabled).toBe(true);
    expect(button(copy.editor.redo).disabled).toBe(false);
    await fireEvent.click(button(copy.editor.redo));
    expect(redPixels()).toEqual(changed);
    await fireEvent.click(button(copy.editor.undo));
    await looksRight();
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalled());
    expect(putDraft).toHaveBeenLastCalledWith(1, W, H, someArea());
  });

  it('previewOnAStripPhotoSendsItsView', async () => {
    const p = placed('found', 1, 0);
    placeOn.mockResolvedValue(p);
    preview.mockResolvedValue();
    await openSet({ 1: someArea() });
    await stripTo(2);
    putDraft.mockClear();
    await fireEvent.click(button(copy.checkWatermark.preview));
    await screen.findByRole('button', { name: copy.checkWatermark.backToEditing });
    expect(putDraft).toHaveBeenCalledWith(2, W, H, viewOf(someArea(), p));
    expect(preview).toHaveBeenCalledWith(2, expect.any(Function));
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(preview.mock.invocationCallOrder[0]);
  });

  it('placementIsAskedAgainOnlyAfterTheDrawingChanged', async () => {
    await openSet({ 1: someArea() });
    // The reference photo shows the drawing as it is, with nothing to ask.
    expect(placeOn).not.toHaveBeenCalled();
    await stripTo(2);
    expect(placeOn).toHaveBeenCalledTimes(1);
    await stripTo(1);
    await stripTo(2);
    expect(placeOn).toHaveBeenCalledTimes(1);
    // Edits made on photo 2 keep its placement.
    await paint();
    await stripTo(1);
    await stripTo(2);
    expect(placeOn).toHaveBeenCalledTimes(1);
    // A change made elsewhere (here an undo on photo 1) asks again.
    await stripTo(1);
    await fireEvent.keyDown(window, { key: 'z', ctrlKey: true });
    await stripTo(2);
    expect(placeOn).toHaveBeenCalledTimes(2);
  });

  it('placeFailedShowsNote', async () => {
    await openSet({ 1: someArea() });
    placeOn.mockRejectedValueOnce({ message: 'Another job is running.', details: '' });
    await stripTo(2);
    expect(screen.getByRole('status').textContent).toBe(copy.checkWatermark.placeFailed);
    expect(screen.queryByText('Another job is running.')).toBeNull();
    expect(button(copy.checkWatermark.preview).disabled).toBe(true);
    await paint();
    expect(button(copy.editor.undo).disabled).toBe(true);
    await fireEvent.click(button(copy.checkWatermark.tryAgain));
    await waitFor(() => expect(placeOn).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole('status')).toBeNull());
    expect(button(copy.checkWatermark.preview).disabled).toBe(false);
    expect(redPixels()).toEqual(someArea());
  });

  it('switchingPhotosStopsThePlacement', async () => {
    const { wizard } = await openSet({ 1: someArea() }, { sample: [1, 2, 3], representative: 1 }, 'reopened');
    placeOn.mockImplementation((_r, id) =>
      id === 2 ? tag(new Promise<StripPlacement>(() => {}), 77) : Promise.resolve(placed('found')),
    );
    // The app takes a stop only when the test says so.
    let take: () => void = () => {};
    cancelJob.mockImplementation(() => new Promise<void>((r) => (take = r)));
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(2, 3)));
    expect(await screen.findByText(copy.checkWatermark.placing)).toBeTruthy();
    await waitFor(() => expect(placeOn).toHaveBeenCalledWith(1, 2));
    const sent = putDraft.mock.calls.length;
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(3, 3)));
    expect(cancelJob).toHaveBeenCalledWith(77);
    // Nothing more reaches the app until it has taken the stop.
    await settle();
    expect(placeOn).toHaveBeenCalledTimes(1);
    expect(putDraft).toHaveBeenCalledTimes(sent);
    take();
    await ready();
    expect(placeOn).toHaveBeenLastCalledWith(1, 3);
    expect(screen.queryByText(copy.checkWatermark.placing)).toBeNull();
    // Leaving step 2 while a photo is placed stops it too, and the groups are cleared only after.
    cancelJob.mockClear();
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(2, 3)));
    await waitFor(() => expect(placeOn).toHaveBeenCalledTimes(3));
    await fireEvent.click(button(copy.common.back));
    expect(cancelJob).toHaveBeenCalledWith(77);
    await settle();
    expect(clearGroups).not.toHaveBeenCalled();
    take();
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(clearGroups).toHaveBeenCalledTimes(1);
    expect(cancelJob.mock.invocationCallOrder[0]).toBeLessThan(clearGroups.mock.invocationCallOrder[0]);
  });

  it('confirmWaitsForAStoppedPlacement', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    placeOn.mockImplementation(() => tag(new Promise<StripPlacement>(() => {}), 78));
    let take: () => void = () => {};
    cancelJob.mockImplementation(() => new Promise<void>((r) => (take = r)));
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(2, 2)));
    await waitFor(() => expect(placeOn).toHaveBeenCalledWith(1, 2));
    // Back on the reference photo, which shows at once, and "Looks right" right away.
    await stripTo(1);
    expect(cancelJob).toHaveBeenCalledWith(78);
    putDraft.mockClear();
    await looksRight();
    await settle();
    expect(putDraft).not.toHaveBeenCalled();
    expect(confirmNewGroup).not.toHaveBeenCalled();
    take();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(confirmNewGroup).toHaveBeenCalledWith(1);
  });

  it('draftLooksRightMakesTheNextGroup', async () => {
    const { wizard } = await openSet({ 3: someArea() }, { sample: [3, 4], representative: 3 }, 'second');
    expect(wizard.groups).toBe(1);
    await looksRight();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(putDraft).toHaveBeenLastCalledWith(3, W, H, someArea());
    expect(confirmNewGroup).toHaveBeenCalledWith(3);
    expect(confirmSetArea).not.toHaveBeenCalled();
    expect(wizard.groups).toBe(2);
    expect(wizard.current?.group).toBe(2);
    expect(wizard.draft).toBeNull();
  });

  it('reopenedGroupConfirmsItsArea', async () => {
    const { wizard } = await openSet({ 1: someArea() }, undefined, 'reopened');
    await looksRight();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(confirmSetArea).toHaveBeenCalledWith(1);
    expect(confirmNewGroup).not.toHaveBeenCalled();
    expect(wizard.groups).toBe(1);
  });

  it('backFromADraftReturnsToStepThree', async () => {
    const { wizard } = await openSet({ 3: someArea() }, { sample: [3, 4], representative: 3 }, 'second');
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
    expect(clearGroups).not.toHaveBeenCalled();
    expect(wizard.draft).toBeNull();
    expect(wizard.groups).toBe(1);
    expect(wizard.current?.group).toBe(1);
  });

  it('backFromGroupOneClearsGroupsAndKeepsTheDrawing', async () => {
    const outcome = { sample: [1, 2], representative: 1 };
    const { wizard, unmount } = await openSet({ 1: empty() }, outcome, 'reopened');
    await paint();
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(clearGroups).toHaveBeenCalledTimes(1);
    expect(wizard.groups).toBe(0);
    expect(wizard.current).toBeNull();
    expect(wizard.draft?.outcome).toBe(outcome);
    unmount();
    wizard.go('check');
    await openAgain(wizard);
    expect(getStartArea).toHaveBeenCalledTimes(1);
    expect(redPixels()).toEqual(full());
    expect(button(copy.editor.undo).disabled).toBe(false);
    await fireEvent.click(button(copy.editor.undo));
    expect(redPixels()).toEqual(empty());
    // Now a draft again: Back goes to Add photos without clearing anything.
    await fireEvent.click(button(copy.common.back));
    await waitFor(() => expect(wizard.screen).toBe('add'));
    expect(clearGroups).toHaveBeenCalledTimes(1);
  });

  it('failedClearingStaysOnStepTwo', async () => {
    const { wizard } = await openSet({ 1: someArea() }, undefined, 'reopened');
    placeOn.mockImplementationOnce(() => new Promise<StripPlacement>(() => {}));
    await fireEvent.click(button(copy.checkWatermark.stripPhoto(2, 2)));
    expect(await screen.findByText(copy.checkWatermark.placing)).toBeTruthy();
    clearGroups.mockRejectedValueOnce({ message: 'Another job is running.', details: '' });
    await fireEvent.click(button(copy.common.back));
    expect(await screen.findByText('Another job is running.')).toBeTruthy();
    expect(wizard.screen).toBe('check');
    expect(wizard.groups).toBe(1);
    // The stopped placement is asked again, so the photo does not stay half shown.
    await waitFor(() => expect(placeOn).toHaveBeenCalledTimes(2));
    await ready();
    expect(screen.queryByText(copy.checkWatermark.placing)).toBeNull();
  });

  it('headingFromGroupTwo', async () => {
    const first = await openSet({ 1: someArea() });
    expect(screen.queryByText(copy.groups.checkHeading(1))).toBeNull();
    first.unmount();
    await openSet({ 3: someArea() }, { sample: [3, 4], representative: 3 }, 'second');
    expect(screen.getByRole('heading', { name: copy.groups.checkHeading(2) })).toBeTruthy();
  });

  it('emptyFirstGuessOpensEmpty', async () => {
    await openSet({ 1: empty() });
    await stripTo(2);
    // Nothing to place: an empty view, painting off, and no note.
    expect(placeOn).not.toHaveBeenCalled();
    expect(screen.queryByRole('status')).toBeNull();
    expect(redPixels()).toEqual(empty());
    await paint();
    expect(redPixels()).toEqual(empty());
    expect(button(copy.editor.undo).disabled).toBe(true);
    await looksRight();
    expect(await screen.findByText(copy.checkWatermark.emptyArea)).toBeTruthy();
    expect(confirmNewGroup).not.toHaveBeenCalled();
    // The reference photo can be painted, and then the drawing is placed on the others.
    await stripTo(1);
    await paint();
    expect(button(copy.editor.undo).disabled).toBe(false);
    await stripTo(2);
    expect(placeOn).toHaveBeenCalledWith(1, 2);
    expect(redPixels()).toEqual(full());
  });

  it('previewShowsRemoverLoadingSentence', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    let emit: (e: ProgressEvent) => void = () => {};
    let finish: () => void = () => {};
    preview.mockImplementation((_id, onEvent) => {
      emit = onEvent;
      return new Promise<void>((resolve) => (finish = resolve));
    });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    await waitFor(() => expect(preview).toHaveBeenCalledWith(1, expect.any(Function)));
    expect(putDraft).toHaveBeenCalledWith(1, W, H, someArea());
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(preview.mock.invocationCallOrder[0]);
    expect(wizard.busy).toBe(true);
    emit({ phase: 'loadingModels', fraction: 0.3, which: 'remover' });
    expect(await screen.findByText(copy.loading.remover)).toBeTruthy();
    emit({ phase: 'preview', fraction: 0.5 });
    expect(await screen.findByText(copy.checkWatermark.trying)).toBeTruthy();
    finish();
    const back = await screen.findByRole('button', { name: copy.checkWatermark.backToEditing });
    expect(wizard.busy).toBe(false);
    const images = [...document.querySelectorAll('img')].map((i) => i.getAttribute('src'));
    expect(images).toContain('puw://edit/1');
    // The version only grows across mounts, so earlier tests' previews change the number.
    expect(images.some((src) => /^puw:\/\/preview\/1\?v=\d+$/.test(src ?? ''))).toBe(true);
    expect(screen.getByRole('slider', { name: copy.beforeAfter.divider })).toBeTruthy();
    expect(await screen.findByText(copy.device.gpu)).toBeTruthy();
    await fireEvent.click(back);
    expect(screen.getByRole('button', { name: copy.tools.brush })).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.checkWatermark.backToEditing })).toBeNull();
  });

  it('previewCanBeCancelled', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    preview.mockImplementation(() => new Promise<void>(() => {}));
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    await fireEvent.click(await screen.findByRole('button', { name: copy.common.cancel }));
    expect(cancelJob).toHaveBeenCalled();
    expect(await screen.findByRole('button', { name: copy.tools.brush })).toBeTruthy();
    expect(wizard.busy).toBe(false);
  });

  it('backGoesToAddPhotos', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('add');
  });

  it('photoModeSavesOnlyThisPhoto', async () => {
    getArea.mockResolvedValue(someArea());
    const wizard = createWizard();
    const onDone = vi.fn();
    render(CheckWatermark, { wizard, mode: 'photo', photoId: 7, onDone });
    await ready(copy.checkWatermark.savePhoto);
    expect(openPhoto).toHaveBeenCalledWith(7);
    expect(getArea).toHaveBeenCalledWith(7);
    expect(getStartArea).not.toHaveBeenCalled();
    expect(screen.getByText(copy.checkWatermark.photoHelp)).toBeTruthy();
    expect(screen.queryByText(copy.checkWatermark.help)).toBeNull();
    expect(screen.queryByRole('group', { name: copy.checkWatermark.strip })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.checkWatermark.looksRight })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.savePhoto }));
    await waitFor(() => expect(onDone).toHaveBeenCalledWith(true));
    expect(putDraft).toHaveBeenCalledWith(7, W, H, someArea());
    expect(savePhotoArea).toHaveBeenCalledWith(7);
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(savePhotoArea.mock.invocationCallOrder[0]);
    expect(confirmSetArea).not.toHaveBeenCalled();
  });

  it('photoModeBackDiscards', async () => {
    getArea.mockResolvedValue(someArea());
    const onDone = vi.fn();
    render(CheckWatermark, { wizard: createWizard(), mode: 'photo', photoId: 7, onDone });
    await ready(copy.checkWatermark.savePhoto);
    await paint();
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(onDone).toHaveBeenCalledWith(false);
    expect(putDraft).not.toHaveBeenCalled();
    expect(savePhotoArea).not.toHaveBeenCalled();
    // An area sent for a preview is dropped too.
    await waitFor(() => expect(vi.mocked(api.dropDraft)).toHaveBeenCalledWith(7));
  });

  it('noBannedWordsRendered', async () => {
    const set = await openSet({ 1: someArea() });
    await fireEvent.keyDown(window, { key: 'm' });
    expect(screen.getByText(copy.checkWatermark.help)).toBeTruthy();
    expect(screen.getByText(copy.editor.spaceHint)).toBeTruthy();
    assertPlainText(set.container);
    preview.mockResolvedValue();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    await screen.findByRole('button', { name: copy.checkWatermark.backToEditing });
    assertPlainText(set.container);
    set.unmount();

    // Every note over a strip photo, and the group heading.
    placeOn.mockImplementation((_r, id) => {
      if (id === 2) return Promise.resolve(placed('unsure'));
      if (id === 3) return Promise.resolve({ status: 'notFound', scaleX: 0, scaleY: 0, dx: 0, dy: 0 });
      if (id === 4) return new Promise<StripPlacement>(() => {});
      return Promise.reject({ message: '', details: 'gone' });
    });
    const strip = await openSet({ 1: someArea() }, { sample: [1, 2, 3, 4, 5], representative: 1 }, 'second');
    expect(screen.getByText(copy.groups.checkHeading(2))).toBeTruthy();
    assertPlainText(strip.container);
    const notes = [
      copy.checkWatermark.unsureHere,
      copy.checkWatermark.notFoundHere,
      copy.checkWatermark.placing,
      copy.checkWatermark.placeFailed,
    ];
    for (const [i, note] of notes.entries()) {
      await fireEvent.click(button(copy.checkWatermark.stripPhoto(i + 2, 5)));
      expect(await screen.findByText(note)).toBeTruthy();
      assertPlainText(strip.container);
    }
    strip.unmount();

    getArea.mockResolvedValue(empty());
    const photo = render(CheckWatermark, { wizard: createWizard(), mode: 'photo', photoId: 3, onDone: () => {} });
    await ready(copy.checkWatermark.savePhoto);
    await fireEvent.keyDown(window, { key: 'm' });
    assertPlainText(photo.container);
  });

  it('previewUrlChangesAcrossMounts', async () => {
    preview.mockResolvedValue();
    const urls: string[] = [];
    for (let i = 0; i < 2; i++) {
      const view = await openSet({ 1: someArea() });
      await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
      await screen.findByRole('button', { name: copy.checkWatermark.backToEditing });
      urls.push(screen.getByAltText(copy.beforeAfter.after).getAttribute('src')!);
      view.unmount();
    }
    expect(urls[0]).toMatch(/^puw:\/\/preview\/1\?v=\d+$/);
    expect(urls[1]).not.toBe(urls[0]);
  });

  it('holdSpaceHidesRedAndReleaseShowsIt', async () => {
    await openSet({ 1: someArea() });
    expect(redLayer().style.visibility).toBe('visible');
    await spaceDown();
    expect(redLayer().style.visibility).toBe('hidden');
    await spaceUp();
    expect(redLayer().style.visibility).toBe('visible');
  });

  it('spaceDragPansWithoutPainting', async () => {
    await openSet({ 1: someArea() });
    const before = sheet().style.transform;
    const stage = screen.getByRole('application');
    await spaceDown();
    await fireEvent.pointerDown(stage, { pointerId: 1, button: 0, clientX: 0, clientY: 0, pointerType: 'mouse' });
    // While dragging the red layer shows again.
    expect(redLayer().style.visibility).toBe('visible');
    await fireEvent.pointerMove(stage, { pointerId: 1, button: 0, clientX: 30, clientY: 20, pointerType: 'mouse' });
    await fireEvent.pointerUp(stage, { pointerId: 1, button: 0, clientX: 30, clientY: 20, pointerType: 'mouse' });
    await spaceUp();
    expect(sheet().style.transform).not.toBe(before);
    expect((screen.getByRole('button', { name: copy.editor.undo }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalled());
    expect(putDraft).toHaveBeenCalledWith(1, W, H, someArea());
  });

  it('spaceStillHidesRedAfterClickingButtons', async () => {
    preview.mockResolvedValue();
    await openSet({ 1: someArea(), 2: someArea() });
    await click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) }));
    await ready();
    await spaceDown();
    expect(redLayer().style.visibility).toBe('hidden');
    await spaceUp();
    expect(redLayer().style.visibility).toBe('visible');
    // The same after Preview and "Back to editing".
    await click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    await click(await screen.findByRole('button', { name: copy.checkWatermark.backToEditing }));
    await spaceDown();
    expect(redLayer().style.visibility).toBe('hidden');
    await spaceUp();
  });

  it('spaceIsReleasedOnWindowBlur', async () => {
    await openSet({ 1: someArea() });
    await spaceDown();
    expect(redLayer().style.visibility).toBe('hidden');
    await fireEvent.blur(window);
    expect(redLayer().style.visibility).toBe('visible');
  });

  it('spaceDuringAStrokeKeepsRed', async () => {
    await openSet({ 1: empty() });
    const stage = screen.getByRole('application');
    await fireEvent.pointerDown(stage, { pointerId: 1, button: 0, clientX: 0, clientY: 0, pointerType: 'mouse' });
    await spaceDown();
    expect(redLayer().style.visibility).toBe('visible');
    await fireEvent.pointerUp(stage, { pointerId: 1, button: 0, clientX: 0, clientY: 0, pointerType: 'mouse' });
    await spaceUp();
    expect((screen.getByRole('button', { name: copy.editor.undo }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('noStrokesWhileTheNextPhotoLoads', async () => {
    await openSet({ 1: someArea() });
    let resolve: (p: StripPlacement) => void = () => {};
    placeOn.mockImplementationOnce(() => new Promise((r) => (resolve = r)));
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) }));
    // The first photo's red layer is not shown over the second photo, and painting does nothing.
    expect(redLayer().style.visibility).toBe('hidden');
    expect(await screen.findByText(copy.checkWatermark.placing)).toBeTruthy();
    expect(button(copy.checkWatermark.looksRight).disabled).toBe(true);
    await paint();
    await fireEvent.keyDown(window, { key: 'z', ctrlKey: true });
    resolve(placed('found'));
    await ready();
    expect(redLayer().style.visibility).toBe('visible');
    expect(screen.queryByText(copy.checkWatermark.placing)).toBeNull();
    expect(button(copy.editor.undo).disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(1, 2) }));
    await ready();
    putDraft.mockClear();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalled());
    expect(putDraft).toHaveBeenCalledWith(1, W, H, someArea());
  });

  it('failedAreaLoadOffersTryAgain', async () => {
    getArea.mockRejectedValueOnce({ message: '', details: 'gone' });
    getArea.mockResolvedValueOnce(someArea());
    render(CheckWatermark, { wizard: createWizard(), mode: 'photo', photoId: 7, onDone: () => {} });
    expect(await screen.findByText(copy.checkWatermark.areaFailed)).toBeTruthy();
    const save = screen.getByRole('button', { name: copy.checkWatermark.savePhoto }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.tryAgain }));
    await ready(copy.checkWatermark.savePhoto);
    expect(getArea).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(copy.checkWatermark.areaFailed)).toBeNull();
    expect(screen.queryByRole('button', { name: copy.checkWatermark.tryAgain })).toBeNull();
  });

  it('failedStripLoadOffersTryAgain', async () => {
    await openSet({ 1: someArea() });
    editSize.mockRejectedValueOnce({ message: '', details: 'gone' });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) }));
    expect(await screen.findByText(copy.checkWatermark.areaFailed)).toBeTruthy();
    await fireEvent.click(await screen.findByRole('button', { name: copy.checkWatermark.tryAgain }));
    await ready();
    expect(editSize).toHaveBeenLastCalledWith(2);
    expect(screen.queryByText(copy.checkWatermark.areaFailed)).toBeNull();
  });

  it('failedDrawingLoadOffersTryAgain', async () => {
    getStartArea.mockRejectedValueOnce({ message: '', details: 'gone' });
    getStartArea.mockResolvedValueOnce(someArea());
    const wizard = createWizard();
    wizard.go('add');
    wizard.startDraft({ sample: [1, 2], representative: 1 });
    wizard.go('check');
    render(CheckWatermark, { wizard });
    expect(await screen.findByText(copy.checkWatermark.areaFailed)).toBeTruthy();
    expect(button(copy.checkWatermark.looksRight).disabled).toBe(true);
    await fireEvent.click(button(copy.checkWatermark.tryAgain));
    await ready();
    expect(getStartArea).toHaveBeenCalledTimes(2);
    expect(redPixels()).toEqual(someArea());
  });

  it('shortcutLabelsComeFromCopy', async () => {
    await openSet({ 1: someArea() });
    const k = copy.editor.keys;
    const title = (name: string) => screen.getByRole('button', { name }).getAttribute('title');
    expect(title(copy.tools.brush)).toBe(copy.editor.withKey(copy.tools.brush, k.brush));
    expect(title(copy.tools.rectangle)).toBe(copy.editor.withKey(copy.tools.rectangle, k.rectangle));
    expect(title(copy.editor.undo)).toBe(copy.editor.withKey(copy.editor.undo, k.undo(false)));
    expect(title(copy.editor.redo)).toBe(copy.editor.withKey(copy.editor.redo, k.redo(false)));
    expect(title(copy.editor.magnifier)).toBe(copy.editor.withKey(copy.editor.magnifier, k.magnifier));
    expect(k.undo(true)).toBe('Cmd+Z');
    expect(k.redo(true)).toBe('Shift+Cmd+Z');
    expect(k.redo(false)).toBe('Ctrl+Y');
  });

  it('confirmingFromThePreviewKeepsThePreview', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    preview.mockResolvedValue();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    await screen.findByRole('button', { name: copy.checkWatermark.backToEditing });
    let finish: () => void = () => {};
    confirmNewGroup.mockImplementationOnce(() => new Promise<void>((r) => (finish = r)));
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmNewGroup).toHaveBeenCalled());
    expect(screen.getByRole('slider', { name: copy.beforeAfter.divider })).toBeTruthy();
    expect(screen.queryByRole('group', { name: copy.checkWatermark.strip })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.tools.brush })).toBeNull();
    finish();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
  });
});
