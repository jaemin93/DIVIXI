/** When the app may check for a release on its own: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { autoFrom, autoTo, CHECK_EVERY_MS, dueForCheck, lastCheckFrom, lastCheckTo } from "./updateSchedule.ts";

/** A fixed "now", so none of this depends on when it is run. */
const NOW = Date.UTC(2026, 8, 30, 12, 0, 0);
const HOUR = 60 * 60 * 1000;

/** The startup case: the switch on, and a stamp `hours` ago. */
const atStartup = (hours: number | null) => dueForCheck({ manual: false, auto: true, last: hours === null ? null : NOW - hours * HOUR, now: NOW });

test("checked 23 hours ago: the day's one request is spent, so it is skipped", () => {
  assert.deepEqual(atStartup(23), { check: false, why: "recent" });
  // And right up to the boundary.
  assert.deepEqual(dueForCheck({ manual: false, auto: true, last: NOW - CHECK_EVERY_MS + 1, now: NOW }), { check: false, why: "recent" });
});

test("checked 25 hours ago: a day has passed, so it checks", () => {
  assert.deepEqual(atStartup(25), { check: true, why: "due" });
  // Exactly a day counts as a day.
  assert.deepEqual(dueForCheck({ manual: false, auto: true, last: NOW - CHECK_EVERY_MS, now: NOW }), { check: true, why: "due" });
});

test("no record of a check: it checks", () => {
  assert.deepEqual(atStartup(null), { check: true, why: "never" });
});

test("a press checks whatever the clock or the switch say", () => {
  const asked = { check: true, why: "asked" } as const;
  // A minute ago.
  assert.deepEqual(dueForCheck({ manual: true, auto: true, last: NOW - 60_000, now: NOW }), asked);
  // This instant.
  assert.deepEqual(dueForCheck({ manual: true, auto: true, last: NOW, now: NOW }), asked);
  // And with the switch off, which is the case that matters: turning the
  // automatic check off must not take the button away.
  assert.deepEqual(dueForCheck({ manual: true, auto: false, last: NOW - 60_000, now: NOW }), asked);
  assert.deepEqual(dueForCheck({ manual: true, auto: false, last: null, now: NOW }), asked);
});

test("the switch off means nothing happens at startup, however long it has been", () => {
  assert.deepEqual(dueForCheck({ manual: false, auto: false, last: null, now: NOW }), { check: false, why: "off" });
  assert.deepEqual(dueForCheck({ manual: false, auto: false, last: NOW - 400 * HOUR, now: NOW }), { check: false, why: "off" });
});

test("a clock that went backwards does not stop it checking for as long as the error", () => {
  // A stamp from the future: a machine whose clock was wrong, or one that has
  // been put back. Waiting it out would mean no check until it came round.
  assert.deepEqual(dueForCheck({ manual: false, auto: true, last: NOW + 400 * HOUR, now: NOW }), { check: true, why: "due" });
});

test("the switch is on unless it was turned off", () => {
  assert.equal(autoFrom(null), true, "never set");
  assert.equal(autoFrom(undefined), true);
  assert.equal(autoFrom("on"), true);
  assert.equal(autoFrom("off"), false);
  // Something else wrote into it: not a reason to stop checking.
  assert.equal(autoFrom("yes"), true);
  assert.equal(autoFrom(""), true);
  // And it round-trips.
  assert.equal(autoFrom(autoTo(true)), true);
  assert.equal(autoFrom(autoTo(false)), false);
});

test("a stamp that is not a number is no stamp, not 1970", () => {
  assert.equal(lastCheckFrom(null), null);
  assert.equal(lastCheckFrom(undefined), null);
  assert.equal(lastCheckFrom(""), null);
  assert.equal(lastCheckFrom("   "), null);
  assert.equal(lastCheckFrom("yesterday"), null);
  assert.equal(lastCheckFrom("NaN"), null);
  assert.equal(lastCheckFrom("Infinity"), null);
  assert.equal(lastCheckFrom(String(NOW)), NOW);
  assert.equal(lastCheckFrom(lastCheckTo(NOW)), NOW);
  // Unreadable reads as "never checked", which checks — never as a stamp so
  // old that it checks by accident, and never as one so new it stops checking.
  assert.deepEqual(dueForCheck({ manual: false, auto: true, last: lastCheckFrom("nonsense"), now: NOW }), { check: true, why: "never" });
});
