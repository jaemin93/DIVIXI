/**
 * When divixi may ask GitHub about a release without being asked to.
 *
 * The app checks once at startup, at most once a day, and only while the
 * switch in **Settings → About** is on. Everything that decides that is this
 * file, and it is a pure function of three things — the switch, when the last
 * check was, and what time it is now — so every rule below is a unit test
 * rather than an app someone has to leave running for a day.
 *
 * The cap is on *attempts*, not on answers. A check that failed still moves
 * the stamp: the point of the limit is that this machine makes at most one
 * request a day on its own, and a GitHub that is down or a network that is not
 * there would otherwise be retried at every launch, which is the opposite of
 * what the limit is for. The button in the settings is not capped at all —
 * someone pressing it has asked, and refusing to look because the app looked
 * an hour ago would be answering a question nobody put.
 *
 * Where the two values live: `updates.auto` and `updates.last_check`, in the
 * same settings the rest of the app uses (`get_setting` / `set_setting`, which
 * is `meta` in the store). No new place to keep things.
 */

/** The settings key holding the switch. Absent means on. */
export const AUTO_KEY = "updates.auto";
/** The settings key holding when the last check was, as epoch milliseconds. */
export const LAST_CHECK_KEY = "updates.last_check";

/** How long the app leaves between checks of its own. */
export const CHECK_EVERY_MS = 24 * 60 * 60 * 1000;

/**
 * Whether to check now, and why.
 *
 * The reason is carried, not just the answer: it goes in the log, so "why did
 * it not check?" is a line in a file rather than a thing to work out from the
 * clock.
 */
export type Due =
  | { check: true; why: "asked" | "never" | "due" }
  | { check: false; why: "off" | "recent" };

export type Asking = {
  /** A person pressed the button. Ignores both the switch and the cap. */
  manual: boolean;
  /** The switch in the settings. */
  auto: boolean;
  /** When the last check was, epoch ms, or null for a machine that has never checked. */
  last: number | null;
  /** Now, epoch ms. */
  now: number;
};

/** The whole of the rule. */
export function dueForCheck({ manual, auto, last, now }: Asking): Due {
  // A press is an instruction, not a suggestion: neither the switch nor the
  // day's allowance has anything to say about it.
  if (manual) return { check: true, why: "asked" };
  if (!auto) return { check: false, why: "off" };
  // Never checked, or a stamp that could not be read: check. Erring towards
  // one request is better than a machine that silently never looks again
  // because something wrote nonsense into the setting once.
  if (last === null) return { check: true, why: "never" };
  // A stamp in the future is a clock that was wrong, or one that has been put
  // back. Waiting for it to come round would mean not checking for as long as
  // the error, so it counts as due.
  if (now < last) return { check: true, why: "due" };
  return now - last >= CHECK_EVERY_MS ? { check: true, why: "due" } : { check: false, why: "recent" };
}

/**
 * The switch as it is stored. Anything that is not the word `off` is on:
 * a machine that has never touched the setting checks, and a value nobody
 * recognises is not a reason to stop.
 */
export function autoFrom(raw: string | null | undefined): boolean {
  return raw !== "off";
}

/** The switch, to store. */
export function autoTo(on: boolean): string {
  return on ? "on" : "off";
}

/**
 * A stored stamp as a number, or null for anything that is not one — never
 * written, half written, or written by something else. `Number("")` is 0 and
 * `Number(null)` is 0, either of which would read as 1970 and so as "due",
 * which is the safe way round but not the honest one: those are "no stamp".
 */
export function lastCheckFrom(raw: string | null | undefined): number | null {
  if (raw === null || raw === undefined || raw.trim() === "") return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

/** A stamp, to store. */
export function lastCheckTo(now: number): string {
  return String(Math.round(now));
}
