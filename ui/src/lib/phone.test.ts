import test from "node:test";
import assert from "node:assert/strict";
import { dist, mid, phoneWidth, pinchView, staleBuild, toWorld } from "./phone.ts";

const centre = { x: 200, y: 400 };
const close = (a: number, b: number) => Math.abs(a - b) < 1e-9;

test("a pinch zooms by how far the fingers moved apart, about the point under them", () => {
  const view = { x: 30, y: -20, k: 1 };
  const a = { x: 150, y: 380 }, b = { x: 250, y: 420 };
  const anchor = toWorld(mid(a, b), view, centre);
  // Twice as far apart, the midpoint where it was: twice the zoom, the anchor still under it.
  const a2 = { x: 100, y: 360 }, b2 = { x: 300, y: 440 };
  const v = pinchView(view.k, dist(a, b), dist(a2, b2), anchor, mid(a2, b2), centre, 0.25, 4);
  assert.ok(close(v.k, 2));
  const back = toWorld(mid(a2, b2), v, centre);
  assert.ok(close(back.x, anchor.x) && close(back.y, anchor.y), "the point under the fingers stays there");
  // The fingers also slide: the anchor follows them.
  const slid = { x: mid(a2, b2).x + 50, y: mid(a2, b2).y - 30 };
  const v2 = pinchView(view.k, dist(a, b), dist(a2, b2), anchor, slid, centre, 0.25, 4);
  const there = toWorld(slid, v2, centre);
  assert.ok(close(there.x, anchor.x) && close(there.y, anchor.y));
});

test("the zoom stays within its bounds, and a pinch from no distance does nothing odd", () => {
  const anchor = { x: 0, y: 0 };
  assert.equal(pinchView(1, 10, 1000, anchor, centre, centre, 0.25, 4).k, 4);
  assert.equal(pinchView(1, 1000, 1, anchor, centre, centre, 0.25, 4).k, 0.25);
  assert.equal(pinchView(1.5, 0, 50, anchor, centre, centre, 0.25, 4).k, 1.5);
});

test("a phone's width is Track's breakpoint", () => {
  assert.equal(phoneWidth(390), true);
  assert.equal(phoneWidth(640), true);
  assert.equal(phoneWidth(641), false);
  assert.equal(phoneWidth(0), false, "not measured yet");
});

test("a page reloads when its socket comes back to another build", () => {
  assert.equal(staleBuild("abc", "abd"), true);
  assert.equal(staleBuild("abc", "abc"), false);
  assert.equal(staleBuild(null, "abc"), false, "the first build seen is the page's own");
  assert.equal(staleBuild("abc", undefined), false, "an older server says none");
});
