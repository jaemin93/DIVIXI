/** What the schedules popover and the conversation's schedule lines say: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { firstLine, inConversation, justEnded, lateness, lineOf, message, nextLabel, rule, span, stamp, upcoming, type ScheduleRow } from "./scheduleList.ts";

/** The words as keys with their values, so a test reads what was asked for. */
const tr = (key: string, vars?: Record<string, string | number>) => (vars ? `${key}(${Object.entries(vars).map(([k, v]) => `${k}=${v}`).join(",")})` : key);

const at = (y: number, mo: number, d: number, h: number, mi: number) => new Date(y, mo - 1, d, h, mi).getTime();
const NOW = at(2026, 10, 20, 21, 0);

const row = (id: number, over: Partial<ScheduleRow> = {}): ScheduleRow => ({ id, track: "t", status: "active", line: `#${id} x: do it`, kind: "at", spec: {}, message: "do it", next_at: null, note: "", ...over });

test("a time reads as the core writes it", () => {
  assert.equal(stamp(at(2026, 10, 21, 8, 31)), "10-21 08:31");
});

test("the next firing is a time today, tomorrow said, a date after", () => {
  assert.equal(nextLabel(at(2026, 10, 20, 23, 5), NOW, tr), "23:05");
  assert.equal(nextLabel(at(2026, 10, 21, 8, 31), NOW, tr), "schedules.tomorrow(time=08:31)");
  assert.equal(nextLabel(at(2026, 10, 23, 9, 0), NOW, tr), "10-23 09:00");
});

test("spans read back in their biggest whole unit", () => {
  assert.equal(span(7200, tr), "schedules.hours(n=2)");
  assert.equal(span(86_400, tr), "schedules.days(n=1)");
  assert.equal(span(1800, tr), "schedules.minutes(n=30)");
  assert.equal(span(90, tr), "schedules.seconds(n=90)");
});

test("each kind says its rule; a one-off time has none", () => {
  assert.equal(rule(row(1, { kind: "at", spec: { at: NOW } }), tr, "en-US"), null);
  assert.equal(rule(row(2, { kind: "every", spec: { every: 7200 } }), tr, "en-US"), "schedules.every(span=schedules.hours(n=2))");
  assert.equal(rule(row(3, { kind: "every", spec: { every: 7200, until: at(2026, 10, 21, 18, 0) } }), tr, "en-US"), "schedules.everyUntil(span=schedules.hours(n=2),until=10-21 18:00)");
  assert.equal(rule(row(4, { kind: "daily", spec: { daily: "09:15" } }), tr, "en-US"), "schedules.daily(time=09:15)");
  assert.equal(rule(row(5, { kind: "weekly", spec: { weekly: "sun 03:20" } }), tr, "en-US"), "schedules.weekly(day=Sun,time=03:20)");
  assert.equal(rule(row(6, { kind: "file", spec: { file: "C:\\logs\\build.log", contains: "DONE", pattern: "" } }), tr, "en-US"), "schedules.file(file=build.log,what=DONE)");
  assert.equal(rule(row(7, { kind: "file", spec: { file: "/var/log/x.log", contains: "", pattern: "ok|fail" } }), tr, "en-US"), "schedules.file(file=x.log,what=ok|fail)");
  assert.equal(rule(row(8, { kind: "process", spec: { pid: 4321 } }), tr, "en-US"), "schedules.process(pid=4321)");
});

test("soonest first, conditions after, ended ones out", () => {
  const list = [
    row(1, { next_at: at(2026, 10, 22, 9, 0) }),
    row(2, { kind: "process", next_at: null }),
    row(3, { next_at: at(2026, 10, 20, 22, 0) }),
    row(4, { status: "cancelled", next_at: at(2026, 10, 20, 21, 30) }),
    row(5, { kind: "file", next_at: null }),
    row(6, { status: "done", next_at: null }),
  ];
  assert.deepEqual(upcoming(list).map((s) => s.id), [3, 1, 2, 5]);
});

test("late: overdue past the tick's slack, or a firing that said it was late", () => {
  assert.equal(lateness(row(1, { next_at: NOW - 60_000 }), NOW), null);
  assert.equal(lateness(row(2, { next_at: NOW - 5 * 60_000 }), NOW), "overdue");
  assert.equal(lateness(row(3, { next_at: NOW + 60_000, note: "late: 5m past 10-20 08:00 (the app or the machine was off)" }), NOW), "fired-late");
  assert.equal(lateness(row(4, { next_at: NOW + 60_000, note: "늦게 실행됨: 예정 10-20 08:00에서 5m 지남 (앱이나 기계가 꺼져 있었음)" }), NOW), "fired-late");
  assert.equal(lateness(row(5, { next_at: NOW + 60_000, note: "moved to 10-21 07:00 past quiet hours (22:00-07:00)" }), NOW), null);
  // Cancelled with its time behind it is not late: it is not coming.
  assert.equal(lateness(row(6, { status: "cancelled", next_at: NOW - 5 * 60_000 }), NOW), null);
});

test("the instruction is the message, or the line's tail without one", () => {
  assert.equal(message(row(1, { message: "check the build" })), "check the build");
  assert.equal(message(row(2, { message: undefined, line: "#2 10-21 08:31: look again" })), "look again");
});

test("a line is the instruction's first line with something on it", () => {
  assert.equal(firstLine("\n  Check the build.\nIf it failed, read the log."), "Check the build.");
  assert.equal(firstLine(""), "");
});

test("the conversation keeps a line for what just ended, after the active ones", () => {
  const list = [row(1, { next_at: at(2026, 10, 22, 9, 0) }), row(2, { status: "cancelled" }), row(3, { next_at: at(2026, 10, 21, 9, 0) }), row(4, { status: "done" }), row(5, { status: "done" })];
  assert.deepEqual(inConversation(list, [5, 2]).map((s) => s.id), [3, 1, 2, 5]);
  assert.deepEqual(inConversation(list, []).map((s) => s.id), [3, 1]);
});

test("what ended is what was active before and is not now", () => {
  const before = [row(1), row(2), row(3, { status: "done" })];
  const after = [row(1), row(2, { status: "cancelled" }), row(3, { status: "done" }), row(4)];
  assert.deepEqual(justEnded(before, after), [2]);
});

test("a line: time, rule, first line of the instruction, and its marks", () => {
  const daily = row(1, { kind: "daily", spec: { daily: "08:31" }, next_at: at(2026, 10, 21, 8, 31), message: "Summarise the PRs.\nThen post it." });
  assert.deepEqual(lineOf(daily, NOW, tr, "en-US"), { when: "10-21 08:31", rule: "schedules.daily(time=08:31)", states: [], text: "Summarise the PRs." });

  const late = { ...daily, note: "late: 3m past 10-20 21:03 (the app or the machine was off)" };
  assert.deepEqual(lineOf(late, NOW, tr, "en-US").states, ["late"]);

  const lateOnce = row(2, { status: "done", next_at: null, last_at: at(2026, 10, 20, 20, 50), note: "late: 5m past 10-20 20:45 (the app or the machine was off)" });
  assert.deepEqual(lineOf(lateOnce, NOW, tr, "en-US"), { when: "10-20 20:50", rule: null, states: ["done", "late"], text: "do it" });

  const cancelled = row(3, { status: "cancelled", next_at: at(2026, 10, 21, 9, 0) });
  assert.deepEqual(lineOf(cancelled, NOW, tr, "en-US").states, ["cancelled"]);

  const file = row(4, { kind: "file", spec: { file: "/x/build.log", contains: "DONE" } });
  assert.deepEqual(lineOf(file, NOW, tr, "en-US"), { when: "schedules.file(file=build.log,what=DONE)", rule: null, states: [], text: "do it" });
});
