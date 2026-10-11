import test from "node:test";
import assert from "node:assert/strict";
import { colorOrNone } from "./cssColor.ts";

test("only #rrggbb goes into a style", () => {
  assert.equal(colorOrNone("#7aa2f7"), "#7aa2f7");
  assert.equal(colorOrNone("#7AA2F7"), "#7AA2F7");
  for (const bad of ["", null, undefined, "red", "#fff", "#7aa2f7; background: url(x)", "url(x)"]) {
    assert.equal(colorOrNone(bad), "transparent", String(bad));
  }
});
