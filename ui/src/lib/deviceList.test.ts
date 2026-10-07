import { test } from "node:test";
import assert from "node:assert/strict";

import { RECENT, recentDevices } from "./deviceList.ts";

const dev = (id: string, last_seen: number) => ({ id, last_seen });

test("the list shows the devices seen most lately, newest first", () => {
  const devices = [dev("a", 10), dev("b", 70), dev("c", 30), dev("d", 50), dev("e", 20), dev("f", 60), dev("g", 40)];
  const { shown, hidden } = recentDevices(devices, false);
  assert.deepEqual(shown.map((d) => d.id), ["b", "f", "d", "g", "c"]);
  assert.equal(hidden, 2);
  assert.equal(shown.length, RECENT);
});

test("show all gives every device, still newest first", () => {
  const devices = [dev("a", 10), dev("b", 70), dev("c", 30), dev("d", 50), dev("e", 20), dev("f", 60)];
  const { shown, hidden } = recentDevices(devices, true);
  assert.deepEqual(shown.map((d) => d.id), ["b", "f", "d", "c", "e", "a"]);
  assert.equal(hidden, 0);
});

test("a short list has nothing to show more of", () => {
  assert.deepEqual(recentDevices([dev("a", 1), dev("b", 2)], false), { shown: [dev("b", 2), dev("a", 1)], hidden: 0 });
  assert.equal(recentDevices(Array.from({ length: RECENT }, (_, i) => dev(String(i), i)), false).hidden, 0, "exactly the limit: no button");
  assert.deepEqual(recentDevices([], false), { shown: [], hidden: 0 });
});

test("the list it was given is left as it was", () => {
  const devices = [dev("a", 1), dev("b", 2)];
  recentDevices(devices, false);
  assert.deepEqual(devices.map((d) => d.id), ["a", "b"]);
});
