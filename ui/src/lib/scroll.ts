/**
 * Where a scrolling conversation counts as being "at the bottom".
 *
 * One rule, one place: the timeline sticks to the newest while this says
 * the view is at the foot, and holds still the moment it is not. The slack
 * matters — asking for exactly zero means a stray pixel from a font or a
 * zoom level quietly turns following off — so it is a named number here
 * rather than a literal buried in a handler.
 */

/** How near the foot still counts as being at it, in CSS pixels. */
export const BOTTOM_SLACK = 64;

/** What the rule needs of an element; a plain object in tests. */
export type ScrollMetrics = { scrollHeight: number; scrollTop: number; clientHeight: number };

/** Pixels of content below the bottom edge of the view. */
export function fromBottom(el: ScrollMetrics): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight;
}

/** Whether the view is at the foot of its content, give or take the slack. */
export function atBottom(el: ScrollMetrics, slack = BOTTOM_SLACK): boolean {
  return fromBottom(el) <= slack;
}
