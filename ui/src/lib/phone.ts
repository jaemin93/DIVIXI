/**
 * A phone, for the UI: its width (Track's breakpoint), a page that outlived
 * an update, and two fingers on the knowledge graph, where the camera zooms
 * by how far apart they are now against when they landed, about the point
 * under them, which stays under them. Kept apart from the components so it
 * is tested.
 *
 * The graph's camera puts world point (wx, wy) at screen point
 * (cx + view.x + wx * k, cy + view.y + wy * k), (cx, cy) the plate's middle.
 */

export type Pt = { x: number; y: number };
export type View = { x: number; y: number; k: number };

export const dist = (a: Pt, b: Pt): number => Math.hypot(a.x - b.x, a.y - b.y);
export const mid = (a: Pt, b: Pt): Pt => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });

/** The world point under a screen point, for a camera and the plate's middle. */
export function toWorld(p: Pt, view: View, centre: Pt): Pt {
  return { x: (p.x - centre.x - view.x) / view.k, y: (p.y - centre.y - view.y) / view.k };
}

/**
 * The camera for a pinch: the zoom scaled by `nowDist / startDist` from
 * `startK`, within `[min, max]`, with `anchor` (the world point under the
 * fingers when they landed) put under their midpoint now.
 */
export function pinchView(startK: number, startDist: number, nowDist: number, anchor: Pt, nowMid: Pt, centre: Pt, min: number, max: number): View {
  const k = Math.min(max, Math.max(min, startK * (startDist > 0 ? nowDist / startDist : 1)));
  return { k, x: nowMid.x - centre.x - anchor.x * k, y: nowMid.y - centre.y - anchor.y * k };
}

/** Whether the page is a phone's width (Track's breakpoint, App.svelte's `narrow`). */
export const PHONE_WIDTH = 640;
export function phoneWidth(width: number): boolean {
  return width > 0 && width <= PHONE_WIDTH;
}

/**
 * Whether a socket that came back is talking to another build than the one
 * this page was loaded from: the page then reloads, so an updated app does
 * not go on being driven by the old page (a phone's tab outlives the PC's
 * restarts). The first build seen is the page's own; nothing known, nothing
 * to compare.
 */
export function staleBuild(first: string | null | undefined, now: string | null | undefined): boolean {
  return !!first && !!now && first !== now;
}
