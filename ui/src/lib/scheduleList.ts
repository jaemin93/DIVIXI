/**
 * What the schedules popover (SchedulePopover.svelte) says about each
 * schedule, kept out of the component so it runs under `node --test`.
 *
 * The core sends a schedule as `schedule::view` builds it
 * (src-tauri/src/schedule.rs): its kind, its spec as the record keeps it,
 * the message, the next firing and the note the last firing left. The
 * words come in through `tr`, the i18n `t`, so no language is written here.
 */

export type Tr = (key: string, vars?: Record<string, string | number>) => string;

/** A schedule as `list_schedules` returns it. */
export type ScheduleRow = {
  id: number;
  track: string;
  status: string;
  line: string;
  kind?: string;
  spec?: Record<string, unknown> | null;
  message?: string;
  next_at?: number | null;
  last_at?: number | null;
  note?: string | null;
};

/** Past this much after its time a firing is late: the core's `LATE`. */
export const LATE_MS = 2 * 60 * 1000;

const two = (n: number) => String(n).padStart(2, "0");

/** `10-21 08:31`, in local time: how the core writes a time (`crate::when`). */
export function stamp(ms: number): string {
  const d = new Date(ms);
  return `${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

/** A span in seconds the way the core reads one back (`2h`, `30m`, `1d`), in the interface's words. */
export function span(secs: number, tr: Tr): string {
  if (secs > 0 && secs % 86_400 === 0) return tr("schedules.days", { n: secs / 86_400 });
  if (secs > 0 && secs % 3600 === 0) return tr("schedules.hours", { n: secs / 3600 });
  if (secs > 0 && secs % 60 === 0) return tr("schedules.minutes", { n: secs / 60 });
  return tr("schedules.seconds", { n: secs });
}

const DAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/** `sun` as the interface's short day name; 2026-01-04 was a Sunday. */
function dayName(day: string, locale: string): string {
  const i = DAYS.indexOf(day.slice(0, 3).toLowerCase());
  if (i < 0) return day;
  return new Intl.DateTimeFormat(locale, { weekday: "short" }).format(new Date(2026, 0, 4 + i));
}

/** The last part of a path, for a label; the whole path goes in the tooltip. */
function base(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

const str = (v: unknown) => (typeof v === "string" ? v : "");
const num = (v: unknown) => (typeof v === "number" ? v : null);

/**
 * What a schedule does besides firing once at a time: its repetition
 * (`daily 09:15`, `every 2h until 10-21 18:00`) or its condition (a file
 * saying something, a process ending). Null for a one-off time.
 */
export function rule(s: ScheduleRow, tr: Tr, locale: string): string | null {
  const spec = s.spec ?? {};
  switch (s.kind) {
    case "every": {
      const every = span(num(spec.every) ?? 0, tr);
      const until = num(spec.until);
      return until ? tr("schedules.everyUntil", { span: every, until: stamp(until) }) : tr("schedules.every", { span: every });
    }
    case "daily":
      return tr("schedules.daily", { time: str(spec.daily) });
    case "weekly": {
      const [day = "", time = ""] = str(spec.weekly).split(" ");
      return tr("schedules.weekly", { day: dayName(day, locale), time });
    }
    case "file":
      return tr("schedules.file", { file: base(str(spec.file)), what: str(spec.pattern) || str(spec.contains) });
    case "process":
      return tr("schedules.process", { pid: num(spec.pid) ?? 0 });
    default:
      return null;
  }
}

/** A condition fires when something happens, not at a time. */
export function isCondition(s: ScheduleRow): boolean {
  return s.kind === "file" || s.kind === "process";
}

/**
 * Whether a schedule is late: it is active, its time passed and it has not
 * fired (the app or the machine is off, or this view has lost its core), or
 * its last firing came late and said so in its note.
 */
export function lateness(s: ScheduleRow, now: number): "overdue" | "fired-late" | null {
  if (s.status === "active" && s.next_at != null && now - s.next_at > LATE_MS) return "overdue";
  if (s.note && /^(late|늦게)/.test(s.note)) return "fired-late";
  return null;
}

/** Soonest first; conditions (no time) after, oldest first. */
function soonest(a: ScheduleRow, b: ScheduleRow): number {
  const x = a.next_at ?? Infinity;
  const y = b.next_at ?? Infinity;
  return x === y ? a.id - b.id : x - y;
}

/** The active ones, soonest first. */
export function upcoming<T extends ScheduleRow>(list: T[]): T[] {
  return list.filter((s) => s.status === "active").sort(soonest);
}

/** The instruction, as the core keeps it; the line's tail when an older core sent no message. */
export function message(s: ScheduleRow): string {
  if (s.message) return s.message;
  const at = s.line.indexOf(": ");
  return at >= 0 ? s.line.slice(at + 2) : s.line;
}
