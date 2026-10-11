import test from "node:test";
import assert from "node:assert/strict";
import { MARGIN, placePanel } from "./popoverPlace.ts";

const view = { width: 1000, height: 800 };

test("a panel with room stays where its alignment puts it", () => {
  const p = placePanel({ left: 100, right: 200, top: 600, bottom: 630 }, view, 520, 300, "left");
  assert.deepEqual(p, { width: 520, shift: 0, below: false, maxHeight: 420 });
});

test("a panel that would cross the right edge is pushed in", () => {
  // The bug: [Planning agent] near the right of the form, a 520px menu from its left.
  const t = { left: 800, right: 900, top: 600, bottom: 630 };
  const p = placePanel(t, view, 520, 300, "left");
  assert.equal(t.left + p.shift + p.width, view.width - MARGIN);
});

test("a right-aligned panel that would cross the left edge is pushed in", () => {
  const t = { left: 20, right: 120, top: 600, bottom: 630 };
  const p = placePanel(t, view, 320, 200, "right");
  assert.equal(t.right - p.width + p.shift, MARGIN);
});

test("a window narrower than the panel gets it at the window's width, inside both edges", () => {
  const phone = { width: 390, height: 844 };
  const t = { left: 250, right: 370, top: 700, bottom: 730 };
  const p = placePanel(t, phone, 520, 300, "left");
  assert.equal(p.width, 390 - 2 * MARGIN);
  const left = t.left + p.shift;
  assert.equal(left, MARGIN);
  assert.ok(left + p.width <= phone.width - MARGIN);
});

test("no room above opens it below, and its height fits the side it opened", () => {
  const p = placePanel({ left: 100, right: 200, top: 40, bottom: 70 }, view, 320, 300, "left");
  assert.equal(p.below, true);
  assert.equal(p.maxHeight, 420);
  const tight = placePanel({ left: 100, right: 200, top: 300, bottom: 330 }, { width: 1000, height: 500 }, 320, 400, "left");
  assert.equal(tight.below, false, "more room above than below: stays above");
  assert.equal(tight.maxHeight, 300 - 8 - MARGIN);
});
