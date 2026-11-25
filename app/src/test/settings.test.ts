import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppStatus, SavedInfo, Settings as SettingsData } from '../lib/types';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(),
  getSettings: vi.fn(),
  setSettings: vi.fn(),
  chooseFolder: vi.fn(),
  removeDownloadedData: vi.fn(),
  deviceStatus: vi.fn(),
  deviceFix: vi.fn(),
  listSaved: vi.fn(),
  renameSaved: vi.fn(),
  deleteSaved: vi.fn(),
  imageUrl: vi.fn((route: string, id: string) => `puw://${route}/${id}`),
  openReleasesPage: vi.fn(),
  RELEASES_URL: 'https://github.com/007hacky007/PhotoUnwatermark/releases',
}));

import * as api from '../lib/api';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import About from '../screens/About.svelte';
import SavedWatermarks from '../screens/SavedWatermarks.svelte';
import Settings from '../screens/Settings.svelte';
import { assertPlainText } from './plainText';

const appStatus = vi.mocked(api.appStatus);
const getSettings = vi.mocked(api.getSettings);
const setSettings = vi.mocked(api.setSettings);
const chooseFolder = vi.mocked(api.chooseFolder);
const removeDownloadedData = vi.mocked(api.removeDownloadedData);
const deviceStatus = vi.mocked(api.deviceStatus);
const listSaved = vi.mocked(api.listSaved);
const renameSaved = vi.mocked(api.renameSaved);
const deleteSaved = vi.mocked(api.deleteSaved);
const openReleasesPage = vi.mocked(api.openReleasesPage);

const status = (platform: string): AppStatus => ({ modelsReady: true, missingBytes: 0, platform, version: '1.2.3' });
const defaults: SettingsData = {
  output: { nextToOriginals: { folderName: 'Without watermark' } },
  detail: 'best',
  preferGpu: true,
  loadSeconds: {},
};

async function openSettings(platform = 'windows', settings: SettingsData = defaults) {
  appStatus.mockResolvedValue(status(platform));
  getSettings.mockResolvedValue(settings);
  const wizard = createWizard();
  wizard.go('add');
  wizard.go('settings');
  const view = render(Settings, { wizard });
  await screen.findByRole('radio', { name: copy.settings.best });
  return { wizard, ...view };
}

describe('Settings', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    deviceStatus.mockResolvedValue({ state: 'notStarted', reason: null, details: null, fixes: [] });
    setSettings.mockResolvedValue();
  });

  it('showsTheSavedChoices', async () => {
    await openSettings();
    expect((screen.getByRole('radio', { name: copy.settings.nextToOriginals }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole('textbox', { name: copy.settings.folderName }) as HTMLInputElement).value).toBe(
      'Without watermark',
    );
    expect((screen.getByRole('radio', { name: copy.settings.best }) as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText(copy.settings.bestHelp)).toBeTruthy();
    expect(screen.getByText(copy.settings.balancedHelp)).toBeTruthy();
    expect(screen.getByText(copy.settings.fasterHelp)).toBeTruthy();
    expect(await screen.findByText(copy.device.notStarted)).toBeTruthy();
  });

  it('detailChoiceIsSaved', async () => {
    await openSettings();
    await fireEvent.click(screen.getByRole('radio', { name: copy.settings.faster }));
    await waitFor(() => expect(setSettings).toHaveBeenCalledWith({ ...defaults, detail: 'faster' }));
  });

  it('folderNameIsSavedButNotEmpty', async () => {
    await openSettings();
    const field = screen.getByRole('textbox', { name: copy.settings.folderName }) as HTMLInputElement;
    await fireEvent.input(field, { target: { value: '  Clean  ' } });
    await fireEvent.change(field);
    await waitFor(() =>
      expect(setSettings).toHaveBeenCalledWith({ ...defaults, output: { nextToOriginals: { folderName: 'Clean' } } }),
    );
    await fireEvent.input(field, { target: { value: '   ' } });
    await fireEvent.change(field);
    expect(setSettings).toHaveBeenCalledTimes(1);
    expect(field.value).toBe('Clean');
    expect(screen.getByText(copy.settings.badFolderName)).toBeTruthy();
  });

  it('badFolderNamesAreRefusedWithAReason', async () => {
    const { validFolderName } = await import('../lib/folderName');
    for (const ok of ['Without watermark', 'Out', 'a.b', '.hidden']) expect(validFolderName(ok), ok).toBe(true);
    for (const bad of ['', '.', '..', 'a/b', 'a\\b', 'x:', 'a*', 'q?', '<x>', '"q"', 'p|q', 'end.', 'end ', 'CON', 'con.txt', 'COM1', 'lpt9.x', 'tab\there']) {
      expect(validFolderName(bad), JSON.stringify(bad)).toBe(false);
    }
    await openSettings();
    const field = screen.getByRole('textbox', { name: copy.settings.folderName }) as HTMLInputElement;
    for (const bad of ['..', 'a/b', 'Out:1']) {
      await fireEvent.input(field, { target: { value: bad } });
      await fireEvent.change(field);
      expect(field.value).toBe('Without watermark');
      expect(screen.getByRole('alert').textContent).toBe(copy.settings.badFolderName);
    }
    expect(setSettings).not.toHaveBeenCalled();
  });

  it('alwaysSaveToAsksForAFolder', async () => {
    await openSettings();
    chooseFolder.mockResolvedValueOnce(null);
    const folder = screen.getByRole('radio', { name: copy.settings.alwaysFolder }) as HTMLInputElement;
    await fireEvent.click(folder);
    await waitFor(() => expect(chooseFolder).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(folder.checked).toBe(false));
    expect((screen.getByRole('radio', { name: copy.settings.nextToOriginals }) as HTMLInputElement).checked).toBe(true);
    expect(setSettings).not.toHaveBeenCalled();

    chooseFolder.mockResolvedValueOnce('/Users/me/Clean');
    await fireEvent.click(folder);
    await waitFor(() => expect(setSettings).toHaveBeenCalledWith({ ...defaults, output: { folder: { path: '/Users/me/Clean' } } }));
    expect(await screen.findByText('/Users/me/Clean')).toBeTruthy();
    expect(folder.checked).toBe(true);
  });

  it('processorChoiceOnlyOnWindows', async () => {
    const win = await openSettings('windows');
    await fireEvent.click(screen.getByRole('radio', { name: copy.settings.useProcessor }));
    await waitFor(() => expect(setSettings).toHaveBeenCalledWith({ ...defaults, preferGpu: false }));
    await waitFor(() => expect(deviceStatus).toHaveBeenCalledTimes(2));
    win.unmount();

    await openSettings('macos');
    expect(screen.queryByRole('radio', { name: copy.settings.useProcessor })).toBeNull();
    expect(await screen.findByText(copy.device.notStarted)).toBeTruthy();
  });

  it('failedSaveRestoresTheChoice', async () => {
    await openSettings('windows');
    const radio = (name: string) => screen.getByRole('radio', { name }) as HTMLInputElement;
    setSettings.mockRejectedValueOnce({ message: 'The settings file is locked.', details: '' });
    await fireEvent.click(radio(copy.settings.faster));
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByText('The settings file is locked.')).toBeTruthy();
    await waitFor(() => expect(radio(copy.settings.best).checked).toBe(true));
    expect(radio(copy.settings.faster).checked).toBe(false);

    setSettings.mockRejectedValueOnce({ message: 'The settings file is locked.', details: '' });
    await fireEvent.click(radio(copy.settings.useProcessor));
    await waitFor(() => expect(radio(copy.settings.useGraphicsCard).checked).toBe(true));
    expect(radio(copy.settings.useProcessor).checked).toBe(false);
  });

  it('quickChangesDoNotUndoEachOther', async () => {
    await openSettings('windows');
    const settle: (() => void)[] = [];
    setSettings.mockImplementation(() => new Promise<void>((resolve) => settle.push(resolve)));
    await fireEvent.click(screen.getByRole('radio', { name: copy.settings.faster }));
    await fireEvent.click(screen.getByRole('radio', { name: copy.settings.useProcessor }));
    await waitFor(() => expect(setSettings).toHaveBeenCalledTimes(1));
    expect(setSettings).toHaveBeenLastCalledWith({ ...defaults, detail: 'faster' });
    // While the first save runs, both clicked choices stay shown.
    expect((screen.getByRole('radio', { name: copy.settings.faster }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole('radio', { name: copy.settings.useProcessor }) as HTMLInputElement).checked).toBe(true);
    settle[0]();
    await waitFor(() => expect(setSettings).toHaveBeenCalledTimes(2));
    expect(setSettings).toHaveBeenLastCalledWith({ ...defaults, detail: 'faster', preferGpu: false });
    settle[1]();
    await waitFor(() => expect(deviceStatus).toHaveBeenCalledTimes(2));
    expect((screen.getByRole('radio', { name: copy.settings.faster }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole('radio', { name: copy.settings.useProcessor }) as HTMLInputElement).checked).toBe(true);
  });

  it('macShowsTheProcessorOnlyWhenChosen', async () => {
    const chosen = await openSettings('macos', { ...defaults, preferGpu: false });
    expect(screen.getByText(copy.settings.macProcessorChosen)).toBeTruthy();
    expect(screen.queryByRole('radio', { name: copy.settings.useProcessor })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: copy.settings.useNeuralEngine }));
    await waitFor(() => expect(setSettings).toHaveBeenCalledWith({ ...defaults, preferGpu: true }));
    await waitFor(() => expect(screen.queryByText(copy.settings.macProcessorChosen)).toBeNull());
    expect(screen.queryByRole('button', { name: copy.settings.useNeuralEngine })).toBeNull();
    chosen.unmount();

    const usual = await openSettings('macos');
    expect(screen.queryByText(copy.settings.macProcessorChosen)).toBeNull();
    expect(screen.queryByRole('button', { name: copy.settings.useNeuralEngine })).toBeNull();
    usual.unmount();

    await openSettings('windows', { ...defaults, preferGpu: false });
    expect(screen.queryByText(copy.settings.macProcessorChosen)).toBeNull();
    expect((screen.getByRole('radio', { name: copy.settings.useProcessor }) as HTMLInputElement).checked).toBe(true);
  });

  it('backToOriginalsUsesTheUsualFolderName', async () => {
    await openSettings('windows', { ...defaults, output: { folder: { path: '/Users/me/Clean' } } });
    await fireEvent.click(screen.getByRole('radio', { name: copy.settings.nextToOriginals }));
    await waitFor(() =>
      expect(setSettings).toHaveBeenCalledWith({
        ...defaults,
        output: { nextToOriginals: { folderName: copy.settings.defaultFolderName } },
      }),
    );
    expect(copy.settings.defaultFolderName).toBe('Without watermark');
  });

  it('removeDataNeedsConfirmation', async () => {
    const { wizard } = await openSettings();
    await fireEvent.click(screen.getByText(copy.settings.advanced));
    await fireEvent.click(screen.getByRole('button', { name: copy.settings.removeData }));
    expect(removeDownloadedData).not.toHaveBeenCalled();
    let dialog = screen.getByRole('dialog', { name: copy.settings.removeDataTitle });
    for (const item of copy.settings.removeDataItems) expect(within(dialog).getByText(item)).toBeTruthy();
    expect(within(dialog).getByText(copy.settings.removeDataKeeps)).toBeTruthy();
    assertPlainText(dialog);

    await fireEvent.click(within(dialog).getByRole('button', { name: copy.common.cancel }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(removeDownloadedData).not.toHaveBeenCalled();

    // Refused while a job runs: the reason is shown and nothing changes.
    removeDownloadedData.mockRejectedValueOnce({
      message: 'The app is still working on your photos. Please wait until it finishes.',
      details: 'busy',
    });
    await fireEvent.click(screen.getByRole('button', { name: copy.settings.removeData }));
    dialog = screen.getByRole('dialog', { name: copy.settings.removeDataTitle });
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.settings.removeDataConfirm }));
    expect(await within(dialog).findByText(/still working on your photos/)).toBeTruthy();
    expect(wizard.screen).toBe('settings');

    removeDownloadedData.mockResolvedValueOnce();
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.settings.removeDataConfirm }));
    await waitFor(() => expect(wizard.screen).toBe('welcome'));
    expect(removeDownloadedData).toHaveBeenCalledTimes(2);
  });

  it('linksToSavedAndAbout', async () => {
    const { wizard } = await openSettings();
    await fireEvent.click(screen.getByRole('button', { name: copy.settings.savedWatermarks }));
    expect(wizard.screen).toBe('saved');
    wizard.back();
    await fireEvent.click(screen.getByRole('button', { name: copy.settings.about }));
    expect(wizard.screen).toBe('about');
    wizard.back();
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('add');
  });

  it('noBannedWordsRendered', async () => {
    const { container } = await openSettings('windows', {
      ...defaults,
      output: { folder: { path: '/Users/me/Clean' } },
    });
    await fireEvent.click(screen.getByText(copy.settings.advanced));
    assertPlainText(container);
  });
});

const saved = (id: string, name: string): SavedInfo => ({ id, name, created: '2026-11-20T10:00:00Z' });

async function openSaved(list: SavedInfo[]) {
  listSaved.mockResolvedValue(list);
  const wizard = createWizard();
  wizard.go('settings');
  wizard.go('saved');
  const view = render(SavedWatermarks, { wizard });
  await waitFor(() => expect(listSaved).toHaveBeenCalled());
  return { wizard, ...view };
}

describe('Saved watermarks', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.imageUrl).mockImplementation((route, id) => `puw://${route}/${id}`);
  });

  it('listsThumbnailNameAndDate', async () => {
    await openSaved([saved('a1', 'Studio'), saved('b2', 'Wedding')]);
    const img = (await screen.findByAltText('Studio')) as HTMLImageElement;
    expect(img.getAttribute('src')).toBe('puw://saved/a1');
    expect(screen.getByText('Wedding')).toBeTruthy();
    const date = new Date('2026-11-20T10:00:00Z').toLocaleDateString(undefined, { dateStyle: 'medium' } as Intl.DateTimeFormatOptions);
    expect(screen.getAllByText(copy.saved.savedOn(date))).toHaveLength(2);
  });

  it('emptyListSaysHowToSave', async () => {
    await openSaved([]);
    expect(await screen.findByText(copy.saved.empty)).toBeTruthy();
  });

  it('renameSaved', async () => {
    await openSaved([saved('a1', 'Studio')]);
    await fireEvent.click(await screen.findByRole('button', { name: copy.saved.renameOne('Studio') }));
    const field = screen.getByRole('textbox', { name: copy.saved.newName('Studio') }) as HTMLInputElement;
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe('Studio');

    // Escape leaves the name as it was.
    await fireEvent.keyDown(field, { key: 'Escape' });
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(renameSaved).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole('button', { name: copy.saved.renameOne('Studio') }));
    const again = screen.getByRole('textbox', { name: copy.saved.newName('Studio') });
    renameSaved.mockResolvedValueOnce();
    listSaved.mockResolvedValue([saved('a1', 'Home studio')]);
    await fireEvent.input(again, { target: { value: ' Home studio ' } });
    await fireEvent.keyDown(again, { key: 'Enter' });
    expect(renameSaved).toHaveBeenCalledWith('a1', 'Home studio');
    expect(await screen.findByText('Home studio')).toBeTruthy();
    expect(screen.queryByRole('textbox')).toBeNull();
  });

  it('renameFailureIsShown', async () => {
    await openSaved([saved('a1', 'Studio')]);
    await fireEvent.click(await screen.findByRole('button', { name: copy.saved.renameOne('Studio') }));
    renameSaved.mockRejectedValueOnce({ message: 'That name cannot be used. Choose a different name and try again.', details: '' });
    await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'Studio/2' } });
    await fireEvent.click(screen.getByRole('button', { name: copy.saved.save }));
    expect(renameSaved).toHaveBeenCalledWith('a1', 'Studio/2');
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByText(/That name cannot be used/)).toBeTruthy();
    expect(screen.getByRole('textbox')).toBeTruthy();
  });

  it('emptyRenameSaysSo', async () => {
    await openSaved([saved('a1', 'Studio')]);
    await fireEvent.click(await screen.findByRole('button', { name: copy.saved.renameOne('Studio') }));
    await fireEvent.input(screen.getByRole('textbox'), { target: { value: '   ' } });
    await fireEvent.click(screen.getByRole('button', { name: copy.saved.save }));
    expect(screen.getByRole('alert').textContent).toBe(copy.saved.emptyName);
    expect(screen.getByRole('textbox')).toBeTruthy();
    expect(renameSaved).not.toHaveBeenCalled();
  });

  it('deleteSavedNeedsConfirmation', async () => {
    await openSaved([saved('a1', 'Studio'), saved('b2', 'Wedding')]);
    await fireEvent.click(await screen.findByRole('button', { name: copy.saved.deleteOne('Studio') }));
    expect(deleteSaved).not.toHaveBeenCalled();
    let dialog = screen.getByRole('dialog', { name: copy.saved.deleteTitle('Studio') });
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.common.cancel }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(deleteSaved).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole('button', { name: copy.saved.deleteOne('Studio') }));
    dialog = screen.getByRole('dialog', { name: copy.saved.deleteTitle('Studio') });
    deleteSaved.mockResolvedValueOnce();
    listSaved.mockResolvedValue([saved('b2', 'Wedding')]);
    await fireEvent.click(within(dialog).getByRole('button', { name: copy.saved.delete }));
    expect(deleteSaved).toHaveBeenCalledWith('a1');
    await waitFor(() => expect(screen.queryByText('Studio')).toBeNull());
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByText('Wedding')).toBeTruthy();
  });

  it('backReturns', async () => {
    const { wizard } = await openSaved([]);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('settings');
  });

  it('noBannedWordsRendered', async () => {
    const { container } = await openSaved([saved('a1', 'Studio')]);
    await screen.findByAltText('Studio');
    await fireEvent.click(screen.getByRole('button', { name: copy.saved.renameOne('Studio') }));
    assertPlainText(container);
  });
});

describe('About', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    appStatus.mockResolvedValue(status('macos'));
  });

  it('showsVersionCreditsAndNote', async () => {
    const wizard = createWizard();
    wizard.go('about');
    const { container } = render(About, { wizard });
    expect(await screen.findByText(copy.about.version('1.2.3'))).toBeTruthy();
    for (const line of [copy.about.responsibleUse, copy.about.finder, copy.about.remover, copy.about.removerWeights, copy.about.faces, copy.about.app]) {
      expect(screen.getByText(line)).toBeTruthy();
    }
    assertPlainText(container);
  });

  it('releasesLinkOpensInTheBrowser', async () => {
    const wizard = createWizard();
    wizard.go('about');
    render(About, { wizard });
    const link = screen.getByRole('link', { name: copy.about.releases });
    expect(link.getAttribute('href')).toBe('https://github.com/007hacky007/PhotoUnwatermark/releases');
    openReleasesPage.mockResolvedValueOnce();
    await fireEvent.click(link);
    expect(openReleasesPage).toHaveBeenCalledTimes(1);
  });
});
