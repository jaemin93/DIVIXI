import test from "node:test";
import assert from "node:assert/strict";
import { hiddenTabs, scrollToShow, wheelToScroll } from "./tabStrip.ts";

const view = (scrollLeft: number, width = 300) => ({ scrollLeft, width });

test("a tab already in view does not move the strip", () => {
  assert.equal(scrollToShow({ left: 100, width: 96 }, view(0)), null);
  assert.equal(scrollToShow({ left: 0, width: 96 }, view(0)), null, "the first tab at the very edge");
  assert.equal(scrollToShow({ left: 204, width: 96 }, view(0)), null, "the last pixel still counts");
});

test("a tab off to the right scrolls in until its right edge shows", () => {
  assert.equal(scrollToShow({ left: 500, width: 96 }, view(0)), 296);
  assert.equal(scrollToShow({ left: 250, width: 96 }, view(0)), 46, "partly cut off");
  assert.equal(scrollToShow({ left: 500, width: 96 }, view(0), 12), 308, "with a margin past it");
});

test("a tab off to the left scrolls in until its left edge shows", () => {
  assert.equal(scrollToShow({ left: 100, width: 96 }, view(400)), 100);
  assert.equal(scrollToShow({ left: 100, width: 96 }, view(400), 12), 88);
  assert.equal(scrollToShow({ left: 4, width: 96 }, view(400), 12), 0, "never before the start");
});

test("a tab wider than the view shows from its left edge", () => {
  assert.equal(scrollToShow({ left: 600, width: 400 }, view(0)), 600);
  assert.equal(scrollToShow({ left: 600, width: 400 }, view(600)), null);
});

test("hidden tabs are the ones not wholly on screen", () => {
  const tabs = [0, 100, 200, 300, 400].map((left) => ({ left, width: 100 }));
  assert.deepEqual(hiddenTabs(tabs, view(0, 300)), [3, 4]);
  assert.deepEqual(hiddenTabs(tabs, view(150, 300)), [0, 1, 4], "cut at both ends");
  assert.deepEqual(hiddenTabs(tabs, view(0, 500)), [], "all fit");
  assert.deepEqual(hiddenTabs(tabs, view(0.6, 499.5)), [], "sub-pixel rounding is not hiding");
  assert.deepEqual(hiddenTabs([], view(0)), []);
});

test("a plain wheel scrolls the strip sideways, only when it overflows", () => {
  const wheel = (deltaY: number, deltaMode = 0, deltaX = 0) => ({ deltaX, deltaY, deltaMode });
  assert.equal(wheelToScroll(wheel(120), 300, true), 120);
  assert.equal(wheelToScroll(wheel(-3, 1), 300, true), -48, "lines");
  assert.equal(wheelToScroll(wheel(1, 2), 300, true), 300, "pages");
  assert.equal(wheelToScroll(wheel(120), 300, false), null, "nothing hidden");
  assert.equal(wheelToScroll(wheel(0, 0, 40), 300, true), null, "a sideways wheel scrolls by itself");
  assert.equal(wheelToScroll(wheel(0), 300, true), null);
});
