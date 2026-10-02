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
 * or `expired` — kept, not dropped, so the screen can say so and offer a new
 * one instead of looking as if it were still making one.
 */
export type CodeState = "none" | "live" | "expired";

export function codeState(expires: number | null, now: number): CodeState {
  if (expires === null) return "none";
  return secondsLeft(expires, now) > 0 ? "live" : "expired";
}
