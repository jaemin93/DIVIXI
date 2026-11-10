/** Which workers the conversation's foot says are working, and for how long: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { workingFor, workingWorkers, type RunLike } from "./workingLine.ts";

/** The words as keys with their values, so a test reads what was asked for. */
const tr = (key: string, vars?: Record<string, string | number>) => (vars ? `${key}(${Object.entries(vars).map(([k, v]) => `${k}=${v}`).join(",")})` : key);

const notWorker = (s: string) => s === "conductor" || s === "artifact";
const run = (session: string, status: string, startedAt: number, track = "t1"): RunLike => ({ track, session, status, startedAt });

test("only this track's workers in a turn, the longest-running first", () => {
  const runs = [
    run("conductor", "running", 50),
    run("fix-parser", "done", 10),
    run("fix-parser", "running", 300),
    run("docs", "connecting", 200),
    run("ui", "running", 100, "t2"),
    run("artifact", "running", 60),
    run("review", "failed", 400),
  ];
  assert.deepEqual(workingWorkers(runs, undefined, "t1", notWorker), [
    { name: "docs", since: 200 },
    { name: "fix-parser", since: 300 },
  ]);
});

test("a session in a turn counts though no run of it is live here, with no time to give", () => {
  const got = workingWorkers([run("a", "running", 100)], [{ name: "b", running: true }, { name: "c", running: false }, { name: "a", running: true }], "t1", notWorker);
  assert.deepEqual(got, [
    { name: "a", since: 100 },
    { name: "b", since: null },
  ]);
});

test("two live runs of one worker read as the newer one's turn", () => {
  assert.deepEqual(workingWorkers([run("a", "running", 100), run("a", "running", 900)], undefined, "t1", notWorker), [{ name: "a", since: 900 }]);
});

test("the time reads in whole minutes, then hours and minutes", () => {
  const MIN = 60_000;
  assert.equal(workingFor(null, 0, tr), "timeline.workingNow");
  assert.equal(workingFor(1000, 1000 + 59_000, tr), "timeline.workingJust");
  // The page's clock can be a little behind a turn that has just begun.
  assert.equal(workingFor(5000, 1000, tr), "timeline.workingJust");
  assert.equal(workingFor(0, 7 * MIN + 30_000, tr), "timeline.workingFor(for=schedules.minutes(n=7))");
  assert.equal(workingFor(0, 60 * MIN, tr), "timeline.workingFor(for=schedules.hours(n=1))");
  assert.equal(workingFor(0, 135 * MIN, tr), "timeline.workingFor(for=schedules.hours(n=2) schedules.minutes(n=15))");
});
