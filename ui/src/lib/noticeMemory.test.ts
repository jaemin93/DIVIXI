/** What keeps a read core notice read: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { addPending, dropPending, parsePending, pendingOf } from "./noticeMemory.ts";

test("a read stays pending per Divixi until its core takes it", () => {
  let p = addPending({}, "inst-a", [7, 9]);
  p = addPending(p, null, [3]);
  p = addPending(p, "inst-a", [7]);
  assert.deepEqual(pendingOf(p, "inst-a"), [7, 9], "the same read twice is one");
  assert.deepEqual(pendingOf(p, null), [3]);
  assert.deepEqual(pendingOf(p, "inst-b"), []);
  p = dropPending(p, "inst-a", [7]);
  assert.deepEqual(pendingOf(p, "inst-a"), [9]);
  p = dropPending(p, "inst-a", [9]);
  assert.deepEqual(Object.keys(p), [""], "an emptied Divixi leaves the list");
  assert.deepEqual(dropPending(p, null, [3]), {});
});

test("a pending list read back keeps only ids", () => {
  const p = addPending(addPending({}, "inst-a", [7]), null, [3]);
  assert.deepEqual(parsePending(JSON.stringify(p)), p);
  for (const raw of [null, "", "nope", "[1,2]", "7", '{"a":"x"}']) assert.deepEqual(parsePending(raw), {}, String(raw));
  assert.deepEqual(parsePending('{"a":[1,"2",3],"b":[]}'), { a: [1, 3] });
});
