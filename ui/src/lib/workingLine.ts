/**
 * The workers of a track that are in a turn now, for the lines at the foot
 * of the conversation (Timeline.svelte). Once the conductor has delegated
 * and ended its turn, the conversation used to say nothing until the
 * report came, however long the worker took (#37). Kept out of the
 * component so it runs under `node --test`.
 *
 * "In a turn" is read the way the session list reads it (TrackList.svelte):
 * a run of the worker's that is live, or its session saying it is running.
 * The words come in through `tr`, the i18n `t`, so no language is written
 * here.
 */

export type Tr = (key: string, vars?: Record<string, string | number>) => string;

/** What is read of a run: whose it is, how it stands, when it began. */
export type RunLike = { track: string; session: string; status: string; startedAt: number };

/** What is read of a worker's live session (`WorkerState`). */
export type SessionLike = { name: string; running: boolean };

/** A worker in a turn, and since when (null when only its session says so). */
export type Working = { name: string; since: number | null };

/** As many workers as get a line of their own; the rest share one. */
export const WORKING_LINES = 3;

const live = (status: string) => status === "running" || status === "connecting";

/**
 * The track's workers in a turn now, the longest-running first. `skip` names
 * the sessions that are not workers (the conductor, an artifact's agent).
 */
export function workingWorkers(runs: RunLike[], sessions: SessionLike[] | undefined, track: string, skip: (session: string) => boolean): Working[] {
  const since = new Map<string, number | null>();
  for (const r of runs) {
    if (r.track !== track || skip(r.session) || !live(r.status)) continue;
    // The turn in flight is the newest live one. Two live at once is a page
    // that missed an older one's end (staleRuns.ts), not two turns.
    const had = since.get(r.session);
    since.set(r.session, had == null ? r.startedAt : Math.max(had, r.startedAt));
  }
  for (const s of sessions ?? []) {
    if (s.running && !skip(s.name) && !since.has(s.name)) since.set(s.name, null);
  }
  return [...since.entries()]
    .map(([name, at]) => ({ name, since: at }))
    .sort((a, b) => (a.since ?? Infinity) - (b.since ?? Infinity) || a.name.localeCompare(b.name));
}

/**
 * How long a worker has been at it, in whole minutes: "just now" under one,
 * then minutes, then hours and minutes. `now` is the page's clock, which
 * moves every 30 s and can be a little behind a turn that has just begun.
 */
export function workingFor(since: number | null, now: number, tr: Tr): string {
  if (since == null) return tr("timeline.workingNow");
  const minutes = Math.floor(Math.max(0, now - since) / 60_000);
  if (minutes < 1) return tr("timeline.workingJust");
  if (minutes < 60) return tr("timeline.workingFor", { for: tr("schedules.minutes", { n: minutes }) });
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  const span = m ? `${tr("schedules.hours", { n: h })} ${tr("schedules.minutes", { n: m })}` : tr("schedules.hours", { n: h });
  return tr("timeline.workingFor", { for: span });
}
