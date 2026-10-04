/** Startup phases: what a fresh webview shows before its data is in. `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { DEADLINE_MS, SLOW_MS, empty, initial, next, slow, waiting, type Startup, type StartupEvent } from "./startup.ts";

/** Run events through from the start. */
function run(remote: boolean, ...events: StartupEvent[]): Startup {
  return events.reduce((s, e) => next(s, e, remote), initial(remote));
}

test("a remote instance starts connecting, and this PC starts loading", () => {
  assert.equal(initial(true).phase, "connecting");
  assert.equal(initial(false).phase, "loading");
});

test("connected, then loaded: ready", () => {
  const s = run(true, { type: "connected" });
  assert.equal(s.phase, "loading");
  assert.equal(run(true, { type: "connected" }, { type: "loaded", ok: true }).phase, "ready");
  assert.equal(run(false, { type: "loaded", ok: true }).phase, "ready");
});

test("no tracks while connecting or loading is not the empty state", () => {
  assert.equal(empty("connecting", 0), false);
  assert.equal(empty("loading", 0), false);
  assert.equal(empty("failed", 0), false);
  assert.equal(empty("ready", 0), true);
  assert.equal(empty("ready", 2), false);
});

test("waiting covers connecting and loading only", () => {
  assert.equal(waiting("connecting"), true);
  assert.equal(waiting("loading"), true);
  assert.equal(waiting("ready"), false);
  assert.equal(waiting("failed"), false);
});

test("a remote that cannot be reached fails with the reason", () => {
  const s = run(true, { type: "failed", error: "connection refused" });
  assert.deepEqual(s, { phase: "failed", error: "connection refused", late: false });
});

test("a remote whose data cannot be read fails rather than looking empty", () => {
  const s = run(true, { type: "connected" }, { type: "loaded", ok: false, error: "list_tracks: timed out" });
  assert.equal(s.phase, "failed");
  assert.equal(s.error, "list_tracks: timed out");
});

test("this PC's failed read still shows the app (its error is a toast)", () => {
  assert.equal(run(false, { type: "loaded", ok: false, error: "boom" }).phase, "ready");
});

test("the deadline stops a remote's spinner with a reason", () => {
  const s = run(true, { type: "deadline", error: "took too long" });
  assert.deepEqual(s, { phase: "failed", error: "took too long", late: true });
  assert.equal(waiting(s.phase), false);
  // Also while loading.
  assert.equal(run(true, { type: "connected" }, { type: "deadline", error: "x" }).phase, "failed");
});

test("the deadline on this PC shows what there is", () => {
  assert.equal(run(false, { type: "deadline", error: "x" }).phase, "ready");
});

test("an answer after the deadline is let through", () => {
  const s = run(true, { type: "deadline", error: "x" }, { type: "connected" });
  assert.deepEqual(s, { phase: "loading", error: "", late: false });
  assert.equal(next(s, { type: "loaded", ok: true }, true).phase, "ready");
  // Or the read finishing, if the deadline came while loading.
  const t = run(true, { type: "connected" }, { type: "deadline", error: "x" }, { type: "loaded", ok: true });
  assert.equal(t.phase, "ready");
});

test("a real failure after the deadline replaces the timeout's reason", () => {
  const s = run(true, { type: "deadline", error: "x" }, { type: "failed", error: "host key changed" });
  assert.deepEqual(s, { phase: "failed", error: "host key changed", late: false });
});

test("settled states stay settled", () => {
  const ready = run(true, { type: "connected" }, { type: "loaded", ok: true });
  assert.equal(next(ready, { type: "deadline", error: "x" }, true), ready);
  assert.equal(next(ready, { type: "failed", error: "x" }, true), ready);
  const failed = run(true, { type: "failed", error: "refused" });
  assert.equal(next(failed, { type: "connected" }, true), failed);
  assert.equal(next(failed, { type: "deadline", error: "x" }, true), failed);
});

test("a second deadline does not change a late failure", () => {
  const s = run(true, { type: "deadline", error: "first" });
  assert.equal(next(s, { type: "deadline", error: "second" }, true), s);
});

test("slow only while waiting, and only after a while", () => {
  assert.equal(slow("connecting", 0, SLOW_MS - 1), false);
  assert.equal(slow("connecting", 0, SLOW_MS), true);
  assert.equal(slow("loading", 0, SLOW_MS + 5), true);
  assert.equal(slow("ready", 0, SLOW_MS * 10), false);
  assert.ok(SLOW_MS < DEADLINE_MS);
});
