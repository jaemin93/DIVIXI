/** What the schedules popover says about each watched job: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { forPopover, isOpen, jobState, lastMoved, RECENT_MS, watches, when, type JobRow } from "./jobList.ts";

const NOW = new Date(2026, 9, 20, 21, 0).getTime();
const MIN = 60_000;

const job = (id: number, over: Partial<JobRow> = {}): JobRow => ({ id, track: "t", status: "running", line: `#${id}`, worker: "collect", what: "", pid: null, log: "", registered_at: NOW - 60 * MIN, changed_at: NOW - 60 * MIN, ended_at: null, note: "", ...over });

test("running and stalled are open; the rest are over", () => {
  assert.equal(isOpen(job(1)), true);
  assert.equal(isOpen(job(2, { status: "stalled" })), true);
  for (const status of ["done", "died", "lost"]) assert.equal(isOpen(job(3, { status })), false);
});

test("a lost job is unwatched when somebody stopped watching it, lost otherwise", () => {
  assert.equal(jobState(job(1, { status: "lost", note: "no longer watched" })), "unwatched");
  assert.equal(jobState(job(2, { status: "lost", note: "지켜보기를 그만둠" })), "unwatched");
  assert.equal(jobState(job(3, { status: "lost", note: "the log is unreadable" })), "lost");
  assert.equal(jobState(job(4, { status: "died" })), "died");
  assert.equal(jobState(job(5, { status: "stalled" })), "stalled");
});

test("open jobs first, newest first; then what ended in the last day, latest first", () => {
  const list = [
    job(1, { registered_at: NOW - 90 * MIN }),
    job(2, { status: "done", ended_at: NOW - 10 * MIN }),
    job(3, { status: "stalled", registered_at: NOW - 30 * MIN }),
    job(4, { status: "died", ended_at: NOW - 5 * MIN }),
    job(5, { status: "died", ended_at: NOW - RECENT_MS - MIN }),
    job(6, { status: "lost", ended_at: null }),
  ];
  assert.deepEqual(forPopover(list, NOW).map((j) => j.id), [3, 1, 4, 2]);
});

test("what it watches, in one line", () => {
  assert.equal(watches(job(1, { what: "nightly collection", pid: 4321, log: "C:\\data\\run\\collect.log" })), "nightly collection · pid 4321 · collect.log");
  assert.equal(watches(job(2, { pid: 99 })), "pid 99");
  assert.equal(watches(job(3, { log: "/srv/logs/build.log" })), "build.log");
});

test("since when it runs, or when it ended", () => {
  assert.equal(when(job(1, { registered_at: NOW - 3 * MIN })), NOW - 3 * MIN);
  assert.equal(when(job(2, { status: "died", ended_at: NOW - MIN })), NOW - MIN);
});

test("the log's last move, only for an open job with a log that has moved", () => {
  assert.equal(lastMoved(job(1, { log: "/x.log", changed_at: NOW - MIN })), NOW - MIN);
  assert.equal(lastMoved(job(2, { log: "/x.log" })), null);
  assert.equal(lastMoved(job(3, { changed_at: NOW - MIN })), null);
  assert.equal(lastMoved(job(4, { status: "done", log: "/x.log", changed_at: NOW - MIN })), null);
});
