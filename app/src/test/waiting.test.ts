import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Waiting from '../lib/components/Waiting.svelte';
import { copy } from '../lib/copy';
import { assertPlainText } from './plainText';

describe('Waiting', () => {
  it('showsSentenceAndFraction', () => {
    const { container } = render(Waiting, {
      sentence: copy.loading.remover,
      fraction: 0.25,
      detail: 'Photo 3 of 12',
    });
    expect(screen.getByText(copy.loading.remover)).toBeTruthy();
    expect(screen.getByText('Photo 3 of 12')).toBeTruthy();
    const bar = screen.getByRole('progressbar');
    expect(bar.getAttribute('aria-valuenow')).toBe('25');
    expect(screen.queryByRole('button')).toBeNull();
    assertPlainText(container);
  });

  it('nullFractionIsIndeterminate', () => {
    render(Waiting, { sentence: copy.loading.finder, fraction: null });
    const bar = screen.getByRole('progressbar');
    expect(bar.hasAttribute('aria-valuenow')).toBe(false);
    expect(bar.dataset.indeterminate).toBe('true');
  });

  it('cancelCallsBack', async () => {
    const onCancel = vi.fn();
    const { container } = render(Waiting, { sentence: copy.loading.finder, fraction: 0.5, onCancel });
    await fireEvent.click(screen.getByRole('button', { name: copy.common.cancel }));
    expect(onCancel).toHaveBeenCalledOnce();
    assertPlainText(container);
  });
});
