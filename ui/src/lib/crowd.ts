/**
 * Whether a list column still fits beside a side panel: the window less the
 * rail, the list and the panel leaves the page itself at least `min` wide.
 * When it does not, the list steps aside (not folded for good: it is back
 * as soon as there is room), so the page in the middle never gets no width.
 */
export const MAIN_MIN = 280;

export function listFits(windowWidth: number, rail: number, list: number, side: number, min = MAIN_MIN): boolean {
  return windowWidth - rail - list - side >= min;
}

/** The least a side panel is drawn at, however narrow the window. */
export const SIDE_MIN = 240;

/**
 * A side panel's width as drawn: as the person set it, but no wider than
 * leaves the page beside it `min` (and never under SIDE_MIN). The width
 * kept is not changed: it comes back with the room.
 */
export function sideWidth(windowWidth: number, rail: number, set: number, min = MAIN_MIN): number {
  return Math.max(SIDE_MIN, Math.min(set, windowWidth - rail - min));
}
