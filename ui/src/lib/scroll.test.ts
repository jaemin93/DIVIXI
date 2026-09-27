/** The stick-to-bottom rule, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { BOTTOM_SLACK, atBottom, fromBottom, type ScrollMetrics } from "./scroll.ts";

/** A view 500 tall over `tall` of content, scrolled to `top`. */
function view(tall: number, top: number): ScrollMetrics {
  return { scrollHeight: tall, scrollTop: top, clientHeight: 500 };
}

test("exactly at the foot", () => {
  assert.equal(fromBottom(view(2000, 1500)), 0);
  assert.equal(atBottom(view(2000, 1500)), true);
});

test("a few pixels short still counts as the foot", () => {
  // The point of the slack: a stray pixel from a font or a zoom level must
  // not quietly turn following off.
  assert.equal(atBottom(view(2000, 1499)), true);
  assert.equal(atBottom(view(2000, 1500 - BOTTOM_SLACK)), true);
});

test("just past the slack is reading back, and is left alone", () => {
  assert.equal(atBottom(view(2000, 1500 - BOTTOM_SLACK - 1)), false);
  assert.equal(atBottom(view(2000, 0)), false);
});

test("content shorter than the view is always at the foot", () => {
  assert.equal(atBottom(view(300, 0)), true);
});

test("content growing below a still view takes it off the foot", () => {
  // Streaming: scrollTop does not move, scrollHeight does.
  const before = view(2000, 1500);
  assert.equal(atBottom(before), true);
  const after = { ...before, scrollHeight: 2400 };
  assert.equal(fromBottom(after), 400);
  assert.equal(atBottom(after), false);
});
