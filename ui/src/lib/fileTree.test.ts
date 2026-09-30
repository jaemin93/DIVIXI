import test from "node:test";
import assert from "node:assert/strict";
import {
  MAX_OPEN_DIRS,
  MAX_REMEMBERED_TRACKS,
  capMemory,
  keyTrack,
  openKey,
  parentOf,
  parseMemory,
  pruneOpen,
  remember,
  treeReply,
  underOpen,
  type WsEntry,
} from "./fileTree.ts";

const dir = (path: string): WsEntry => ({ path, name: path.split("/").at(-1) ?? path, dir: true, size: 0 });
const file = (path: string): WsEntry => ({ path, name: path.split("/").at(-1) ?? path, dir: false, size: 1 });

test("a track with nothing remembered opens shut, one level deep", () => {
  const memory = parseMemory(null);
  const open = new Set(memory[openKey("tr001", "C:/work")] ?? []);
  assert.deepEqual([...open], [], "no folder is open");
  // The listing asked for is the root alone, so only its children come back,
  // and each of those folders draws shut.
  assert.equal(underOpen("gold-stage1", open), true, "a row at the top is shown");
  assert.equal(underOpen("gold-stage1/.venv", open), false, "nothing inside a shut folder is");
});

test("a folder opened comes back open, in the same app and the next", () => {
  const live = new Set(["tr001"]);
  const key = openKey("tr001", "C:/work");
  let memory = remember({}, key, ["gold-stage1"], live);
  // Still open while the app runs.
  assert.deepEqual(memory[key], ["gold-stage1"]);
  // And after it is written out and read back.
  memory = parseMemory(JSON.stringify(memory));
  const open = new Set(memory[key]);
  assert.equal(underOpen("gold-stage1/.venv", open), true, "what is in it is shown again");
  assert.equal(underOpen("gold-stage1/.venv/Lib", open), false, "and no further");
});

test("a folder that is no longer there is dropped without a word", () => {
  const entries = [dir("gold-stage1"), file("gold-stage1/findings.md")];
  // `moved` was open and its folder was read: it is not in the folder, so it goes.
  assert.deepEqual(pruneOpen(["gold-stage1", "gold-stage1/moved"], entries), ["gold-stage1"]);
  // A folder inside one still shut was never looked for, so it stays.
  assert.deepEqual(pruneOpen(["deep/inside"], [dir("deep")]), ["deep/inside"]);
  // What is under a folder that went goes with it.
  assert.deepEqual(pruneOpen(["a", "a/b", "a/b/c"], [dir("z")]), []);
  // A listing that was cut proves nothing, so it drops nothing.
  assert.deepEqual(pruneOpen(["gold-stage1", "gone"], entries, true), ["gold-stage1", "gone"]);
});

test("one track's open folders never show up in another", () => {
  const live = new Set(["tr001", "tr002"]);
  let memory = remember({}, openKey("tr001", "C:/one"), ["src", "src/lib"], live);
  memory = remember(memory, openKey("tr002", "C:/two"), ["docs"], live);
  assert.deepEqual(memory[openKey("tr001", "C:/one")], ["src", "src/lib"]);
  assert.deepEqual(memory[openKey("tr002", "C:/two")], ["docs"]);
  assert.equal(memory[openKey("tr002", "C:/one")], undefined, "not by track alone");
  // The same track sent to another folder starts shut rather than borrowing.
  assert.equal(memory[openKey("tr001", "C:/elsewhere")], undefined);
  assert.equal(keyTrack(openKey("tr001", "C:/one")), "tr001");
});

test("the memory does not grow without end", () => {
  const live = new Set(Array.from({ length: 40 }, (_, i) => `tr${i}`));
  let memory: Record<string, string[]> = {};
  for (let i = 0; i < 40; i++) memory = remember(memory, openKey(`tr${i}`, "/w"), ["a"], live);
  assert.equal(Object.keys(memory).length, MAX_REMEMBERED_TRACKS, "only the last tracks touched are kept");
  assert.ok(memory[openKey("tr39", "/w")], "the one touched last survives");
  assert.equal(memory[openKey("tr0", "/w")], undefined, "the oldest went");

  const many = Array.from({ length: MAX_OPEN_DIRS + 50 }, (_, i) => `d${i}`);
  const one = remember({}, openKey("tr1", "/w"), many, new Set(["tr1"]));
  assert.equal(one[openKey("tr1", "/w")].length, MAX_OPEN_DIRS, "one track's folders are capped too");

  // A track that is gone takes its memory with it, and so does one with
  // nothing left open.
  const stale = { [openKey("gone", "/w")]: ["a"], [openKey("tr1", "/w")]: ["a"] };
  assert.deepEqual(Object.keys(capMemory(stale, new Set(["tr1"]))), [openKey("tr1", "/w")]);
  assert.deepEqual(remember({ [openKey("tr1", "/w")]: ["a"] }, openKey("tr1", "/w"), [], new Set(["tr1"])), {});
});

test("a stored memory that makes no sense is no memory", () => {
  assert.deepEqual(parseMemory(null), {});
  assert.deepEqual(parseMemory("not json"), {});
  assert.deepEqual(parseMemory("[1,2]"), {});
  assert.deepEqual(parseMemory('{"k": "nope"}'), {});
  assert.deepEqual(parseMemory('{"k": ["a", 7, "", "b"]}'), { k: ["a", "b"] });
});

test("an older Divixi answering with the entries alone is understood", () => {
  const entries = [dir("src"), file("src/main.rs")];
  assert.deepEqual(treeReply([entries, true]), [entries, true]);
  assert.deepEqual(treeReply([entries, false]), [entries, false]);
  // What an older remote instance sends: the list, no word about a cut.
  assert.deepEqual(treeReply(entries), [entries, false]);
  assert.deepEqual(treeReply([]), [[], false]);
  assert.deepEqual(treeReply(null), [[], false]);
});

test("a path's folder is the part before its last slash", () => {
  assert.equal(parentOf("a"), "");
  assert.equal(parentOf("a/b"), "a");
  assert.equal(parentOf("a/b/c.txt"), "a/b");
});
