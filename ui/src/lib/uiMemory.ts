/**
 * What a screen remembers of how the person left it, where the PC's own
 * settings cannot keep it (`layoutSettings.ts`): the zoom of a remote
 * instance's webview, and whether a phone's track list was open.
 *
 * Both are the screen's, not the Divixi's. A remote instance's zoom is kept
 * on this PC, one per instance, so the same instance opened on a phone does
 * not inherit a 34-inch monitor's 110%. A phone's panels are kept in that
 * browser, apart from the desktop window's. Reading and writing are the
 * callers'; the rules (keys, bounds, defaults) are here so they can be
 * tested without a webview.
 */

/** Zoom bounds in percent; steps of 10, as Ctrl+= / Ctrl+- move. */
export const ZOOM_MIN = 50;
export const ZOOM_MAX = 200;
export const ZOOM_STEP = 10;

/** A zoom asked for, on a step and within bounds. Anything not a number is 100%. */
export function clampZoom(percent: number): number {
  if (!Number.isFinite(percent)) return 100;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(percent / ZOOM_STEP) * ZOOM_STEP));
}

/**
 * A stored zoom read back: the percent to apply, or null to stay at 100%.
 * A value that is not a plain number within bounds (hand-edited, from an
 * older build, cut short) is ignored rather than trusted: a corrupt 5000
 * must not open a window nobody can use.
 */
export function parseZoom(raw: string | null | undefined): number | null {
  if (typeof raw !== "string" || !/^\s*\d+(\.\d+)?\s*$/.test(raw)) return null;
  const z = Number(raw);
  if (!Number.isFinite(z) || z < ZOOM_MIN || z > ZOOM_MAX) return null;
  return clampZoom(z);
}

/**
 * The key under which this PC keeps a remote instance's zoom (in its own
 * settings, through `client_get_setting`). One per instance: each has a
 * webview of its own, and a zoom of its own.
 */
export function instanceZoomKey(instanceId: string): string {
  return `instance:${instanceId}:zoom`;
}

/**
 * Where a browser keeps whether its side panels are open: the track list,
 * and the rail beside it. The desktop window's are settings of the PC.
 */
export const WEB_TRACKLIST_KEY = "divixi.tracklist";
export const WEB_RAIL_KEY = "divixi.rail";

/**
 * Whether a browser's side panel (the track list, the rail spelled out)
 * starts open: as the person last left it on this device, else closed on a
 * phone (narrow or touch) and open elsewhere. On a phone either one takes
 * half the screen or more, so it waits to be asked for.
 */
export function webPanelOpen(stored: string | null | undefined, phone: boolean): boolean {
  if (stored === "open") return true;
  if (stored === "closed") return false;
  return !phone;
}

/**
 * Whether picking a track folds the list away. Only in a browser too narrow
 * for the list and the track side by side, where the list is the way to the
 * track and is done with once it is picked; the desktop window keeps its
 * list where it is, as it always has.
 */
export function foldsOnPick(overWeb: boolean, narrow: boolean): boolean {
  return overWeb && narrow;
}
