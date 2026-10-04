/** Whether the track list's filter is on, and what it lets through: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { anyFilterOn, CLEARED, hasAllTags, passesFilter, toggleTag, type FilterRow } from "./trackFilter.ts";
import type { TrackFilter } from "./store.svelte";

const none: TrackFilter = { running: false, active: false, recent: "", sort: "recent", fold: 7, tags: [] };
const HOUR = 60 * 60 * 1000;
const row = (over: Partial<FilterRow> = {}): FilterRow => ({ live: false, active: false, lastAt: 0, tags: [], ...over });

test("nothing on is off; sort and fold do not turn it on", () => {
  assert.equal(anyFilterOn(none), false);
  assert.equal(anyFilterOn({ ...none, sort: "az", fold: 0 }), false);
});

test("running, active, a recent window or one tag each turn it on", () => {
  assert.equal(anyFilterOn({ ...none, running: true }), true);
  assert.equal(anyFilterOn({ ...none, active: true }), true);
  assert.equal(anyFilterOn({ ...none, recent: "24h" }), true);
  assert.equal(anyFilterOn({ ...none, tags: ["ui"] }), true);
});

test("clearing turns every filter off and keeps sort and fold", () => {
  const f: TrackFilter = { running: true, active: true, recent: "1h", sort: "za", fold: 2, tags: ["ui"] };
  const cleared = { ...f, ...CLEARED };
  assert.equal(anyFilterOn(cleared), false);
  assert.equal(cleared.sort, "za");
  assert.equal(cleared.fold, 2);
});

test("picked tags are AND-ed: a track needs every one", () => {
  assert.equal(hasAllTags(["ui", "bug"], ["ui", "bug"]), true);
  assert.equal(hasAllTags(["ui", "bug", "p1"], ["ui", "bug"]), true);
  assert.equal(hasAllTags(["ui"], ["ui", "bug"]), false);
  assert.equal(hasAllTags(["bug"], ["ui", "bug"]), false);
  assert.equal(hasAllTags([], []), true);
});

test("a tag toggles in and out, the rest keep their order", () => {
  assert.deepEqual(toggleTag([], "ui"), ["ui"]);
  assert.deepEqual(toggleTag(["ui", "bug"], "p1"), ["ui", "bug", "p1"]);
  assert.deepEqual(toggleTag(["ui", "bug", "p1"], "bug"), ["ui", "p1"]);
});

test("running and active each narrow to the tracks that are so", () => {
  assert.equal(passesFilter(row(), { ...none, running: true }, 0, 0), false);
  assert.equal(passesFilter(row({ live: true }), { ...none, running: true }, 0, 0), true);
  assert.equal(passesFilter(row({ live: true }), { ...none, running: true, active: true }, 0, 0), false);
  assert.equal(passesFilter(row({ live: true, active: true }), { ...none, running: true, active: true }, 0, 0), true);
});

test("recent keeps tracks that moved inside the window", () => {
  const f = { ...none, recent: "1h" as const };
  assert.equal(passesFilter(row({ lastAt: 10 * HOUR - 30 * 60 * 1000 }), f, 10 * HOUR, HOUR), true);
  assert.equal(passesFilter(row({ lastAt: 8 * HOUR }), f, 10 * HOUR, HOUR), false);
});

test("the tag filter in passesFilter is the same AND", () => {
  const f = { ...none, tags: ["ui", "bug"] };
  assert.equal(passesFilter(row({ tags: ["ui"] }), f, 0, 0), false);
  assert.equal(passesFilter(row({ tags: ["bug", "ui"] }), f, 0, 0), true);
});
