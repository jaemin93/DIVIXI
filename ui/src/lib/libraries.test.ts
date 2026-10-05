import test from "node:test";
import assert from "node:assert/strict";
import {
  GENERAL,
  GENERAL_NAME,
  MAX_NAME,
  addTarget,
  cleanName,
  deleteBlock,
  displayName,
  keepLibrary,
  libraryPick,
  nameProblem,
  parseLibrary,
  sourcesIn,
} from "./libraries.ts";

const libs = [
  { id: GENERAL, name: GENERAL_NAME, created_at: 0, sources: 3 },
  { id: 2, name: "Work notes", created_at: 1, sources: 0 },
];

test("General is shown in the person's language until it is renamed", () => {
  assert.equal(displayName(libs[0], "일반"), "일반");
  assert.equal(displayName({ id: GENERAL, name: "Everything" }, "일반"), "Everything", "renamed: theirs");
  assert.equal(displayName({ id: 2, name: GENERAL_NAME }, "일반"), GENERAL_NAME, "only the General library");
});

test("a view shows one library's documents, or all of them", () => {
  const docs = [{ id: "a", library_id: 1 }, { id: "b", library_id: 2 }, { id: "c", library_id: 1 }];
  assert.deepEqual(sourcesIn(docs, 1).map((d) => d.id), ["a", "c"]);
  assert.deepEqual(sourcesIn(docs, 2).map((d) => d.id), ["b"]);
  assert.equal(sourcesIn(docs, null).length, 3);
  assert.deepEqual(sourcesIn(docs, 9), []);
});

test("names are checked as the core checks them", () => {
  assert.equal(cleanName("  Work   notes "), "Work notes");
  assert.equal(nameProblem("Papers", libs), "");
  assert.equal(nameProblem("   ", libs), "empty");
  assert.equal(nameProblem("x".repeat(MAX_NAME + 1), libs), "long");
  assert.equal(nameProblem("x".repeat(MAX_NAME), libs), "");
  assert.equal(nameProblem("가".repeat(MAX_NAME), libs), "", "characters, not bytes");
  assert.equal(nameProblem("work  NOTES", libs), "taken", "case and spacing aside");
  assert.equal(nameProblem("general", libs), "taken", "General's stored name too");
  assert.equal(nameProblem("Work Notes", libs, 2), "", "a library may keep its own name in another case");
});

test("General is never deleted, nor a library with documents", () => {
  assert.equal(deleteBlock(libs[0]), "general");
  assert.equal(deleteBlock({ id: GENERAL, sources: 0 }), "general", "empty or not");
  assert.equal(deleteBlock({ id: 2, sources: 1 }), "notEmpty");
  assert.equal(deleteBlock(libs[1]), "");
});

test("a document added from the view of all goes to General", () => {
  assert.equal(addTarget(null), GENERAL);
  assert.equal(addTarget(2), 2);
});

test("the graph conversation is told the library on screen, and nothing for all", () => {
  assert.deepEqual(libraryPick(2), ["l:2"]);
  assert.deepEqual(libraryPick(null), []);
});

test("a view keeps its library while it exists, and falls back to all", () => {
  assert.equal(keepLibrary(2, libs), 2);
  assert.equal(keepLibrary(5, libs), null, "deleted elsewhere");
  assert.equal(keepLibrary(null, libs), null);
  assert.equal(parseLibrary("2"), 2);
  for (const s of ["all", "", null, undefined, "x", "-1", "1.5", "0"]) assert.equal(parseLibrary(s), null, String(s));
});
