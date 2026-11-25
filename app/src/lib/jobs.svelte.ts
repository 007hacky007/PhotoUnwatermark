// Stopping a long job: the stop names that job's ticket, and the screens can tell while a
// stopped job is still finishing (the next job waits for it).

import { cancelJob } from './api';
import { ticketOf } from './tickets';

/** How many stopped jobs have not ended yet. */
export const stopping = $state({ count: 0 });

/**
 * Asks the job of the call `job` (and only that job) to stop. Resolves once the app has taken
 * the stop, so a job started afterwards is never stopped by it and waits for this one to end.
 */
export async function stopJob(job: Promise<unknown>): Promise<void> {
  stopping.count++;
  let ended = false;
  const end = () => {
    if (ended) return;
    ended = true;
    stopping.count--;
  };
  Promise.resolve(job).then(end, end);
  try {
    await cancelJob(ticketOf(job));
  } catch {
    // The job reports how it ended; a failed stop changes nothing here.
  }
}
