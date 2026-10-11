/**
 * The plan's step drawer: a panel under the graph that is toggled up and
 * down as the terminal is under a track, kept open or closed and at its
 * height. Picking a step only picks it; an open drawer shows the step
 * picked, a closed one stays closed. Pure, so it is tested without Svelte.
 */

/** Its settings, kept as the terminal's are (layoutSettings.ts). */
export const DRAWER_KEY = "plandrawer";
export const DRAWER_HEIGHT_KEY = "plandrawer_height";

/** The shortest it is dragged to, and the height a double-click gives. */
export const DRAWER_MIN = 120;
export const DRAWER_RESET = 260;

export type Drawer = { open: boolean; height: number };

/** The tallest it may be in a window this tall: the graph keeps some room. */
export function drawerMax(viewport: number): number {
  return Math.max(160, viewport - 260);
}

export function clampDrawer(px: number, viewport: number): number {
  return Math.min(drawerMax(viewport), Math.max(DRAWER_MIN, Math.round(px)));
}

/** The drawer as last left: closed, and at the reset height, until told otherwise. */
export function drawerFromSettings(open: string | null | undefined, height: string | null | undefined): Drawer {
  const h = Number(height);
  return { open: open === "open", height: Number.isFinite(h) && h > 0 ? h : DRAWER_RESET };
}

/** What is written back, the reverse of [`drawerFromSettings`]. */
export function drawerSettings(d: Drawer): Record<string, string> {
  return { [DRAWER_KEY]: d.open ? "open" : "closed", [DRAWER_HEIGHT_KEY]: String(d.height) };
}

export type Picking = { selected: string[]; picked: string; open: boolean };

/**
 * Pick a step (`add`: add it to, or take it from, the selection). The
 * drawer's open state is left as it was: a press picks, it does not open.
 */
export function pickStep(state: Picking, id: string, add = false): Picking {
  if (add) {
    const has = state.selected.includes(id);
    const selected = has ? state.selected.filter((x) => x !== id) : [...state.selected, id];
    return { selected, picked: selected.length === 1 ? selected[0] : "", open: state.open };
  }
  return { selected: id ? [id] : [], picked: id, open: state.open };
}
