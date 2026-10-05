import test from "node:test";
import assert from "node:assert/strict";
import { indexProgress } from "./indexProgress.ts";

const off = { enabled: false, embedded: 0, failed: 0, total: 0, error: "" };
const docs = (...status: string[]) => status.map((s) => ({ status: s }));

test("nothing to say when every document is synced and nothing is embedding", () => {
  assert.equal(indexProgress(docs("synced", "synced"), off), null);
  assert.equal(indexProgress([], off), null);
  assert.equal(indexProgress(docs("synced", "missing", "error"), off), null, "a broken document is not progress");
});

test("syncing counts documents, leaving out the ones that will not index", () => {
  assert.deepEqual(indexProgress(docs("synced", "indexing", "pending"), off), { kind: "sync", done: 1, total: 3 });
  assert.deepEqual(indexProgress(docs("synced", "indexing", "duplicate", "missing"), off), { kind: "sync", done: 1, total: 2 });
  assert.deepEqual(indexProgress(docs("pending"), off), { kind: "sync", done: 0, total: 1 });
});

test("embedding counts passages, once syncing is done", () => {
  const embedding = { enabled: true, embedded: 40, failed: 2, total: 100, error: "" };
  assert.deepEqual(indexProgress(docs("synced"), embedding), { kind: "embed", done: 40, total: 100 });
  assert.equal(indexProgress(docs("synced", "indexing"), embedding)?.kind, "sync", "syncing is reported first");
});

test("embedding that is finished, off, or stopped by an error says nothing", () => {
  assert.equal(indexProgress(docs("synced"), { enabled: true, embedded: 98, failed: 2, total: 100, error: "" }), null, "the refused ones are done too");
  assert.equal(indexProgress(docs("synced"), { enabled: false, embedded: 0, failed: 0, total: 100, error: "" }), null);
  assert.equal(indexProgress(docs("synced"), { enabled: true, embedded: 10, failed: 0, total: 100, error: "401" }), null, "the error banner says it instead");
});
