// Which ticket each long job's call runs under. Kept apart from api.ts so screen tests that
// replace api.ts still use the real bookkeeping.

const tickets = new WeakMap<Promise<unknown>, number>();

/** Notes that the call `job` runs under `ticket`, and returns it. */
export function tag<T>(job: Promise<T>, ticket: number): Promise<T> {
  tickets.set(job, ticket);
  return job;
}

/** The ticket the call `job` runs under (`undefined` for a call that has none). */
export function ticketOf(job: Promise<unknown>): number | undefined {
  return tickets.get(job);
}

/**
 * The first ticket of this page: different on every load of the page (milliseconds since
 * 1970, times 1000), so a ticket of an earlier load that the app still remembers as stopped
 * never matches a new job. Safe as a number until the year 2255.
 */
export function firstTicket(now: number = Date.now()): number {
  return Math.floor(now) * 1000;
}
