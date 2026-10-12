/**
 * Paging a long conversation: only the newest turns are drawn at first, and
 * each page of older ones is added when the human reads back toward the top.
 *
 * A window is the index its first shown turn sits at, in a list oldest
 * first, and the conversation it belongs to. The index is kept absolute on
 * purpose: a turn that arrives at the foot while someone reads back must not
 * push the top of their window out from under them. Pure, so `node --test`
 * reaches the rules; the components do the drawing and the scroll repair.
 */

/** Turns drawn at first, and per page of earlier ones. */
export const PAGE = 20;

/** How near the top, in CSS pixels, the view must be to ask for the next page by itself. */
export const NEAR_TOP = 300;

/** Where a conversation's window starts. `from: null` has never been widened. */
export type PageWindow = { key: string; from: number | null };

export const NO_WINDOW: PageWindow = { key: "", from: null };

/** The first index of the newest page. */
export function newest(total: number): number {
  return Math.max(0, total - PAGE);
}

/** The first index drawn for conversation `key`, which has `total` turns. */
export function startAt(w: PageWindow, key: string, total: number): number {
  if (w.key !== key || w.from === null) return newest(total);
  return Math.min(w.from, total);
}

/** The window with one more page of earlier turns, down to the oldest. */
export function widen(w: PageWindow, key: string, total: number): PageWindow {
  return { key, from: Math.max(0, startAt(w, key, total) - PAGE) };
}

/** Whether any turns are still left above the window. */
export function hasEarlier(w: PageWindow, key: string, total: number): boolean {
  return startAt(w, key, total) > 0;
}

/**
 * The window reaching back, if it must, so the turn at index `at` is drawn
 * with a page above it. A turn already drawn, or none (`at < 0`), leaves it be.
 */
export function reach(w: PageWindow, key: string, total: number, at: number): PageWindow {
  if (at < 0 || at >= startAt(w, key, total)) return w;
  return { key, from: Math.max(0, at - PAGE) };
}

/** The DOM event a notice dispatches to ask the timeline to draw a decision's turn. Detail: the decision id. */
export const REVEAL_DECISION = "divixi:reveal-decision";
