import test from "node:test";
import assert from "node:assert/strict";
import { DRAWER_MIN, DRAWER_RESET, clampDrawer, drawerFromSettings, drawerSettings, pickStep } from "./planDrawer.ts";

test("picking a step does not open a closed drawer", () => {
  const closed = { selected: [], picked: "", open: false };
  const p = pickStep(closed, "s2");
  assert.deepEqual(p, { selected: ["s2"], picked: "s2", open: false });
  assert.equal(pickStep(p, "s3", true).open, false, "nor does adding one to the selection");
});

test("an open drawer stays open and follows the step picked", () => {
  const open = { selected: ["s1"], picked: "s1", open: true };
  assert.deepEqual(pickStep(open, "s2"), { selected: ["s2"], picked: "s2", open: true });
  assert.deepEqual(pickStep(open, ""), { selected: [], picked: "", open: true }, "picking nothing leaves it open, empty");
});

test("the drawer's open state and height are kept and read back", () => {
  for (const d of [{ open: true, height: 340 }, { open: false, height: 200 }]) {
    const kept = drawerSettings(d);
    assert.deepEqual(drawerFromSettings(kept.plandrawer, kept.plandrawer_height), d);
  }
});

test("never kept: closed, at the reset height", () => {
  assert.deepEqual(drawerFromSettings(null, null), { open: false, height: DRAWER_RESET });
  assert.deepEqual(drawerFromSettings("closed", "nonsense"), { open: false, height: DRAWER_RESET });
});

test("its height stays between the minimum and what leaves the graph room", () => {
  assert.equal(clampDrawer(10, 900), DRAWER_MIN);
  assert.equal(clampDrawer(5000, 900), 900 - 260);
  assert.equal(clampDrawer(300.4, 900), 300);
});
