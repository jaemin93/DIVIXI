/**
 * Whether a list column still fits beside a side panel: the window less the
 * rail, the list and the panel leaves the page itself at least `min` wide.
 * When it does not, the list steps aside (not folded for good: it is back
 * as soon as there is room), so the page in the middle never gets no width.
 */
export const MAIN_MIN = 360;

export function listFits(windowWidth: number, rail: number, list: number, side: number, min = MAIN_MIN): boolean {
  return windowWidth - rail - list - side >= min;
}
