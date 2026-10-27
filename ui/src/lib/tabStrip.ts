/**
 * The arithmetic of a strip of tabs that scrolls sideways: the working
 * folder panel's open files. Offsets are in the strip's scrolled content,
 * as `offsetLeft` gives them; the view is the part of it on screen.
 */

export type Span = { left: number; width: number };
export type View = { scrollLeft: number; width: number };

/**
 * Where to scroll so a tab is wholly in view, `margin` px clear of the
 * edges, or null when it already is. A tab wider than the view lines up
 * with its left edge, where its name starts.
 */
export function scrollToShow(tab: Span, view: View, margin = 0): number | null {
  const right = tab.left + tab.width;
  const viewRight = view.scrollLeft + view.width;
  if (tab.width + 2 * margin >= view.width) {
    const to = Math.max(0, tab.left - margin);
    return to === view.scrollLeft ? null : to;
  }
  if (tab.left - margin < view.scrollLeft) return Math.max(0, tab.left - margin);
  if (right + margin > viewRight) return right + margin - view.width;
  return null;
}

/** The tabs not wholly in view, by index; a pixel of rounding is forgiven. */
export function hiddenTabs(tabs: Span[], view: View): number[] {
  const out: number[] = [];
  const viewRight = view.scrollLeft + view.width;
  tabs.forEach((tab, i) => {
    if (tab.left < view.scrollLeft - 1 || tab.left + tab.width > viewRight + 1) out.push(i);
  });
  return out;
}

/** Pixels in a wheel's line or page step, which some mice report in. */
const LINE = 16;

/**
 * How far a mouse wheel moves the strip sideways, or null to leave the
 * event alone: a sideways wheel or trackpad already scrolls it, and a
 * strip with nothing hidden has nowhere to go.
 */
export function wheelToScroll(e: { deltaX: number; deltaY: number; deltaMode: number }, page: number, overflows: boolean): number | null {
  if (!overflows || e.deltaX !== 0 || e.deltaY === 0) return null;
  if (e.deltaMode === 1) return e.deltaY * LINE;
  if (e.deltaMode === 2) return e.deltaY * page;
  return e.deltaY;
}

/** How far a press must travel before it drags the strip instead of clicking. */
export const DRAG_SLOP = 5;
