/**
 * Where a popover's panel goes so it stays in the window (Popover.svelte).
 * Pure, so it is tested without a browser.
 */

export type Box = { left: number; right: number; top: number; bottom: number };

export type Placed = {
  /** The panel's width: as asked, or the window's less the margins. */
  width: number;
  /** How far to move it sideways from where its alignment puts it. */
  shift: number;
  /** Opened under the trigger, for want of room above. */
  below: boolean;
  /** The most it may be tall on the side it opened. */
  maxHeight: number;
};

/** Kept clear of the window's edges. */
export const MARGIN = 8;
/** Between the trigger and the panel (the CSS `calc(100% + 8px)`). */
const GAP = 8;
/** The panel's own cap (the CSS `max-height`). */
const CAP = 420;

export function placePanel(
  trigger: Box,
  view: { width: number; height: number },
  asked: number,
  height: number,
  align: "left" | "right",
): Placed {
  const width = Math.max(0, Math.min(asked, view.width - 2 * MARGIN));
  const natural = align === "right" ? trigger.right - width : trigger.left;
  const left = Math.min(Math.max(natural, MARGIN), view.width - MARGIN - width);
  const above = trigger.top - GAP - MARGIN;
  const under = view.height - trigger.bottom - GAP - MARGIN;
  const below = Math.min(height, CAP) > above && under > above;
  return { width, shift: left - natural, below, maxHeight: Math.max(0, Math.min(CAP, below ? under : above)) };
}
