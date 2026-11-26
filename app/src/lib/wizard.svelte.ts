// Which screen the app shows, and the way back. State the screens share lives here, so going
// back never loses work.

import type { DetectOutcome, PhotoId, RemoveReport } from './types';

export type Screen =
  | 'welcome'
  | 'add'
  | 'check'
  | 'checkAll'
  | 'remove'
  | 'done'
  | 'saved'
  | 'settings'
  | 'about';

/** The wizard step of each screen (1-5), or null for screens outside the steps. */
const STEPS: Record<Screen, number | null> = {
  welcome: null,
  add: 1,
  check: 2,
  checkAll: 3,
  remove: 4,
  done: 5,
  saved: null,
  settings: null,
  about: null,
};

/** A drawing step 2 shows: the group it is for and the photos found for it in step 1. */
export interface Drawing {
  group: number;
  /** Null for a group started from a saved watermark, until its photos are known. */
  outcome: DetectOutcome | null;
}

export class Wizard {
  screen: Screen = $state('welcome');
  /** True while a long job runs; the header then hides the Settings button. */
  busy = $state(false);
  /** The number of the last group made, which is the current one (0 for none). */
  groups = $state(0);
  // Drawings are replaced, never changed in place, and are not proxied: a drawing's outcome
  // stays the very object it was made with (step 2 keeps its edits per outcome).
  /** The current group's drawing. */
  current: Drawing | null = $state.raw(null);
  /** A drawing for the next group, not confirmed yet. */
  draft: Drawing | null = $state.raw(null);
  /** The photo list group 1's drawing was found on: with the same list, step 2 opens again as it was. */
  outcomePhotos: PhotoId[] = [];
  /** How Saved watermarks opens: 'manage' to rename and delete, 'choose' to start the next group from one. */
  savedMode: 'manage' | 'choose' = $state('manage');
  /** How the last removal went, for step 5. */
  report: RemoveReport | null = $state(null);
  /** The result step 5 shows. */
  resultIndex = $state(0);
  /** The groups whose watermark step 5 saved. */
  savedGroups: number[] = $state([]);
  #history: Screen[] = $state([]);

  get step(): number | null {
    return STEPS[this.screen];
  }

  /** What step 2 shows: the draft, else the current group. */
  get drawing(): Drawing | null {
    return this.draft ?? this.current;
  }

  /** A new drawing for the next group. `photos` is the list it was found on (for group 1). */
  startDraft(outcome: DetectOutcome, photos?: PhotoId[]): void {
    this.draft = { group: this.groups + 1, outcome };
    if (photos !== undefined) this.outcomePhotos = [...photos];
  }

  /** The draft becomes the current group. Does nothing without a draft. */
  confirmDraft(): void {
    const draft = this.draft;
    if (draft === null) return;
    this.current = draft;
    this.groups = draft.group;
    this.draft = null;
  }

  /** Group `group` was started from a saved watermark and is now the current one. */
  useGroup(group: number, photos?: PhotoId[]): void {
    this.current = { group, outcome: null };
    this.groups = group;
    this.draft = null;
    if (photos !== undefined) this.outcomePhotos = [...photos];
  }

  /** No group any more. Group 1's drawing stays as the draft, so an unchanged list returns to it. */
  clearGroups(): void {
    const one = [this.draft, this.current].find((d) => d?.group === 1) ?? null;
    this.groups = 0;
    this.current = null;
    this.draft = one;
    this.savedGroups = [];
  }

  /**
   * Back from step 2: to Add photos for group 1 (its draft stays), else to step 3 (a draft of a
   * later group is dropped, a reopened group stays as it was).
   */
  backFromCheck(): void {
    const drawing = this.drawing;
    if (drawing === null || drawing.group === 1) {
      this.go('add');
      return;
    }
    this.draft = null;
    this.go('checkAll');
  }

  get canGoBack(): boolean {
    return this.#history.length > 0;
  }

  go(screen: Screen): void {
    if (screen === this.screen) return;
    this.#history.push(this.screen);
    this.screen = screen;
  }

  /** The screen before the current one; does nothing on the first screen. */
  back(): void {
    const previous = this.#history.pop();
    if (previous !== undefined) this.screen = previous;
  }

  /** A new set: Add photos, with nothing from the last set and no way back to it. */
  restart(): void {
    this.groups = 0;
    this.current = null;
    this.draft = null;
    this.outcomePhotos = [];
    this.savedMode = 'manage';
    this.savedGroups = [];
    this.report = null;
    this.resultIndex = 0;
    this.#history = [];
    this.screen = 'add';
  }
}

/** True when group 1's drawing has its photos and `photos` is the list they were found on, in order. */
export function sameSet(wizard: Wizard, photos: PhotoId[]): boolean {
  const one = [wizard.draft, wizard.current].find((d) => d?.group === 1);
  const was = wizard.outcomePhotos;
  return (
    one?.outcome != null && was.length === photos.length && was.every((id, i) => id === photos[i])
  );
}

export function createWizard(): Wizard {
  return new Wizard();
}

/** The app's wizard. */
export const wizard = createWizard();
