/**
 * The arithmetic behind the pairing code's countdown, kept out of
 * `pairing.svelte.ts` so it runs under `node --test` without the runes.
 *
 * Both times are unix seconds: what `phone_pair_link` sends as `expires`.
 */

/** Seconds from `now` until `expires`; never below zero. */
export function secondsLeft(expires: number, now: number): number {
  return Math.max(0, Math.floor(expires - now));
}

/** `m:ss`. */
export function clock(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/**
 * Where a code stands: `none` (nothing in hand, or one being made), `live`,
 * `used` (a device signed in with it: it is good once) or `expired`. The last
 * two are kept, not dropped, so the screen can say so and offer a new one
 * instead of showing a dead code that looks exactly like a live one.
 */
export type CodeState = "none" | "live" | "used" | "expired";

export function codeState(expires: number | null, now: number, used = false): CodeState {
  if (expires === null) return "none";
  if (used) return "used";
  return secondsLeft(expires, now) > 0 ? "live" : "expired";
}

/** Whether `phone_paired` names the code on screen (`id` of `phone_pair_link`). */
export function spends(shown: string | null | undefined, paired: string): boolean {
  return !!shown && shown === paired;
}
