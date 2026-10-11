import test from "node:test";
import assert from "node:assert/strict";
import { workspaceTarget } from "./workspaceTarget.ts";

test("the file browser shows the folder of the page it is beside", () => {
  assert.equal(workspaceTarget("routines", "ro1", "ar1", "tr1"), "routine:ro1");
  assert.equal(workspaceTarget("routines", "new", "ar1", "tr1"), "", "a routine not saved yet has no folder");
  assert.equal(workspaceTarget("design", "ro1", "ar1", "tr1"), "artifact:ar1");
  assert.equal(workspaceTarget("design", "ro1", "", "tr1"), "");
  for (const view of ["track", "worker", "settings", "knowledge", "new-track", "edit-track"]) {
    assert.equal(workspaceTarget(view, "ro1", "ar1", "tr1"), "tr1", `on ${view}: the track's`);
  }
});
