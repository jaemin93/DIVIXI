/** The switch for the check at startup: `node --test`. When a start checks is tested in src-tauri/src/update.rs. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { autoFrom, autoTo } from "./updateSchedule.ts";

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
