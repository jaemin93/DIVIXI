/** Which tracks' headers are folded, as the setting keeps them: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { headerFolded, parseHeaderFolds, withHeaderFold, withoutTrack } from "./trackHeaders.ts";

test("with nothing stored, a header starts folded", () => {
  assert.equal(headerFolded({}, "t1"), true);
  assert.equal(headerFolded(parseHeaderFolds(null), "t1"), true);
  assert.equal(headerFolded(parseHeaderFolds(""), "t1"), true);
});

test("a header opened stays open, and one folded stays folded", () => {
  let folds = withHeaderFold({}, "t1", false);
  folds = withHeaderFold(folds, "t2", true);
  const back = parseHeaderFolds(JSON.stringify(folds));
  assert.equal(headerFolded(back, "t1"), false);
  assert.equal(headerFolded(back, "t2"), true);
  assert.equal(headerFolded(back, "t3"), true);
});

test("what an older build stored still reads back folded", () => {
  const back = parseHeaderFolds(JSON.stringify({ t1: true }));
  assert.deepEqual(back, { t1: true });
  assert.equal(headerFolded(back, "t1"), true);
});

test("a stored value that is not a map of booleans counts as nothing stored", () => {
  assert.deepEqual(parseHeaderFolds("not json"), {});
  assert.deepEqual(parseHeaderFolds("[true]"), {});
  assert.deepEqual(parseHeaderFolds("null"), {});
  assert.deepEqual(parseHeaderFolds(JSON.stringify({ t1: "yes", t2: false })), { t2: false });
});

test("a track gone takes its entry with it", () => {
  assert.deepEqual(withoutTrack({ t1: false, t2: true }, "t1"), { t2: true });
});
