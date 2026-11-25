// Which screen the app shows, and the way back. State the screens share lives here, so going
// back never loses work.

import type { DetectOutcome, RemoveReport } from './types';

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

export class Wizard {
  screen: Screen = $state('welcome');
  /** True while a long job runs; the header then hides the Settings button. */
  busy = $state(false);
  /** The photos found in step 1, for the step-2 editor. */
  outcome: DetectOutcome | null = $state(null);
  /** How the last removal went, for step 5. */
  report: RemoveReport | null = $state(null);
  #history: Screen[] = $state([]);

  get step(): number | null {
    return STEPS[this.screen];
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
    this.outcome = null;
    this.report = null;
    this.#history = [];
    this.screen = 'add';
  }
}

export function createWizard(): Wizard {
  return new Wizard();
}

/** The app's wizard. */
export const wizard = createWizard();
