/**
 * Sticking a conversation to its foot.
 *
 * One rule, one place: the timeline follows the newest while the view is
 * at the bottom, and holds still the moment it is scrolled up. The
 * component supplies the DOM; every decision is made here, so the rules
 * can be read and tested without a browser.
 *
 * Two events drive it, and only two:
 *
 *   - the content (or the box around it) changed size  -> `resized`
 *   - the view moved                                   -> `scrolled`
 *
 * Everything that puts something in a conversation — a streamed word, a
 * tool line, a decision card, a waiting-report line, the report card that
 * replaces it, the track header folding — changes a height, so `resized`
 * sees all of them without being told about any of them.
 */

/** How near the foot still counts as being at it, in CSS pixels. */
export const BOTTOM_SLACK = 64;

/**
 * How far the view must actually move before a scroll event is taken as
 * the human moving it. A pixel: `scrollTop` is fractional under page
 * zoom, which this app has, so exact comparison would call our own
 * scrolling somebody else's.
 */
export const MOVED_SLOP = 1;

/** What the rules need of an element; a plain object in tests. */
export type ScrollMetrics = { scrollHeight: number; scrollTop: number; clientHeight: number };

/** Pixels of content below the bottom edge of the view. */
export function fromBottom(el: ScrollMetrics): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight;
}

/** Whether the view is at the foot of its content, give or take the slack. */
export function atBottom(el: ScrollMetrics, slack = BOTTOM_SLACK): boolean {
  return fromBottom(el) <= slack;
}

/** The `scrollTop` that puts the view at the foot. */
export function bottomOf(el: ScrollMetrics): number {
  return Math.max(0, el.scrollHeight - el.clientHeight);
}

/** Everything the timeline remembers about where the human is reading. */
export type Stick = {
  /** Following the newest. */
  stuck: boolean;
  /** Something arrived below while they were reading back. */
  missed: boolean;
  /** Where the view sat when we last looked at it. */
  lastTop: number;
};

/** A conversation opens at its newest, following it. */
export const FOLLOWING: Stick = { stuck: true, missed: false, lastTop: 0 };

/**
 * A scroll event.
 *
 * Only a view that actually moved says anything about what the human
 * wants. Content arriving below does not move it, so an event that finds
 * the view exactly where we left it is our own follow being reported back
 * — late, after more content has landed. Reading the position at that
 * moment says "far from the bottom" and letting go of the foot on the
 * strength of it is the bug this guard exists for: a worker report lands
 * in several steps (the turn, then the card, then the card's body once it
 * has been read), and the event for step one arrives after step three.
 */
export function scrolled(s: Stick, el: ScrollMetrics, slack = BOTTOM_SLACK): Stick {
  if (Math.abs(el.scrollTop - s.lastTop) < MOVED_SLOP) return s;
  const stuck = atBottom(el, slack);
  return { stuck, missed: stuck ? false : s.missed, lastTop: el.scrollTop };
}

/**
 * The content, or the box around it, changed size. `follow` asks the
 * caller to put the view back at the foot — and to tell us where it
 * landed with `followed`, since only the DOM knows what the browser
 * clamped to.
 */
export function resized(s: Stick, grew: boolean): { next: Stick; follow: boolean } {
  if (s.stuck) return { next: s, follow: true };
  return { next: grew && !s.missed ? { ...s, missed: true } : s, follow: false };
}

/** The view was put at the foot by us, and landed on `top`. */
export function followed(s: Stick, top: number): Stick {
  return { stuck: true, missed: false, lastTop: top };
}
