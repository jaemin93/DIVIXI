/** The paging rules for long conversations, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { NO_WINDOW, PAGE, hasEarlier, newest, reach, startAt, widen } from "./history.ts";

/** The turn indices a window draws, in order. */
function drawn(w: { key: string; from: number | null }, key: string, total: number): number[] {
  const from = startAt(w, key, total);
  return Array.from({ length: total - from }, (_, i) => from + i);
}

test("the newest page is the last PAGE turns", () => {
  assert.equal(newest(0), 0);
  assert.equal(newest(PAGE), 0);
  assert.equal(newest(PAGE + 1), 1);
  assert.equal(newest(750), 750 - PAGE);
});

test("a conversation not seen before draws its newest page", () => {
  assert.deepEqual(drawn(NO_WINDOW, "track:a", 45), Array.from({ length: PAGE }, (_, i) => 45 - PAGE + i));
  assert.equal(hasEarlier(NO_WINDOW, "track:a", 45), true);
  assert.equal(hasEarlier(NO_WINDOW, "track:a", PAGE), false);
});

test("a window of another conversation does not carry over", () => {
  const w = widen(NO_WINDOW, "track:a", 750);
  assert.equal(startAt(w, "track:b", 750), newest(750));
});

test("each widen adds one page and stops at the oldest turn", () => {
  const key = "track:a";
  let w = NO_WINDOW;
  const starts: number[] = [];
  for (let i = 0; i < 40; i++) {
    w = widen(w, key, 750);
    starts.push(startAt(w, key, 750));
  }
  assert.equal(starts[0], 750 - 2 * PAGE);
  assert.equal(starts[1], 750 - 3 * PAGE);
  assert.equal(starts.at(-1), 0);
  assert.equal(hasEarlier(w, key, 750), false);
  assert.equal(startAt(widen(w, key, 750), key, 750), 0);
});

test("pages never overlap and never leave a gap", () => {
  for (const total of [0, 1, PAGE - 1, PAGE, PAGE + 1, 45, 750]) {
    const key = `track:${total}`;
    let w = NO_WINDOW;
    let seen = drawn(w, key, total);
    let guard = 0;
    while (hasEarlier(w, key, total) && guard++ < 1000) {
      const before = startAt(w, key, total);
      w = widen(w, key, total);
      const next = drawn(w, key, total);
      // The new page is exactly the turns just above the old one.
      assert.deepEqual(next.slice(0, before - startAt(w, key, total)), Array.from({ length: before - startAt(w, key, total) }, (_, i) => startAt(w, key, total) + i));
      assert.deepEqual(next.slice(before - startAt(w, key, total)), seen);
      seen = next;
    }
    assert.deepEqual(seen, Array.from({ length: total }, (_, i) => i), `total ${total}`);
  }
});

test("turns that arrive at the foot leave the window where it is", () => {
  const key = "track:a";
  const w = widen(NO_WINDOW, key, 750);
  const from = startAt(w, key, 750);
  // A new turn lands at the foot while the human reads back: the top stays put.
  assert.equal(startAt(w, key, 751), from);
  assert.equal(drawn(w, key, 751).length, 751 - from);
});

test("reaching a turn above the window draws it with a page above", () => {
  const key = "track:a";
  const total = 750;
  const w = widen(NO_WINDOW, key, total);
  const reached = reach(w, key, total, 100);
  assert.equal(startAt(reached, key, total), 100 - PAGE);
  assert.deepEqual(drawn(reached, key, total).slice(0, PAGE), Array.from({ length: PAGE }, (_, i) => 80 + i));
  assert.equal(drawn(reached, key, total).at(-1), 749);
});

test("reaching a turn already drawn, or none, leaves the window alone", () => {
  const key = "track:a";
  const w = widen(NO_WINDOW, key, 750);
  assert.equal(reach(w, key, 750, 740), w);
  assert.equal(reach(w, key, 750, -1), w);
  assert.equal(startAt(reach(NO_WINDOW, key, 750, 3), key, 750), 0);
});

test("a window past the end of a shrunken conversation is clamped", () => {
  const key = "track:a";
  const w = { key, from: 700 };
  assert.equal(startAt(w, key, 500), 500);
});
