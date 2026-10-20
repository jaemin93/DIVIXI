/** The schedules popover's filter and order: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { clock, DEFAULT_WATCH_FILTER, isFiltered, orderJobs, orderSchedules, parseWatchFilter, visible, WATCH_FILTER_KEY } from "./watchFilter.ts";
import { LAYOUT_SETTINGS } from "./layoutSettings.ts";
import type { ScheduleRow } from "./scheduleList.ts";
import type { JobRow } from "./jobList.ts";

const NOW = new Date(2026, 9, 20, 21, 0).getTime();
const MIN = 60_000;
const tr = (key: string) => key;

const sched = (id: number, over: Partial<ScheduleRow> = {}): ScheduleRow => ({ id, track: "t", status: "active", line: `#${id}`, kind: "at", next_at: null, ...over });
const job = (id: number, over: Partial<JobRow> = {}): JobRow => ({ id, track: "t", status: "running", line: `#${id}`, registered_at: NOW - 60 * MIN, ended_at: null, ...over });

const schedules = [
  sched(1, { next_at: NOW + 120 * MIN }),
  sched(2, { kind: "file", next_at: null }),
  sched(3, { next_at: NOW + 10 * MIN }),
  sched(4, { status: "cancelled", next_at: NOW + 5 * MIN }),
  sched(5, { kind: "process", next_at: null }),
  sched(6, { next_at: NOW + 60 * MIN }),
];

const jobs = [
  job(1, { registered_at: NOW - 90 * MIN }),
  job(2, { status: "done", ended_at: NOW - 30 * MIN }),
  job(3, { status: "stalled", registered_at: NOW - 20 * MIN }),
  job(4, { status: "died", ended_at: NOW - 5 * MIN }),
  job(5, { status: "done", ended_at: NOW - 25 * 60 * MIN }),
];

test("schedules: newest is the soonest firing first, oldest the furthest; conditions after either way", () => {
  assert.deepEqual(orderSchedules(schedules, "newest").map((s) => s.id), [3, 6, 1, 2, 5]);
  assert.deepEqual(orderSchedules(schedules, "oldest").map((s) => s.id), [1, 6, 3, 5, 2]);
});

test("jobs: open above ended; newest is the latest registered or ended first, oldest the earliest; older than a day out", () => {
  assert.deepEqual(orderJobs(jobs, NOW, "newest").map((j) => j.id), [3, 1, 4, 2]);
  assert.deepEqual(orderJobs(jobs, NOW, "oldest").map((j) => j.id), [1, 3, 2, 4]);
});

test("each kind the filter keeps, in its order", () => {
  const all = visible(schedules, jobs, NOW, { kind: "all", order: "newest" });
  assert.deepEqual([all.schedules.map((s) => s.id), all.jobs.map((j) => j.id)], [[3, 6, 1, 2, 5], [3, 1, 4, 2]]);
  const onlySchedules = visible(schedules, jobs, NOW, { kind: "schedules", order: "oldest" });
  assert.deepEqual([onlySchedules.schedules.map((s) => s.id), onlySchedules.jobs.map((j) => j.id)], [[1, 6, 3, 5, 2], []]);
  const onlyJobs = visible(schedules, jobs, NOW, { kind: "jobs", order: "newest" });
  assert.deepEqual([onlyJobs.schedules.map((s) => s.id), onlyJobs.jobs.map((j) => j.id)], [[], [3, 1, 4, 2]]);
});

test("a kept filter is read back field by field; anything else is the default", () => {
  assert.deepEqual(parseWatchFilter('{"kind":"jobs","order":"oldest"}'), { kind: "jobs", order: "oldest" });
  assert.deepEqual(parseWatchFilter('{"kind":"tasks","order":"oldest"}'), { kind: "all", order: "oldest" });
  assert.deepEqual(parseWatchFilter("not json"), DEFAULT_WATCH_FILTER);
  assert.deepEqual(parseWatchFilter(null), DEFAULT_WATCH_FILTER);
  assert.equal(isFiltered(DEFAULT_WATCH_FILTER), false);
  assert.equal(isFiltered({ kind: "all", order: "oldest" }), true);
});

test("the filter is this window's layout: kept by the PC's window, not written from a phone", () => {
  assert.equal((LAYOUT_SETTINGS as readonly string[]).includes(WATCH_FILTER_KEY), true);
});

test("the time column: the time, and the day when it is not today", () => {
  assert.deepEqual(clock(new Date(2026, 9, 20, 23, 5).getTime(), NOW, tr), { day: "", time: "23:05" });
  assert.deepEqual(clock(new Date(2026, 9, 21, 8, 31).getTime(), NOW, tr), { day: "schedules.tomorrowDay", time: "08:31" });
  assert.deepEqual(clock(new Date(2026, 9, 19, 22, 0).getTime(), NOW, tr), { day: "schedules.yesterday", time: "22:00" });
  assert.deepEqual(clock(new Date(2026, 9, 25, 3, 20).getTime(), NOW, tr), { day: "10-25", time: "03:20" });
});
