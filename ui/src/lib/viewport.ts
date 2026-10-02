/**
 * How big the message box may get, and where the visible screen is.
 *
 * Two questions a phone asks that a desktop window never does: how much of
 * a short screen the box may take, and how much of the screen is left once
 * the soft keyboard is up. The component reads the DOM; the arithmetic is
 * here so it can be tested without a browser.
 */

/** One line of the box, in CSS pixels (its line-height). */
export const LINE = 22;
/** The box's padding, top and bottom together. */
export const PAD = 24;
/** The box's height with one line in it: also the smallest touch target. */
export const MIN_BOX = 44;
/** On a desktop the box grows to eight lines; only past that does it scroll. */
export const DESK_MAX = 8 * LINE + PAD;
/** On a phone it grows to this share of the visible screen. */
export const PHONE_SHARE = 0.4;

/** The tallest the box may grow before it scrolls. */
export function boxLimit(viewportHeight: number, narrow: boolean): number {
  if (!narrow) return DESK_MAX;
  return Math.max(MIN_BOX, Math.floor(viewportHeight * PHONE_SHARE));
}

/** The height to give a box whose content wants `wanted` pixels, and whether it scrolls. */
export function boxFit(wanted: number, limit: number): { height: number; scroll: boolean } {
  return { height: Math.min(wanted, limit), scroll: wanted > limit };
}

/** What the page needs of `window.visualViewport`; a plain object in tests. */
export type Visual = { height: number; offsetTop: number; scale: number };

/**
 * The part of the page the person can see: its height and how far down the
 * page it starts. With the soft keyboard up that is the strip above the
 * keyboard, so the app sized to it keeps the box and the newest message in
 * view. Pinch-zoomed in, the visible part is a magnified corner, not a
 * smaller screen: the app keeps the whole window then.
 */
export function visibleFrame(vv: Visual | null | undefined, innerHeight: number): { height: number; top: number } {
  if (!vv || vv.scale > 1.01 || vv.height <= 0) return { height: innerHeight, top: 0 };
  return { height: Math.round(vv.height), top: Math.max(0, Math.round(vv.offsetTop)) };
}
