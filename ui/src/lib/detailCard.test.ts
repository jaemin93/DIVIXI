import test from "node:test";
import assert from "node:assert/strict";
import { CARD_GAP, CARD_TOP, CARD_WIDTH, DEFAULT_PREFS, FOLDED_HEIGHT, RAIL_ROOM, cardMaxHeight, cardRect, cardShape, listMinLeft, railVisible, toggled } from "./detailCard.ts";

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

test("labels keep out of the plate's top left, where the card is", () => {
  assert.equal(cardRect("none", 1000, 400), null);
  assert.deepEqual(cardRect("folded", 1000, 400), [CARD_GAP, CARD_TOP, CARD_GAP + CARD_WIDTH, CARD_TOP + FOLDED_HEIGHT]);
  assert.deepEqual(cardRect("open", 1000, 400), [CARD_GAP, CARD_TOP, CARD_GAP + CARD_WIDTH, CARD_TOP + 400]);
  assert.equal(cardRect("open", 1000, 0)![3], CARD_TOP + FOLDED_HEIGHT, "never shorter than its header, before it is measured");
  assert.equal(cardRect("open", 200, 400)![2], 200, "no wider than the plate");
});

test("a kind's node list opens beside the card, not over it", () => {
  assert.equal(listMinLeft("none"), CARD_GAP);
  assert.equal(listMinLeft("folded"), CARD_WIDTH + 2 * CARD_GAP);
  assert.equal(listMinLeft("open"), CARD_WIDTH + 2 * CARD_GAP);
});

test("the card stops short of the query and centre boxes at the bottom left", () => {
  assert.equal(cardMaxHeight(600, 0, 34), 600 - CARD_TOP - CARD_GAP, "no boxes: down to the margin");
  assert.equal(cardMaxHeight(600, 120, 34), 600 - CARD_TOP - 120 - 34 - CARD_GAP);
  assert.equal(cardMaxHeight(150, 120, 34), FOLDED_HEIGHT, "never less than its header");
});

test("on a narrow plate the rail steps out for an open card, and only then", () => {
  assert.ok(RAIL_ROOM < 760, "the card no longer pushes the rail inwards, so it needs less room");
  assert.equal(railVisible("open", RAIL_ROOM - 1), false, "the agent panel open beside a picked node");
  assert.equal(railVisible("open", RAIL_ROOM), true);
  assert.equal(railVisible("folded", 300), true, "a folded header leaves it its place");
  assert.equal(railVisible("none", 300), true);
  assert.equal(railVisible("open", 0), true, "not measured yet: shown");
});
