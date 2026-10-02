/** The track list as `track_saved` keeps it, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { withTrack } from "./tracks.ts";

test("a track told of twice (the reply and the event) is listed once", () => {
  const once = withTrack([], { id: "a", name: "x" });
  assert.deepEqual(withTrack(once, { id: "a", name: "x" }), [{ id: "a", name: "x" }]);
  assert.deepEqual(withTrack(once, { id: "b", name: "y" }).map((t) => t.id), ["a", "b"]);
});

test("a changed track takes its own place", () => {
  const list = [
    { id: "a", name: "x" },
    { id: "b", name: "y" },
  ];
  assert.deepEqual(withTrack(list, { id: "a", name: "renamed" }), [
    { id: "a", name: "renamed" },
    { id: "b", name: "y" },
  ]);
});
