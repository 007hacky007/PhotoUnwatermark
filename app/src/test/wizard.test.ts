import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import ProgressLine from '../lib/components/ProgressLine.svelte';
import { createWizard, sameSet } from '../lib/wizard.svelte';
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
    w.go('about');
    expect(w.step).toBeNull();
  });

  it('sameSetNeedsTheSamePhotos', () => {
    const w = createWizard();
    expect(sameSet(w, [])).toBe(false);
    w.startDraft({ sample: [1], representative: 1 }, [1, 2]);
    expect(sameSet(w, [1, 2])).toBe(true);
    expect(sameSet(w, [1])).toBe(false);
    expect(sameSet(w, [2, 1])).toBe(false);
    expect(sameSet(w, [1, 3])).toBe(false);
    w.restart();
    expect(sameSet(w, [1, 2])).toBe(false);
  });

  it('backKeepsState', () => {
    const w = createWizard();
    w.go('add');
    w.go('check');
    const outcome = { sample: [3, 5], representative: 5 };
    w.startDraft(outcome);
    w.go('checkAll');
    w.go('settings');
    w.back();
    expect(w.screen).toBe('checkAll');
    w.back();
    expect(w.screen).toBe('check');
    expect(w.drawing?.outcome).toEqual(outcome);
    w.back();
    expect(w.screen).toBe('add');
    expect(w.drawing?.outcome).toEqual(outcome);
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

  it('draftBecomesTheCurrentGroup', () => {
    const w = createWizard();
    const one = { sample: [1, 2], representative: 1 };
    w.startDraft(one, [1, 2, 3]);
    expect(w.draft).toEqual({ group: 1, outcome: one });
    expect(w.current).toBeNull();
    expect(w.groups).toBe(0);
    expect(w.drawing).toBe(w.draft);
    w.confirmDraft();
    expect(w.current).toEqual({ group: 1, outcome: one });
    expect(w.draft).toBeNull();
    expect(w.groups).toBe(1);
    expect(w.drawing).toBe(w.current);
    const two = { sample: [3], representative: 3 };
    w.startDraft(two);
    expect(w.draft).toEqual({ group: 2, outcome: two });
    expect(w.drawing).toBe(w.draft);
    expect(w.outcomePhotos).toEqual([1, 2, 3]);
    w.confirmDraft();
    expect(w.current).toEqual({ group: 2, outcome: two });
    expect(w.groups).toBe(2);
    // Nothing to confirm: nothing changes.
    w.confirmDraft();
    expect(w.current).toEqual({ group: 2, outcome: two });
    expect(w.groups).toBe(2);
  });

  it('usingASavedWatermarkMakesTheCurrentGroup', () => {
    const w = createWizard();
    w.startDraft({ sample: [1], representative: 1 }, [1, 2]);
    w.useGroup(1, [1, 2]);
    expect(w.current).toEqual({ group: 1, outcome: null });
    expect(w.groups).toBe(1);
    expect(w.draft).toBeNull();
    expect(w.outcomePhotos).toEqual([1, 2]);
    w.startDraft({ sample: [2], representative: 2 });
    w.useGroup(2);
    expect(w.current).toEqual({ group: 2, outcome: null });
    expect(w.groups).toBe(2);
    expect(w.draft).toBeNull();
  });

  it('backFromADraftReturnsToStepThree', () => {
    const w = createWizard();
    w.go('add');
    w.startDraft({ sample: [1], representative: 1 }, [1, 2]);
    w.go('check');
    w.confirmDraft();
    w.go('checkAll');
    w.startDraft({ sample: [2], representative: 2 });
    w.go('check');
    w.backFromCheck();
    expect(w.screen).toBe('checkAll');
    expect(w.draft).toBeNull();
    expect(w.current?.group).toBe(1);
    expect(w.groups).toBe(1);
  });

  it('backFromGroupOneGoesToAddPhotos', () => {
    const w = createWizard();
    w.go('add');
    const one = { sample: [1], representative: 1 };
    w.startDraft(one, [1, 2]);
    w.go('check');
    w.backFromCheck();
    expect(w.screen).toBe('add');
    expect(w.draft).toEqual({ group: 1, outcome: one });
    // A reopened group 1 goes there too.
    w.go('check');
    w.confirmDraft();
    w.go('checkAll');
    w.go('check');
    w.backFromCheck();
    expect(w.screen).toBe('add');
    expect(w.current).toEqual({ group: 1, outcome: one });
  });

  it('backFromAReopenedGroupReturnsToStepThree', () => {
    const w = createWizard();
    w.startDraft({ sample: [1], representative: 1 }, [1, 2]);
    w.confirmDraft();
    const two = { sample: [2], representative: 2 };
    w.startDraft(two);
    w.confirmDraft();
    w.go('checkAll');
    w.go('check');
    w.backFromCheck();
    expect(w.screen).toBe('checkAll');
    expect(w.current).toEqual({ group: 2, outcome: two });
    expect(w.groups).toBe(2);
  });

  it('clearGroupsKeepsGroupOnesDrawing', () => {
    const w = createWizard();
    const one = { sample: [1, 2], representative: 1 };
    w.startDraft(one, [1, 2]);
    w.confirmDraft();
    w.savedGroups = [1];
    w.clearGroups();
    expect(w.groups).toBe(0);
    expect(w.current).toBeNull();
    expect(w.savedGroups).toEqual([]);
    expect(w.draft?.group).toBe(1);
    expect(w.draft?.outcome).toBe(one);
    expect(sameSet(w, [1, 2])).toBe(true);
    // A group-1 draft stays as it is.
    w.clearGroups();
    expect(w.draft?.outcome).toBe(one);
  });

  it('sameSetFollowsGroupOne', () => {
    const w = createWizard();
    const one = { sample: [1], representative: 1 };
    w.startDraft(one, [1, 2]);
    expect(sameSet(w, [1, 2])).toBe(true);
    w.confirmDraft();
    expect(sameSet(w, [1, 2])).toBe(true);
    // While group 2 is a draft, group 1 is still the current group.
    w.startDraft({ sample: [2], representative: 2 });
    expect(sameSet(w, [1, 2])).toBe(true);
    w.confirmDraft();
    expect(sameSet(w, [1, 2])).toBe(false);
    // A group from a saved watermark has no drawing to go back to.
    const saved = createWizard();
    saved.useGroup(1, [1, 2]);
    expect(sameSet(saved, [1, 2])).toBe(false);
  });

  it('restartClearsGroups', () => {
    const w = createWizard();
    w.startDraft({ sample: [1], representative: 1 }, [1, 2]);
    w.confirmDraft();
    w.startDraft({ sample: [2], representative: 2 });
    w.savedMode = 'choose';
    w.savedGroups = [1];
    w.restart();
    expect(w.groups).toBe(0);
    expect(w.current).toBeNull();
    expect(w.draft).toBeNull();
    expect(w.drawing).toBeNull();
    expect(w.outcomePhotos).toEqual([]);
    expect(w.savedMode).toBe('manage');
    expect(w.savedGroups).toEqual([]);
    expect(w.screen).toBe('add');
    expect(w.canGoBack).toBe(false);
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
