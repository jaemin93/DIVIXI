import test from "node:test";
import assert from "node:assert/strict";
import {
  GENERAL,
  GENERAL_NAME,
  MAX_NAME,
  libraryOfTagKey,
  libraryRow,
  libraryTagKey,
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
  { id: GENERAL, name: GENERAL_NAME, created_at: 0, sources: 3, color: "", tags: [] },
  { id: 2, name: "Work notes", created_at: 1, sources: 0, color: "#1a73e8", tags: ["docs", "q4"] },
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

test("a library with documents is not deleted, General as any other", () => {
  assert.equal(deleteBlock(libs[0]), "notEmpty", "General with documents: the same reason as any library");
  assert.equal(deleteBlock({ sources: 0 }), "", "General empty: deleted");
  assert.equal(deleteBlock({ sources: 1 }), "notEmpty");
  assert.equal(deleteBlock(libs[1]), "");
});

test("a library's tags open the one tag dialog by a key of their own", () => {
  assert.equal(libraryTagKey(2), "lib:2");
  assert.equal(libraryOfTagKey("lib:2"), 2);
  for (const other of ["tr001", "ar002", "lib:", "lib:x", "xlib:2"]) assert.equal(libraryOfTagKey(other), null, other);
});

test("a view keeps its library while it exists, and falls back to the first", () => {
  assert.equal(keepLibrary(2, libs), 2);
  assert.equal(keepLibrary(5, libs), GENERAL, "deleted elsewhere: the first, General while it is listed first");
  assert.equal(keepLibrary(null, libs), GENERAL, "first opening, nothing remembered");
  assert.equal(parseLibrary("2"), 2);
  for (const s of ["all", "", null, undefined, "x", "-1", "1.5", "0"]) assert.equal(parseLibrary(s), null, String(s));
});

test("a remembered view of all libraries, from before there was none, opens on General", () => {
  assert.equal(keepLibrary(parseLibrary("all"), libs), GENERAL);
  assert.equal(keepLibrary(parseLibrary("2"), libs), 2);
});

test("General deleted: the first library is shown, and nothing once the last is gone", () => {
  const noGeneral = [{ id: 3 }, { id: 4 }];
  assert.equal(keepLibrary(null, noGeneral), 3);
  assert.equal(keepLibrary(9, noGeneral), 3);
  assert.equal(keepLibrary(4, noGeneral), 4);
  assert.equal(keepLibrary(null, []), null);
  assert.equal(keepLibrary(2, []), null);
  assert.equal(keepLibrary(GENERAL, [{ id: 2 }]), 2, "General just deleted: the next library");
  assert.equal(keepLibrary(GENERAL, []), null, "the last library just deleted: none");
  assert.equal(keepLibrary(parseLibrary("1"), [{ id: 7 }]), 7, "a remembered General that is gone");
});
