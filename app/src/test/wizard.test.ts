import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import ProgressLine from '../lib/components/ProgressLine.svelte';
import { createWizard } from '../lib/wizard.svelte';
import { assertPlainText } from './plainText';

describe('wizard', () => {
  it('startsOnWelcomeWithoutStep', () => {
    const w = createWizard();
    expect(w.screen).toBe('welcome');
    expect(w.step).toBeNull();
    expect(w.canGoBack).toBe(false);
  });

  it('stepsFollowScreens', () => {
    const w = createWizard();
    const steps = (['add', 'check', 'checkAll', 'remove', 'done'] as const).map((s) => {
      w.go(s);
      return w.step;
    });
    expect(steps).toEqual([1, 2, 3, 4, 5]);
    w.go('settings');
    expect(w.step).toBeNull();
    w.go('saved');
    expect(w.step).toBeNull();
  });

  it('backKeepsState', () => {
    const w = createWizard();
    w.go('add');
    w.go('check');
    const outcome = { sample: [3, 5], representative: 5 };
    w.outcome = outcome;
    w.go('checkAll');
    w.go('settings');
    w.back();
    expect(w.screen).toBe('checkAll');
    w.back();
    expect(w.screen).toBe('check');
    expect(w.outcome).toEqual(outcome);
    w.back();
    expect(w.screen).toBe('add');
    expect(w.outcome).toEqual(outcome);
    w.back();
    expect(w.screen).toBe('welcome');
    w.back();
    expect(w.screen).toBe('welcome');
  });

  it('goingToTheSameScreenAddsNoBackStep', () => {
    const w = createWizard();
    w.go('add');
    w.go('add');
    w.back();
    expect(w.screen).toBe('welcome');
  });

  it('progressLineMarksDoneSteps', () => {
    const { container } = render(ProgressLine, { step: 3 });
    const items = [...container.querySelectorAll('li')];
    expect(items.map((li) => li.dataset.state)).toEqual(['done', 'done', 'current', 'todo', 'todo']);
    expect(items[2].getAttribute('aria-current')).toBe('step');
    expect(items.filter((li) => li.hasAttribute('aria-current'))).toHaveLength(1);
    expect(container.textContent?.replace(/\s+/g, ' ').trim()).toBe(
      '1 Add photos > 2 Check the watermark > 3 Check all photos > 4 Remove > 5 Done',
    );
    assertPlainText(container);
  });
});
