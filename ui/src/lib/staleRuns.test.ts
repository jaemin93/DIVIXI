/** Which worker runs the page should check against the store, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { notInFlight } from "./staleRuns.ts";

const run = (id: string, status: string, session = "ui", track = "tr001") => ({ id, track, session, status });
const ids = (runs: { id: string }[]) => runs.map((r) => r.id);

test("a run working here and in flight in the core is left alone", () => {
  assert.deepEqual(notInFlight([run("t1", "running")], { tr001: [{ name: "ui", run: "t1" }] }), []);
  assert.deepEqual(notInFlight([run("t1", "connecting")], { tr001: [{ name: "ui", run: "t1" }] }), []);
});

test("a run working here whose worker has no session in the core is checked", () => {
  assert.deepEqual(ids(notInFlight([run("t1", "running")], {})), ["t1"]);
  assert.deepEqual(ids(notInFlight([run("t1", "running")], { tr001: [{ name: "api", run: "t2" }] })), ["t1"]);
});

test("a run working here whose worker is idle, or on a later turn, is checked", () => {
  assert.deepEqual(ids(notInFlight([run("t1", "running")], { tr001: [{ name: "ui", run: null }] })), ["t1"]);
  assert.deepEqual(ids(notInFlight([run("t1", "running"), run("t5", "running")], { tr001: [{ name: "ui", run: "t5" }] })), ["t1"]);
});

test("a run that ended here is not checked, whatever the core says", () => {
  assert.deepEqual(notInFlight([run("t1", "done"), run("t2", "failed")], {}), []);
});

test("a worker of the same name on another track does not vouch for this one", () => {
  assert.deepEqual(ids(notInFlight([run("t1", "running", "ui", "tr001")], { tr002: [{ name: "ui", run: "t1" }] })), ["t1"]);
});
