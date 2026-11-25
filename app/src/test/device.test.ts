import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppStatus, DeviceStatus } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  deviceStatus: vi.fn(),
  deviceFix: vi.fn(),
}));

import * as api from '../lib/api';
import DeviceChip from '../lib/components/DeviceChip.svelte';
import WhyDialog from '../lib/components/WhyDialog.svelte';
import { copy } from '../lib/copy';
import { assertPlainText } from './plainText';

const appStatus = vi.mocked(api.appStatus);
const deviceStatus = vi.mocked(api.deviceStatus);
const deviceFix = vi.mocked(api.deviceFix);

const status = (platform: string): AppStatus => ({ modelsReady: true, missingBytes: 0, platform, version: '0.1.0' });
const gpu: DeviceStatus = { state: 'gpu', reason: null, details: null, fixes: [] };
const notStarted: DeviceStatus = { state: 'notStarted', reason: null, details: null, fixes: [] };
const loadFailed: DeviceStatus = {
  state: 'cpu',
  reason: 'The graphics card could not load the watermark remover.',
  details: 'watermark remover: DirectML error 0x887A0005',
  fixes: ['tryAgain', 'useProcessor'],
};
const shared: DeviceStatus = {
  state: 'gpu',
  reason: 'Most of the work runs on the Neural Engine; a small part runs on the processor.',
  details: 'watermark remover: Core ML runs 1680 of 2877 steps',
  fixes: [],
};

describe('DeviceChip', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    appStatus.mockResolvedValue(status('windows'));
  });

  it('chipTexts', async () => {
    deviceStatus.mockResolvedValue(gpu);
    const windows = render(DeviceChip);
    expect(await screen.findByText(copy.device.gpu)).toBeTruthy();
    windows.unmount();

    appStatus.mockResolvedValue(status('macos'));
    const mac = render(DeviceChip);
    expect(await screen.findByText(copy.device.neuralEngine)).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
    mac.unmount();

    deviceStatus.mockResolvedValue(loadFailed);
    const slow = render(DeviceChip);
    expect(await screen.findByRole('button', { name: copy.device.cpu })).toBeTruthy();
    slow.unmount();

    deviceStatus.mockResolvedValue(notStarted);
    const onStep = render(DeviceChip);
    await waitFor(() => expect(deviceStatus).toHaveBeenCalledTimes(4));
    await Promise.resolve();
    expect(onStep.container.textContent?.trim()).toBe('');
    onStep.unmount();

    render(DeviceChip, { inSettings: true });
    expect(await screen.findByText(copy.device.notStarted)).toBeTruthy();
  });

  it('platformPropSkipsAppStatus', async () => {
    deviceStatus.mockResolvedValue(gpu);
    render(DeviceChip, { platform: 'macos' });
    expect(await screen.findByText(copy.device.neuralEngine)).toBeTruthy();
    expect(appStatus).not.toHaveBeenCalled();
  });

  it('refreshAsksAgain', async () => {
    deviceStatus.mockResolvedValueOnce(notStarted).mockResolvedValueOnce(gpu);
    const view = render(DeviceChip, { inSettings: true, refresh: 0 });
    await screen.findByText(copy.device.notStarted);
    await view.rerender({ inSettings: true, refresh: 1 });
    expect(await screen.findByText(copy.device.gpu)).toBeTruthy();
  });

  it('whyShowsFixes', async () => {
    deviceStatus.mockResolvedValueOnce(loadFailed).mockResolvedValue(notStarted);
    deviceFix.mockResolvedValue();
    const onChange = vi.fn();
    render(DeviceChip, { inSettings: true, onChange });
    const chip = await screen.findByRole('button', { name: copy.device.cpu });
    expect(chip.getAttribute('aria-haspopup')).toBe('dialog');
    await fireEvent.click(chip);

    const dialog = screen.getByRole('dialog', { name: copy.device.whyTitle });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(dialog);
    expect(screen.getByText(loadFailed.reason!)).toBeTruthy();
    expect(screen.queryByText(loadFailed.details!)).toBeNull();
    const toggle = screen.getByRole('button', { name: copy.device.showDetails });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    await fireEvent.click(toggle);
    expect(screen.getByText(loadFailed.details!)).toBeTruthy();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toggle.textContent).toBe(copy.device.hideDetails);

    expect(screen.getByRole('button', { name: copy.device.fixes.tryAgain })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: copy.device.fixes.useProcessor }));
    expect(deviceFix).toHaveBeenCalledWith('useProcessor');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await screen.findByText(copy.device.notStarted)).toBeTruthy();
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('whyClosesWithEscapeAndReturnsFocus', async () => {
    deviceStatus.mockResolvedValue(loadFailed);
    render(DeviceChip);
    const chip = await screen.findByRole('button', { name: copy.device.cpu });
    chip.focus();
    await fireEvent.click(chip);
    expect(screen.getByRole('dialog')).toBeTruthy();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(chip);
    expect(deviceFix).not.toHaveBeenCalled();
  });

  it('sharedGpuExplainsWithoutFixes', async () => {
    appStatus.mockResolvedValue(status('macos'));
    deviceStatus.mockResolvedValue(shared);
    render(DeviceChip);
    await fireEvent.click(await screen.findByRole('button', { name: copy.device.neuralEngine }));
    expect(screen.getByText(shared.reason!)).toBeTruthy();
    const dialog = screen.getByRole('dialog');
    const buttons = [...dialog.querySelectorAll('button')].map((b) => b.textContent);
    expect(buttons).toEqual([copy.device.showDetails, copy.common.close]);
  });

  it('chipLooksClickable', async () => {
    deviceStatus.mockResolvedValue(loadFailed);
    const slow = render(DeviceChip);
    const chip = await screen.findByRole('button', { name: copy.device.cpu });
    const mark = chip.querySelector('.info');
    expect(mark?.textContent).toBe(copy.device.infoGlyph);
    expect(mark?.getAttribute('aria-hidden')).toBe('true');
    slow.unmount();

    appStatus.mockResolvedValue(status('macos'));
    deviceStatus.mockResolvedValue(shared);
    const explained = render(DeviceChip);
    const fast = await screen.findByRole('button', { name: copy.device.neuralEngine });
    expect(fast.querySelector('.info')).toBeTruthy();
    explained.unmount();

    deviceStatus.mockResolvedValue(gpu);
    const plain = render(DeviceChip);
    await screen.findByText(copy.device.neuralEngine);
    expect(plain.container.querySelector('.info')).toBeNull();
  });

  it('fixInFlightKeepsTheDialogOpen', async () => {
    deviceStatus.mockResolvedValue(loadFailed);
    let fail: (e: unknown) => void = () => {};
    deviceFix.mockImplementationOnce(() => new Promise<void>((_, reject) => (fail = reject)));
    render(DeviceChip);
    await fireEvent.click(await screen.findByRole('button', { name: copy.device.cpu }));
    await fireEvent.click(screen.getByRole('button', { name: copy.device.fixes.tryAgain }));
    const close = screen.getByRole('button', { name: copy.common.close }) as HTMLButtonElement;
    expect(close.disabled).toBe(true);
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByRole('dialog')).toBeTruthy();
    fail({ message: 'The graphics card is still busy.', details: '' });
    expect(await screen.findByText('The graphics card is still busy.')).toBeTruthy();
    expect(close.disabled).toBe(false);
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('failedFixSaysSo', async () => {
    deviceStatus.mockResolvedValue(loadFailed);
    deviceFix.mockRejectedValueOnce({ message: 'The app is still working on your photos.', details: '' });
    render(DeviceChip);
    await fireEvent.click(await screen.findByRole('button', { name: copy.device.cpu }));
    await fireEvent.click(screen.getByRole('button', { name: copy.device.fixes.tryAgain }));
    expect(await screen.findByText('The app is still working on your photos.')).toBeTruthy();
    expect(screen.getByRole('dialog')).toBeTruthy();
  });
});

describe('WhyDialog', () => {
  it('fixLabelsFollowThePlatform', () => {
    const status: DeviceStatus = { state: 'cpu', reason: 'You chose the processor in Settings.', details: null, fixes: ['useGraphicsCard'] };
    const props = { status, onFix: vi.fn(), onClose: vi.fn() };
    const mac = render(WhyDialog, { ...props, platform: 'macos' });
    expect(screen.getByRole('button', { name: copy.device.fixes.useNeuralEngine })).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.device.showDetails })).toBeNull();
    mac.unmount();
    render(WhyDialog, { ...props, platform: 'windows' });
    expect(screen.getByRole('button', { name: copy.device.fixes.useGraphicsCard })).toBeTruthy();
  });

  it('noBannedWordsRendered', () => {
    const all: DeviceStatus = {
      ...loadFailed,
      fixes: ['tryAgain', 'useProcessor', 'useGraphicsCard', { downloadHelper: { bytes: 120_000_000 } }],
    };
    const { container } = render(WhyDialog, { status: all, platform: 'windows', onFix: vi.fn(), onClose: vi.fn() });
    expect(screen.getByRole('button', { name: copy.device.fixes.downloadHelper(120) })).toBeTruthy();
    assertPlainText(container);
  });
});
