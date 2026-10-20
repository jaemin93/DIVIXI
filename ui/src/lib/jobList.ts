/**
 * What the schedules popover (SchedulePopover.svelte) says about each
 * background job a worker or the conductor registered (`watch_job`), kept
 * out of the component so it runs under `node --test`.
 *
 * The core sends a job as `jobs::view` builds it (src-tauri/src/jobs.rs):
 * who registered it, what it watches (a pid, a log, a pattern its finishing
 * line matches), where it stands, when it was registered, last moved and
 * ended, and its log's last line.
 */

/** A job as `list_jobs` returns it. */
export type JobRow = {
  id: number;
  track: string;
  status: string;
  line: string;
  worker?: string;
  what?: string;
  pid?: number | null;
  log?: string;
  done_pattern?: string;
  registered_at?: number;
  changed_at?: number;
  ended_at?: number | null;
  last_line?: string;
  note?: string;
};

/** How long an ended job stays in the list. */
export const RECENT_MS = 24 * 60 * 60 * 1000;

/** Still watched: its process is alive (moving or not). */
export function isOpen(j: JobRow): boolean {
  return j.status === "running" || j.status === "stalled";
}

export type JobState = "running" | "stalled" | "done" | "died" | "unwatched" | "lost";

/**
 * Where a job stands. The core keeps one `lost` for two things: somebody
 * stopped watching it (its note says so, `jobs::unwatch`) and the app could
 * no longer tell; a person reads those differently.
 */
export function jobState(j: JobRow): JobState {
  switch (j.status) {
    case "running":
    case "stalled":
    case "done":
    case "died":
      return j.status;
    default:
      return /no longer watched|지켜보기를 그만둠/.test(j.note ?? "") ? "unwatched" : "lost";
  }
}

/** The open ones, newest first; then the ones that ended in the last day, latest first. */
export function forPopover<T extends JobRow>(list: T[], now: number): T[] {
  const open = list.filter(isOpen).sort((a, b) => (b.registered_at ?? 0) - (a.registered_at ?? 0) || b.id - a.id);
  const ended = list.filter((j) => !isOpen(j) && j.ended_at != null && now - j.ended_at <= RECENT_MS).sort((a, b) => (b.ended_at ?? 0) - (a.ended_at ?? 0) || b.id - a.id);
  return [...open, ...ended];
}

/** The last part of a path. */
function base(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** What it watches, in one line: its own words, its pid, its log's file name. */
export function watches(j: JobRow): string {
  return [j.what?.trim(), j.pid ? `pid ${j.pid}` : "", j.log ? base(j.log) : ""].filter(Boolean).join(" · ");
}

/** The time a row shows: since when for an open job, when it ended for one that is over. */
export function when(j: JobRow): number | null {
  return isOpen(j) ? (j.registered_at ?? null) : (j.ended_at ?? j.changed_at ?? null);
}

/**
 * When its log last moved, for a running job that has a log and has moved
 * since it was registered: the closest the record has to "last seen
 * working". The app looks every half minute but keeps no time of the look.
 * Not for a stalled one: `changed_at` is then when it was found stalled.
 */
export function lastMoved(j: JobRow): number | null {
  if (j.status !== "running" || !j.log || j.changed_at == null || j.changed_at === j.registered_at) return null;
  return j.changed_at;
}
