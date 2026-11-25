import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../lib/api', () => ({
  appStatus: vi.fn(() => new Promise(() => {})),
  downloadModels: vi.fn(),
  cancelJob: vi.fn(),
  listPhotos: vi.fn(async () => []),
  suggestSaved: vi.fn(async () => null),
  onDragDrop: vi.fn(async () => () => {}),
  imageUrl: vi.fn(),
}));

import App from '../App.svelte';
import { copy } from '../lib/copy';
import { createWizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

describe('App', () => {
  it('startsOnWelcomeWithoutProgressLine', () => {
    const { container } = render(App, { wizard: createWizard() });
    expect(screen.getByText(copy.appName)).toBeTruthy();
    expect(container.querySelector('nav')).toBeNull();
    assertPlainText(container);
  });

  it('settingsOpensAndBackReturns', async () => {
    const wizard = createWizard();
    wizard.go('add');
    const { container } = render(App, { wizard });
    expect(container.querySelector('li[aria-current="step"]')?.textContent).toContain(copy.steps[0]);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.settings }));
    expect(screen.getByRole('heading', { name: copy.settings.title })).toBeTruthy();
    expect(screen.queryByRole('button', { name: copy.common.settings })).toBeNull();
    assertPlainText(container);
    await fireEvent.click(screen.getByRole('button', { name: copy.common.back }));
    expect(wizard.screen).toBe('add');
  });

  it('settingsHiddenWhileBusy', async () => {
    const wizard = createWizard();
    render(App, { wizard });
    expect(screen.getByRole('button', { name: copy.common.settings })).toBeTruthy();
    wizard.busy = true;
    await tick();
    expect(screen.queryByRole('button', { name: copy.common.settings })).toBeNull();
  });
});
