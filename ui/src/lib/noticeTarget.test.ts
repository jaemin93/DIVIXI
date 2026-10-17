/** What a core notice opens: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { routineOf } from "./noticeTarget.ts";

test("a cut routine run's notice names its routine", () => {
  assert.equal(routineOf({ routine: "ro001", run: "rr1" }), "ro001");
});

test("anything else names none", () => {
  for (const data of [null, undefined, "ro001", 7, {}, { runs: ["t1"] }, { routine: "" }, { routine: 3 }]) {
    assert.equal(routineOf(data), null, JSON.stringify(data));
  }
});
