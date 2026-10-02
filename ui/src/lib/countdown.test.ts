/** The pairing code's countdown: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { clock, codeState, secondsLeft, spends } from "./countdown.ts";

/** A fixed "now", in unix seconds, so none of this depends on when it is run. */
const NOW = 1_790_000_000;

test("a fresh code reads 5:00 and goes down a second at a time", () => {
  const expires = NOW + 300;
  assert.equal(clock(secondsLeft(expires, NOW)), "5:00");
  assert.equal(clock(secondsLeft(expires, NOW + 1)), "4:59");
  assert.equal(clock(secondsLeft(expires, NOW + 2)), "4:58");
  assert.equal(clock(secondsLeft(expires, NOW + 60)), "4:00");
  assert.equal(clock(secondsLeft(expires, NOW + 291)), "0:09");
  assert.equal(clock(secondsLeft(expires, NOW + 299)), "0:01");
});

test("at and past the expiry it is zero, not negative", () => {
  assert.equal(secondsLeft(NOW, NOW), 0);
  assert.equal(secondsLeft(NOW, NOW + 5), 0);
  assert.equal(clock(secondsLeft(NOW, NOW + 5)), "0:00");
});

test("a code is live until its last second, then expired; none without one", () => {
  const expires = NOW + 300;
  assert.equal(codeState(null, NOW), "none");
  assert.equal(codeState(expires, NOW), "live");
  assert.equal(codeState(expires, NOW + 299), "live");
  assert.equal(codeState(expires, NOW + 300), "expired");
  // It stays expired: nothing makes a new one on its own.
  assert.equal(codeState(expires, NOW + 3600), "expired");
});

test("a code a device signed in with is used, not live, however much time is left", () => {
  const expires = NOW + 300;
  assert.equal(codeState(expires, NOW + 10, true), "used");
  // Used stays used past the expiry: the screen says what happened to it.
  assert.equal(codeState(expires, NOW + 3600, true), "used");
  assert.equal(codeState(null, NOW, true), "none");
});

test("only the code on screen is spent by a pairing", () => {
  assert.equal(spends("abc", "abc"), true);
  assert.equal(spends("abc", "def"), false, "an older code, or one minted elsewhere");
  assert.equal(spends(null, "abc"), false);
  assert.equal(spends(undefined, "abc"), false);
  assert.equal(spends("", ""), false, "a link without an id spends nothing");
});

test("a new code after an expired one is live again from 5:00", () => {
  const first = NOW + 300;
  const later = NOW + 400;
  assert.equal(codeState(first, later), "expired");
  const second = later + 300;
  assert.equal(codeState(second, later), "live");
  assert.equal(clock(secondsLeft(second, later)), "5:00");
});
