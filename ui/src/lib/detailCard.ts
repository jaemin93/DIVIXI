/**
 * The graph's entity card, folded or not, and where it leaves room for the
 * rest of the plate. Kept apart from the component so the rules are tested.
 *
 * The card used to open at full size every time a node was picked, with the
 * description and every passage that mentions the entity, and it covered a
 * good third of the plate. Now it can fold to a one-line header (the name and
 * its kind), and the passages are a list of their own that starts closed.
 * Both choices last while the app runs: picking another node keeps them.
 */

export type CardPrefs = { folded: boolean; sourcesOpen: boolean };

/** How the card starts the first time: open, its sources closed. */
export const DEFAULT_PREFS: CardPrefs = { folded: false, sourcesOpen: false };

/** The card's width when open, and its gap from the plate's edge. */
export const CARD_WIDTH = 280;
export const CARD_GAP = 12;
/** The card's top: under the plate's caption (FIG. 01 …), top left. */
export const CARD_TOP = 40;
/** A folded card is a header only, this tall. */
export const FOLDED_HEIGHT = 38;

export type CardShape = "none" | "folded" | "open";

/** What the card is showing: nothing picked, a header, or the whole card. */
export function cardShape(picked: boolean, prefs: CardPrefs): CardShape {
  return !picked ? "none" : prefs.folded ? "folded" : "open";
}

/**
 * Below this plate width an open card (top left) and the overview rail
 * (right edge, about 150 px with its labels) would meet.
 */
export const RAIL_ROOM = 480;

/**
 * Whether the overview rail is shown. The card is on the left and the rail
 * on the right edge, so the rail keeps its place; only on a plate too narrow
 * for both (the agent panel open beside it, a small window) does it step out
 * until the card is folded or closed.
 */
export function railVisible(shape: CardShape, plateWidth: number): boolean {
  return !(shape === "open" && plateWidth > 0 && plateWidth < RAIL_ROOM);
}

/**
 * The part of the plate (left, top, right, bottom, in pixels) the card covers,
 * for labels to keep out of; null when there is no card. `height` is the
 * card's measured height when open.
 */
export function cardRect(shape: CardShape, plateWidth: number, height: number): [number, number, number, number] | null {
  if (shape === "none") return null;
  const left = CARD_GAP;
  const right = Math.min(plateWidth, left + CARD_WIDTH);
  const bottom = CARD_TOP + (shape === "folded" ? FOLDED_HEIGHT : Math.max(FOLDED_HEIGHT, height));
  return [left, CARD_TOP, right, bottom];
}

/**
 * Where a kind's node list may start on the left: beside the card when one
 * is shown (the list drops from the legend over the plate's top left, where
 * the card is), else at the plate's margin.
 */
export function listMinLeft(shape: CardShape): number {
  return shape === "none" ? CARD_GAP : CARD_GAP * 2 + CARD_WIDTH;
}

/**
 * The card's tallest, in pixels of a plate `plateHeight` tall: down to the
 * plate's margin, or, with the query and centre boxes shown at the bottom
 * left (`boxes` px tall, their bottom `boxesBottom` px up), down to them.
 */
export function cardMaxHeight(plateHeight: number, boxes: number, boxesBottom: number): number {
  const floor = boxes > 0 ? boxes + boxesBottom + CARD_GAP : CARD_GAP;
  return Math.max(FOLDED_HEIGHT, plateHeight - CARD_TOP - floor);
}

/** Turn one preference over, keeping the other. */
export function toggled(prefs: CardPrefs, key: keyof CardPrefs): CardPrefs {
  return { ...prefs, [key]: !prefs[key] };
}
