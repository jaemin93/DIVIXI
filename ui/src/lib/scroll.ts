/**
 * Sticking a conversation to its foot.
 *
 * One rule, one place: the timeline follows the newest while the view is
 * at the bottom, and holds still the moment it is scrolled up. The
 * component supplies the DOM; every decision is made here, so the rules
 * can be read and tested without a browser.
 *
 * Three events drive it, and only three:
 *
 *   - the content (or the box around it) changed size  -> `resized`
 *   - the view moved                                   -> `scrolled`
 *   - the human reached for it                         -> `reached`
 *
 * Everything that puts something in a conversation — a streamed word, a
 * tool line, a decision card, a waiting-report line, the report card that
 * replaces it, the track header folding — changes a height, so `resized`
 * sees all of them without being told about any of them.
 *
 * The third exists because the first two cannot be told apart by position:
 * a scroll event does not say who caused it. `reached` is the human's hand
 * on the wheel, which nothing we do can imitate.
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
  const moved = el.scrollTop - s.lastTop;
  if (Math.abs(moved) < MOVED_SLOP) return s;
  // Taking the foot back is something the human does by scrolling *down* to
  // it. Merely being near it is not enough once it has been let go of: while
  // a turn streams, an upward nudge lands inside the slack, and re-arming
  // there would hand the view straight back to `follow` — which is how the
  // conversation used to be impossible to scroll up at all. Above the foot,
  // only a downward move takes it back.
  //
  // Landing *on* the foot takes it back whichever way the view got there: the
  // content shrinking under someone reading back (a card they folded) clamps
  // them to it, and there is no downward move left to make from there.
  const onFoot = fromBottom(el) < MOVED_SLOP;
  const stuck = s.stuck ? atBottom(el, slack) : (moved > 0 || onFoot) && atBottom(el, slack);
  return { stuck, missed: stuck ? false : s.missed, lastTop: el.scrollTop };
}

/**
 * The human reached for the view — a wheel, a finger, a paging key — and
 * pulled it back from the newest.
 *
 * This is the one signal that cannot be mistaken for our own following:
 * `follow` moves a view, it never turns a wheel. Position alone cannot tell
 * the two apart — that is what `scrolled` has to be careful about, and the
 * care costs the slack. An upward nudge of a few dozen pixels still reads as
 * "at the foot", and while we count as following, the next thing to arrive
 * puts the view back; the gesture never accumulates past the slack, so under
 * a stream the conversation cannot be scrolled up at all.
 *
 * Reading the gesture from the input event closes that gap. The event arrives
 * before the scroll it causes, so the foot is let go of before anything that
 * arrives can follow it.
 */
export function reached(s: Stick, el: ScrollMetrics, up: boolean): Stick {
  // Not reaching back, already reading back, or nowhere to go: leave it be.
  // In particular leave `lastTop` alone, so a gesture in progress keeps the
  // position its scroll events are measured against.
  if (!up || !s.stuck || bottomOf(el) <= 0) return s;
  return { stuck: false, missed: false, lastTop: el.scrollTop };
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
