// Starting a group from a saved watermark: the one flow behind "Yes" on the offer (Add photos
// and step 3) and "Use for these photos" on the Saved watermarks screen.

import { useSaved } from './api';
import type { PhotoId } from './types';
import type { Wizard } from './wizard.svelte';

/**
 * Makes the next group from saved watermark `id` and makes it the wizard's current group.
 * `photos` is the photo list, kept only when this is group 1 (so an unchanged list later opens
 * step 2 as it was). Resolves with the group's number; nothing changes when it rejects. The
 * caller goes on to step 3, where the group is matched.
 */
export async function startFromSaved(wizard: Wizard, id: string, photos?: PhotoId[]): Promise<number> {
  const group = await useSaved(id);
  wizard.useGroup(group, group === 1 ? photos : undefined);
  return group;
}
