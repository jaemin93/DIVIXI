import test from "node:test";
import assert from "node:assert/strict";
import { CARD_GAP, CARD_WIDTH, DEFAULT_PREFS, FOLDED_HEIGHT, RAIL_ROOM, cardRect, cardShape, railRight, railVisible, toggled } from "./detailCard.ts";

test("the card starts open with its sources closed", () => {
  assert.deepEqual(DEFAULT_PREFS, { folded: false, sourcesOpen: false });
  assert.equal(cardShape(true, DEFAULT_PREFS), "open");
});

test("the card's shape: nothing without a pick, a header when folded, the card otherwise", () => {
  assert.equal(cardShape(false, { folded: false, sourcesOpen: true }), "none");
  assert.equal(cardShape(false, { folded: true, sourcesOpen: false }), "none");
  assert.equal(cardShape(true, { folded: true, sourcesOpen: true }), "folded");
});

test("folding and the sources list are separate switches", () => {
  const folded = toggled(DEFAULT_PREFS, "folded");
  assert.deepEqual(folded, { folded: true, sourcesOpen: false });
  assert.deepEqual(toggled(folded, "sourcesOpen"), { folded: true, sourcesOpen: true }, "opening the sources keeps the fold");
  assert.deepEqual(toggled(toggled(folded, "folded"), "folded"), folded, "two turns come back");
  assert.deepEqual(DEFAULT_PREFS, { folded: false, sourcesOpen: false }, "the defaults are not changed in place");
});

test("the overview rail moves aside for an open card only", () => {
  assert.equal(railRight("none"), 0);
  assert.equal(railRight("folded"), 0, "a folded header runs along the top, clear of the rail");
  assert.equal(railRight("open"), CARD_WIDTH + 2 * CARD_GAP);
});

test("labels keep out of the part of the plate the card covers", () => {
  assert.equal(cardRect("none", 1000, 400), null);
  assert.deepEqual(cardRect("folded", 1000, 400), [1000 - CARD_GAP - CARD_WIDTH, CARD_GAP, 1000 - CARD_GAP, CARD_GAP + FOLDED_HEIGHT]);
  assert.deepEqual(cardRect("open", 1000, 400), [1000 - CARD_GAP - CARD_WIDTH, CARD_GAP, 1000 - CARD_GAP, CARD_GAP + 400]);
  assert.equal(cardRect("open", 1000, 0)![3], CARD_GAP + FOLDED_HEIGHT, "never shorter than its header, before it is measured");
});

test("on a narrow plate the rail steps out for an open card, and only then", () => {
  assert.equal(railVisible("open", RAIL_ROOM - 1), false, "the agent panel open beside a picked node");
  assert.equal(railVisible("open", RAIL_ROOM), true);
  assert.equal(railVisible("folded", 300), true, "a folded header leaves it its place");
  assert.equal(railVisible("none", 300), true);
  assert.equal(railVisible("open", 0), true, "not measured yet: shown");
});
