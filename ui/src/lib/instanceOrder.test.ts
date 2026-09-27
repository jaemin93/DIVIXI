/** The switcher's order, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { arrange, move, nudge, savedOrder, type InstanceRef } from "./instanceOrder.ts";

const json = (v: unknown) => JSON.stringify(v);

test("nothing stored: Local first, then the hosts as the backend has them", () => {
  assert.deepEqual(arrange([], ["a", "b"]), [null, "a", "b"]);
});

test("the stored order is the order, Local included wherever it sits", () => {
  assert.deepEqual(arrange(["b", null, "a"], ["a", "b"]), ["b", null, "a"]);
  assert.deepEqual(arrange(["a", "b", null], ["a", "b"]), ["a", "b", null]);
});

test("a new remote lands at the end", () => {
  assert.deepEqual(arrange(["b", null], ["a", "b", "c"]), ["b", null, "a", "c"]);
});

test("a deleted instance leaves no hole", () => {
  assert.deepEqual(arrange(["b", null, "a"], ["a"]), [null, "a"]);
});

test("junk in storage cannot break the list", () => {
  assert.deepEqual(arrange([null, null, "a", "a", "gone"], ["a"]), [null, "a"]);
});

test("a pinned setup keeps the order those people saw", () => {
  // Chips were Local and then the pins, in that order.
  const saved = savedOrder(null, json(["b"]));
  assert.deepEqual(saved, [null, "b"]);
  assert.deepEqual(arrange(saved, ["a", "b", "c"]), [null, "b", "a", "c"]);
});

test("a stored order wins over any pins left behind", () => {
  assert.deepEqual(savedOrder(json(["a", null]), json(["b"])), ["a", null]);
});

test("no pins, no order: empty, which arrange fills", () => {
  assert.deepEqual(savedOrder(null, null), []);
  assert.deepEqual(savedOrder(null, json([])), []);
  assert.deepEqual(savedOrder("not json", "not json"), []);
});

const four: InstanceRef[] = [null, "a", "b", "c"];

test("dropping into a gap counts the gaps of the list as it stands", () => {
  assert.deepEqual(move(four, 0, 4), ["a", "b", "c", null]);
  assert.deepEqual(move(four, 3, 0), ["c", null, "a", "b"]);
  assert.deepEqual(move(four, 1, 3), [null, "b", "a", "c"]);
});

test("dropping an item beside itself changes nothing", () => {
  assert.deepEqual(move(four, 1, 1), four);
  assert.deepEqual(move(four, 1, 2), four);
});

test("a drop out of range is left alone, not allowed to lose an item", () => {
  assert.deepEqual(move(four, -1, 2), four);
  assert.deepEqual(move(four, 9, 2), four);
  assert.deepEqual(move(four, 0, 99), ["a", "b", "c", null]);
});

test("the keyboard moves one step, and the ends hold", () => {
  assert.deepEqual(nudge(four, 0, 1), ["a", null, "b", "c"]);
  assert.deepEqual(nudge(four, 3, -1), [null, "a", "c", "b"]);
  assert.deepEqual(nudge(four, 0, -1), four);
  assert.deepEqual(nudge(four, 3, 1), four);
});

test("moving keeps every instance: nothing is dropped or doubled", () => {
  for (let from = 0; from < four.length; from++) {
    for (let to = 0; to <= four.length; to++) {
      const out = move(four, from, to);
      assert.equal(out.length, four.length);
      assert.deepEqual([...out].sort(), [...four].sort());
    }
  }
});
