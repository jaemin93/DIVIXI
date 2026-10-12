/**
 * Paging a long conversation: only a page of its turns is drawn at first.
 * Reading back toward the top adds the page above; reading on toward the
 * foot adds the page below. A notice that points at an old turn draws a
 * page around that turn, and the foot is back when the window reaches the end.
 *
 * A window is the index range it draws, [from, to), in a list oldest first,
 * and the conversation it belongs to. `from: null` is the newest page and
 * `to: null` runs to the end. Indices are kept absolute on purpose: a turn
 * that arrives at the foot must not push the top of a window out from under
 * someone reading back. Pure, so `node --test` reaches the rules; the pager
 * (pager.svelte.ts) does the drawing and the scroll repair.
 */

/** Turns drawn at first, and per page after that. */
export const PAGE = 20;

/** How near an edge, in CSS pixels, the view must be to ask for the next page by itself. */
export const NEAR_TOP = 300;

/** Turns a notice's page reaches past the turn it is about (that turn included). */
export const AFTER = 5;

/** A conversation's window: `from` and `to` as above, for the conversation `key`. */
export type PageWindow = { key: string; from: number | null; to: number | null };

export const NO_WINDOW: PageWindow = { key: "", from: null, to: null };

/** The first index of the newest page. */
export function newest(total: number): number {
  return Math.max(0, total - PAGE);
}

/** The first index drawn for conversation `key`, which has `total` turns. */
export function startAt(w: PageWindow, key: string, total: number): number {
  if (w.key !== key || w.from === null) return newest(total);
  return Math.min(w.from, endAt(w, key, total));
}

/** One past the last index drawn for conversation `key`. */
export function endAt(w: PageWindow, key: string, total: number): number {
  if (w.key !== key || w.to === null) return total;
  return Math.min(w.to, total);
}

/** Whether any turns are still left above the window. */
export function hasEarlier(w: PageWindow, key: string, total: number): boolean {
  return startAt(w, key, total) > 0;
}

/** Whether any turns are still left below the window. */
export function hasNewer(w: PageWindow, key: string, total: number): boolean {
  return endAt(w, key, total) < total;
}

/** The window with one more page above it, down to the oldest turn. */
export function widenUp(w: PageWindow, key: string, total: number): PageWindow {
  const end = endAt(w, key, total);
  return { key, from: Math.max(0, startAt(w, key, total) - PAGE), to: end < total ? end : null };
}

/** The window with one more page below it, down to the foot. */
export function widenDown(w: PageWindow, key: string, total: number): PageWindow {
  const next = endAt(w, key, total) + PAGE;
  return { key, from: startAt(w, key, total), to: next < total ? next : null };
}

/**
 * The window a notice about turn `at` draws: the turn, with a little after it
 * and a page of the turns before, and no more. A turn already drawn, or none
 * (`at < 0`), leaves the window as it is.
 */
export function reach(w: PageWindow, key: string, total: number, at: number): PageWindow {
  if (at < 0 || (at >= startAt(w, key, total) && at < endAt(w, key, total))) return w;
  // A page at least, so a turn near the top is not drawn alone (a short window
  // ends up at its foot, and the foot pages on by itself).
  const to = Math.min(total, Math.max(at + AFTER, PAGE));
  return { key, from: Math.max(0, to - PAGE), to: to < total ? to : null };
}

/** The DOM event a notice dispatches to ask the timeline to draw a decision's turn. Detail: the decision id. */
export const REVEAL_DECISION = "divixi:reveal-decision";
