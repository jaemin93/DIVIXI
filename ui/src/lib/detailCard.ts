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
/** A folded card is a header only, this tall. */
export const FOLDED_HEIGHT = 38;

export type CardShape = "none" | "folded" | "open";

/** What the card is showing: nothing picked, a header, or the whole card. */
export function cardShape(picked: boolean, prefs: CardPrefs): CardShape {
  return !picked ? "none" : prefs.folded ? "folded" : "open";
}

/**
 * How far from the plate's right edge the overview rail sits. An open card
 * takes the right-hand column, so the rail moves to its left; a folded card is
 * only a header along the top, so the rail keeps its place on the edge.
 */
export function railRight(shape: CardShape): number {
  return shape === "open" ? CARD_WIDTH + CARD_GAP * 2 : 0;
}

/**
 * The part of the plate (left, top, right, bottom, in pixels) the card covers,
 * for labels to keep out of; null when there is no card. `height` is the
 * card's measured height when open.
 */
export function cardRect(shape: CardShape, plateWidth: number, height: number): [number, number, number, number] | null {
  if (shape === "none") return null;
  const right = plateWidth - CARD_GAP;
  const left = right - CARD_WIDTH;
  const bottom = CARD_GAP + (shape === "folded" ? FOLDED_HEIGHT : Math.max(FOLDED_HEIGHT, height));
  return [left, CARD_GAP, right, bottom];
}

/** Turn one preference over, keeping the other. */
export function toggled(prefs: CardPrefs, key: keyof CardPrefs): CardPrefs {
  return { ...prefs, [key]: !prefs[key] };
}
