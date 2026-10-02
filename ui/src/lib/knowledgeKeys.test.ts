/** Which knowledge settings the pane reads and writes, here and from another device: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { DESCRIBE_KEYS, EMBED_KEYS, embedSettable, paneKeys } from "./knowledgeKeys.ts";

/** What `setting_closed` in remote/bridge.rs refuses another device. */
const closedRemotely = (key: string) => key.startsWith("knowledge.embed") || key.includes("key");

test("from another device the pane touches no embedding setting", () => {
  assert.equal(embedSettable(false), false);
  assert.deepEqual(paneKeys(false), [...DESCRIBE_KEYS]);
  for (const key of paneKeys(false)) assert.ok(!closedRemotely(key), key);
});

test("on this PC the pane reads every one of them", () => {
  assert.equal(embedSettable(true), true);
  assert.deepEqual(paneKeys(true), [...DESCRIBE_KEYS, ...EMBED_KEYS]);
});

test("every embedding key is one the bridge closes, the key itself included", () => {
  assert.equal(EMBED_KEYS.length, 6);
  for (const key of EMBED_KEYS) assert.ok(key.startsWith("knowledge.embed."), key);
  assert.ok(EMBED_KEYS.includes("knowledge.embed.key"));
});
