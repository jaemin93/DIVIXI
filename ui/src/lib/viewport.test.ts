/** The message box's size and the visible screen: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { DESK_MAX, MIN_BOX, boxFit, boxLimit, visibleFrame } from "./viewport.ts";

test("a desktop window keeps its eight lines, whatever its height", () => {
  assert.equal(DESK_MAX, 200);
  assert.equal(boxLimit(400, false), 200);
  assert.equal(boxLimit(1400, false), 200);
});

test("on a phone the box stops at 40% of what is visible", () => {
  assert.equal(boxLimit(812, true), 324);
  // Keyboard up: the visible strip is shorter, and so is the box's share.
  assert.equal(boxLimit(470, true), 188);
  assert.equal(boxLimit(1024, true), 409);
});

test("never below one line, however short the screen", () => {
  assert.equal(boxLimit(60, true), MIN_BOX);
  assert.equal(boxLimit(0, true), MIN_BOX);
});

test("the box takes what its text wants until the limit, then scrolls", () => {
  assert.deepEqual(boxFit(44, 188), { height: 44, scroll: false });
  assert.deepEqual(boxFit(188, 188), { height: 188, scroll: false });
  assert.deepEqual(boxFit(400, 188), { height: 188, scroll: true });
});

test("the visible frame follows the keyboard", () => {
  assert.deepEqual(visibleFrame({ height: 812, offsetTop: 0, scale: 1 }, 812), { height: 812, top: 0 });
  assert.deepEqual(visibleFrame({ height: 470.4, offsetTop: 120.6, scale: 1 }, 812), { height: 470, top: 121 });
});

test("pinch-zoomed or unknown, the whole window counts", () => {
  assert.deepEqual(visibleFrame({ height: 300, offsetTop: 200, scale: 2 }, 812), { height: 812, top: 0 });
  assert.deepEqual(visibleFrame(null, 700), { height: 700, top: 0 });
  assert.deepEqual(visibleFrame(undefined, 700), { height: 700, top: 0 });
  assert.deepEqual(visibleFrame({ height: 0, offsetTop: 0, scale: 1 }, 700), { height: 700, top: 0 });
});

test("a negative offset (overscroll bounce) is held at the top", () => {
  assert.deepEqual(visibleFrame({ height: 800, offsetTop: -12, scale: 1 }, 812), { height: 800, top: 0 });
});
