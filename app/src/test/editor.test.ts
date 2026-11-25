import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DetectOutcome, DeviceStatus, ProgressEvent } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  deviceStatus: vi.fn(),
  deviceFix: vi.fn(),
  cancelJob: vi.fn(),
  editSize: vi.fn(),
  getArea: vi.fn(),
  putDraft: vi.fn(),
  dropDraft: vi.fn(),
  preview: vi.fn(),
  confirmSetArea: vi.fn(),
  openPhoto: vi.fn(),
  savePhotoArea: vi.fn(),
  imageUrl: vi.fn(),
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import CheckWatermark from '../screens/CheckWatermark.svelte';
import { assertPlainText } from './plainText';

const editSize = vi.mocked(api.editSize);
const getArea = vi.mocked(api.getArea);
const putDraft = vi.mocked(api.putDraft);
const preview = vi.mocked(api.preview);
const confirmSetArea = vi.mocked(api.confirmSetArea);
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

/** Step 2 for a set whose sample is `outcome`, with `areas` as each photo's stored area. */
async function openSet(areas: Record<number, Uint8Array>, outcome: DetectOutcome = { sample: [1, 2], representative: 1 }) {
  getArea.mockImplementation(async (id) => (areas[id] ?? empty()).slice());
  const wizard = createWizard();
  wizard.go('add');
  wizard.outcome = outcome;
  wizard.go('check');
  const view = render(CheckWatermark, { wizard });
  await ready();
  return { wizard, ...view };
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
    expect(confirmSetArea).toHaveBeenCalledWith(1);
    expect(putDraft.mock.invocationCallOrder[0]).toBeLessThan(confirmSetArea.mock.invocationCallOrder[0]);
  });

  it('confirmFailureStaysWithMessage', async () => {
    const { wizard } = await openSet({ 1: someArea() });
    confirmSetArea.mockRejectedValueOnce({ message: 'Another job is running.', details: '' });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText('Another job is running.')).toBeTruthy();
    expect(wizard.screen).toBe('check');
  });

  it('emptyAreaShowsHint', async () => {
    const { wizard } = await openSet({ 1: empty() });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText(copy.checkWatermark.emptyArea)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.preview }));
    expect(putDraft).not.toHaveBeenCalled();
    expect(confirmSetArea).not.toHaveBeenCalled();
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
    await waitFor(() => expect(confirmSetArea).toHaveBeenCalled());
    expect(putDraft).toHaveBeenCalledWith(1, W, H, new Uint8Array(W * H).fill(255));
  });

  it('stripKeepsEachPhotosEdits', async () => {
    await openSet({ 1: empty(), 2: someArea() });
    const strip = screen.getByRole('group', { name: copy.checkWatermark.strip });
    const first = screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(1, 2) });
    const second = screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) });
    expect(strip.contains(first)).toBe(true);
    expect(first.getAttribute('aria-pressed')).toBe('true');
    await paint();
    await fireEvent.click(second);
    expect(second.getAttribute('aria-pressed')).toBe('true');
    await waitFor(() => expect(getArea).toHaveBeenCalledWith(2));
    await ready();
    await fireEvent.click(first);
    await ready();
    expect(getArea).toHaveBeenCalledTimes(2);
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmSetArea).toHaveBeenCalledWith(1));
    expect(putDraft).toHaveBeenCalledWith(1, W, H, new Uint8Array(W * H).fill(255));
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
    expect(images).toContain('puw://preview/1?v=1');
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
    await waitFor(() => expect(confirmSetArea).toHaveBeenCalled());
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
    await openSet({ 1: empty() });
    let resolve: (a: Uint8Array) => void = () => {};
    getArea.mockImplementationOnce(() => new Promise((r) => (resolve = r)));
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) }));
    // The first photo's red layer is not shown over the second photo, and painting does nothing.
    expect(redLayer().style.visibility).toBe('hidden');
    await paint();
    await fireEvent.keyDown(window, { key: 'z', ctrlKey: true });
    resolve(someArea());
    await ready();
    expect(redLayer().style.visibility).toBe('visible');
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(1, 2) }));
    await ready();
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    expect(await screen.findByText(copy.checkWatermark.emptyArea)).toBeTruthy();
    expect(putDraft).not.toHaveBeenCalled();
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
    getArea.mockRejectedValueOnce({ message: '', details: 'gone' });
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.stripPhoto(2, 2) }));
    await fireEvent.click(await screen.findByRole('button', { name: copy.checkWatermark.tryAgain }));
    await ready();
    expect(getArea).toHaveBeenLastCalledWith(2);
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
    confirmSetArea.mockImplementationOnce(() => new Promise<void>((r) => (finish = r)));
    await fireEvent.click(screen.getByRole('button', { name: copy.checkWatermark.looksRight }));
    await waitFor(() => expect(confirmSetArea).toHaveBeenCalled());
    expect(screen.getByRole('slider', { name: copy.beforeAfter.divider })).toBeTruthy();
    expect(screen.queryByRole('group', { name: copy.checkWatermark.strip })).toBeNull();
    expect(screen.queryByRole('button', { name: copy.tools.brush })).toBeNull();
    finish();
    await waitFor(() => expect(wizard.screen).toBe('checkAll'));
  });
});
