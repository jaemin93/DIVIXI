/**
 * The schedules popover's filter (SchedulePopover.svelte): which kind it
 * shows and in which order, kept out of the component so it runs under
 * `node --test`.
 *
 * "Newest" means what is nearest now on each kind's own clock:
 * - a schedule: by its next firing, the soonest first (oldest: the furthest
 *   first). One waiting on a condition has no time and comes after the
 *   timed ones either way, the first set first (oldest: the last set first).
 * - a job: the open ones (running, stalled) always above the ended ones;
 *   the open by when they were registered, the ended by when they ended,
 *   the latest first (oldest: the earliest first). Only jobs that ended in
 *   the last day are listed (`jobList.RECENT_MS`).
 */

import { RECENT_MS, isOpen, type JobRow } from "./jobList.ts";
import type { ScheduleRow } from "./scheduleList.ts";

export type WatchKind = "all" | "schedules" | "jobs";
export type WatchOrder = "newest" | "oldest";
export type WatchFilter = { kind: WatchKind; order: WatchOrder };

export const DEFAULT_WATCH_FILTER: WatchFilter = { kind: "all", order: "newest" };

/** Where this PC's window keeps the filter (a layout setting), and where a browser does. */
export const WATCH_FILTER_KEY = "watch_filter";
export const WEB_WATCH_FILTER_KEY = "divixi.watchfilter";

const KINDS: WatchKind[] = ["all", "schedules", "jobs"];
const ORDERS: WatchOrder[] = ["newest", "oldest"];

/** A kept filter read back; anything that is not one is the default, field by field. */
export function parseWatchFilter(raw: string | null | undefined): WatchFilter {
  let v: Partial<Record<keyof WatchFilter, unknown>> = {};
  try {
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object") v = parsed as typeof v;
  } catch {
    // The default.
  }
  return {
    kind: KINDS.includes(v.kind as WatchKind) ? (v.kind as WatchKind) : DEFAULT_WATCH_FILTER.kind,
    order: ORDERS.includes(v.order as WatchOrder) ? (v.order as WatchOrder) : DEFAULT_WATCH_FILTER.order,
  };
}

/** The filter narrows or reorders something: what the button's tint says. */
export function isFiltered(f: WatchFilter): boolean {
  return f.kind !== DEFAULT_WATCH_FILTER.kind || f.order !== DEFAULT_WATCH_FILTER.order;
}

/** The active schedules, in the filter's order. */
export function orderSchedules<T extends ScheduleRow>(list: T[], order: WatchOrder): T[] {
  const sign = order === "newest" ? 1 : -1;
  const active = list.filter((s) => s.status === "active");
  const timed = active.filter((s) => s.next_at != null).sort((a, b) => sign * ((a.next_at ?? 0) - (b.next_at ?? 0)) || sign * (a.id - b.id));
  const waiting = active.filter((s) => s.next_at == null).sort((a, b) => sign * (a.id - b.id));
  return [...timed, ...waiting];
}

/** The open jobs, then the ones that ended in the last day, each in the filter's order. */
export function orderJobs<T extends JobRow>(list: T[], now: number, order: WatchOrder): T[] {
  const sign = order === "newest" ? -1 : 1;
  const open = list.filter(isOpen).sort((a, b) => sign * ((a.registered_at ?? 0) - (b.registered_at ?? 0)) || sign * (a.id - b.id));
  const ended = list
    .filter((j) => !isOpen(j) && j.ended_at != null && now - j.ended_at <= RECENT_MS)
    .sort((a, b) => sign * ((a.ended_at ?? 0) - (b.ended_at ?? 0)) || sign * (a.id - b.id));
  return [...open, ...ended];
}

/** What one track's part of the popover shows: each kind the filter keeps, in its order. */
export function visible<S extends ScheduleRow, J extends JobRow>(schedules: S[], jobs: J[], now: number, f: WatchFilter): { schedules: S[]; jobs: J[] } {
  return {
    schedules: f.kind === "jobs" ? [] : orderSchedules(schedules, f.order),
    jobs: f.kind === "schedules" ? [] : orderJobs(jobs, now, f.order),
  };
}

const two = (n: number) => String(n).padStart(2, "0");

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/**
 * A time for the rows' time column: the time, and above it the day when it
 * is not today (yesterday and tomorrow in words, else `10-23`).
 */
export function clock(ms: number, now: number, tr: (key: string) => string): { day: string; time: string } {
  const d = new Date(ms);
  const n = new Date(now);
  const time = `${two(d.getHours())}:${two(d.getMinutes())}`;
  if (sameDay(d, n)) return { day: "", time };
  const shift = (days: number) => {
    const x = new Date(n);
    x.setDate(n.getDate() + days);
    return x;
  };
  if (sameDay(d, shift(1))) return { day: tr("schedules.tomorrowDay"), time };
  if (sameDay(d, shift(-1))) return { day: tr("schedules.yesterday"), time };
  return { day: `${two(d.getMonth() + 1)}-${two(d.getDate())}`, time };
}
