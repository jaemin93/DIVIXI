/** The paging rules for long conversations, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { AFTER, NO_WINDOW, PAGE, endAt, hasEarlier, hasNewer, newest, reach, startAt, widenDown, widenUp, type PageWindow } from "./history.ts";

const KEY = "track:a";

/** The turn indices a window draws, in order. */
function drawn(w: PageWindow, total: number, key = KEY): number[] {
  const from = startAt(w, key, total);
  const to = endAt(w, key, total);
  return Array.from({ length: Math.max(0, to - from) }, (_, i) => from + i);
}

const range = (from: number, to: number) => Array.from({ length: Math.max(0, to - from) }, (_, i) => from + i);

test("the newest page is the last PAGE turns", () => {
  assert.equal(newest(0), 0);
  assert.equal(newest(PAGE), 0);
  assert.equal(newest(PAGE + 1), 1);
  assert.equal(newest(750), 750 - PAGE);
});

test("a conversation not seen before draws its newest page to the end", () => {
  assert.deepEqual(drawn(NO_WINDOW, 45), range(45 - PAGE, 45));
  assert.equal(hasEarlier(NO_WINDOW, KEY, 45), true);
  assert.equal(hasNewer(NO_WINDOW, KEY, 45), false);
  assert.equal(hasEarlier(NO_WINDOW, KEY, PAGE), false);
});

test("a window of another conversation does not carry over", () => {
  const w = widenUp(NO_WINDOW, KEY, 750);
  assert.equal(startAt(w, "track:b", 750), newest(750));
  assert.equal(endAt(w, "track:b", 750), 750);
});

test("each widen up adds one page and stops at the oldest turn", () => {
  let w = NO_WINDOW;
  const starts: number[] = [];
  for (let i = 0; i < 40; i++) {
    w = widenUp(w, KEY, 750);
    starts.push(startAt(w, KEY, 750));
  }
  assert.equal(starts[0], 750 - 2 * PAGE);
  assert.equal(starts[1], 750 - 3 * PAGE);
  assert.equal(starts.at(-1), 0);
  assert.equal(hasEarlier(w, KEY, 750), false);
  assert.equal(startAt(widenUp(w, KEY, 750), KEY, 750), 0);
});

test("pages up never overlap and never leave a gap", () => {
  for (const total of [0, 1, PAGE - 1, PAGE, PAGE + 1, 45, 750]) {
    let w = NO_WINDOW;
    let seen = drawn(w, total);
    for (let guard = 0; hasEarlier(w, KEY, total) && guard < 1000; guard++) {
      w = widenUp(w, KEY, total);
      const next = drawn(w, total);
      assert.deepEqual(next.slice(next.length - seen.length), seen, `total ${total}`);
      seen = next;
    }
    assert.deepEqual(seen, range(0, total), `total ${total}`);
  }
});

test("turns that arrive at the foot leave the newest window where it is", () => {
  const w = widenUp(NO_WINDOW, KEY, 750);
  const from = startAt(w, KEY, 750);
  assert.equal(startAt(w, KEY, 751), from);
  assert.equal(drawn(w, 751).length, 751 - from);
});

test("a notice reaches a turn above the window: a page around it, not to the end", () => {
  const total = 750;
  const w = reach(widenUp(NO_WINDOW, KEY, total), KEY, total, 100);
  assert.equal(endAt(w, KEY, total), 100 + AFTER);
  assert.equal(startAt(w, KEY, total), 100 + AFTER - PAGE);
  assert.equal(drawn(w, total).length, PAGE);
  assert.ok(drawn(w, total).includes(100));
  assert.equal(hasNewer(w, KEY, total), true);
  assert.equal(hasEarlier(w, KEY, total), true);
});

test("a notice near the end draws through to the foot", () => {
  const total = 750;
  const w = reach(NO_WINDOW, KEY, total, 748);
  assert.equal(endAt(w, KEY, total), total);
  assert.equal(hasNewer(w, KEY, total), false);
  assert.ok(drawn(w, total).includes(748));
});

test("a notice near the top draws a whole page from the first turn", () => {
  const w = reach(NO_WINDOW, KEY, 750, 3);
  assert.equal(startAt(w, KEY, 750), 0);
  assert.equal(endAt(w, KEY, 750), PAGE);
  assert.equal(hasEarlier(w, KEY, 750), false);
  assert.ok(hasNewer(w, KEY, 750));
});

test("a notice about a turn already drawn, or none, leaves the window alone", () => {
  const total = 750;
  const w = widenUp(NO_WINDOW, KEY, total);
  assert.equal(reach(w, KEY, total, 740), w);
  assert.equal(reach(w, KEY, total, -1), w);
  const r = reach(NO_WINDOW, KEY, total, 100);
  assert.equal(reach(r, KEY, total, 101), r);
});

test("paging from a notice's window works both ways, and back to the foot", () => {
  const total = 750;
  let w = reach(NO_WINDOW, KEY, total, 300);
  const first = drawn(w, total);
  // Up: older pages land above the span already drawn, no repeats, no gap.
  let up = w;
  let seenUp = first;
  for (let guard = 0; hasEarlier(up, KEY, total) && guard < 100; guard++) {
    up = widenUp(up, KEY, total);
    const next = drawn(up, total);
    assert.deepEqual(next.slice(next.length - seenUp.length), seenUp, "up keeps the old span");
    seenUp = next;
  }
  assert.deepEqual(seenUp, range(0, endAt(w, KEY, total)));
  // Down: newer pages land below the window until the foot, then it is open-ended.
  let down = w;
  let seen = drawn(down, total);
  for (let guard = 0; hasNewer(down, KEY, total) && guard < 1000; guard++) {
    down = widenDown(down, KEY, total);
    const next = drawn(down, total);
    assert.deepEqual(next.slice(0, seen.length), seen, "down keeps the old span");
    seen = next;
  }
  assert.deepEqual(seen, range(startAt(down, KEY, total), total));
  assert.equal(seen.at(-1), total - 1);
  assert.equal(down.to, null, "the foot is open-ended again, so new turns follow");
  assert.equal(drawn(down, total + 3).at(-1), total + 2);
});

test("a notice window with turns arriving at the foot keeps its place", () => {
  const total = 750;
  const w = reach(NO_WINDOW, KEY, total, 300);
  assert.equal(startAt(w, KEY, total + 5), startAt(w, KEY, total));
  assert.equal(endAt(w, KEY, total + 5), endAt(w, KEY, total));
  assert.equal(hasNewer(w, KEY, total + 5), true);
});

test("a window past the end of a shrunken conversation is clamped", () => {
  const w = { key: KEY, from: 700, to: null };
  assert.equal(startAt(w, KEY, 500), 500);
  assert.equal(endAt({ key: KEY, from: 300, to: 900 }, KEY, 500), 500);
});
