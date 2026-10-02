/** Which log level the settings page spells out: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { customFilter, LEVEL_PRESETS } from "./logLevel.ts";

test("a preset is shown by its button, so no filter is spelled out", () => {
  for (const preset of LEVEL_PRESETS) {
    assert.equal(customFilter({ filter: "warn", preset, source: "setting" }), null, preset);
  }
});

test("a filter no preset matches is spelled out as it is", () => {
  const filter = "warn,orchestra_app=trace";
  assert.equal(customFilter({ filter, preset: "custom", source: "env" }), filter);
});

test("before the level is read, nothing is shown", () => {
  assert.equal(customFilter(null), null);
});
