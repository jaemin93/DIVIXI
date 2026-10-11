import test from "node:test";
import assert from "node:assert/strict";
import { MAIN_MIN, listFits } from "./crowd.ts";

test("the list steps aside when the page would get less than its least width", () => {
  // The case found: a 700px window, rail 200, list 264, a 460px panel.
  assert.equal(listFits(700, 200, 264, 460), false);
  assert.equal(listFits(1400, 200, 264, 460), true);
  assert.equal(listFits(1283, 200, 264, 460), true, "a usual laptop window keeps its list beside the panel");
  assert.equal(listFits(200 + 264 + 460 + MAIN_MIN, 200, 264, 460), true, "exactly enough is enough");
  assert.equal(listFits(1000, 56, 264, 0), true, "no panel: the list stays");
});
