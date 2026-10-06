import test from "node:test";
import assert from "node:assert/strict";
import { matchesQuery, narrow, parseTags } from "./listFilter.ts";

const items = [
  { id: "1", name: "양쪽 비교 보드", tags: ["docs"] },
  { id: "2", name: "Release plan", tags: ["q4", "docs"] },
  { id: "3", name: "일반", tags: [] },
  { id: "4", name: "Garden", tags: ["홈"] },
];

test("the search finds a name or a tag, case aside, Korean too", () => {
  assert.deepEqual(narrow(items, "비교", []).map((i) => i.id), ["1"]);
  assert.deepEqual(narrow(items, "RELEASE", []).map((i) => i.id), ["2"]);
  assert.deepEqual(narrow(items, "docs", []).map((i) => i.id), ["1", "2"], "a tag's name");
  assert.deepEqual(narrow(items, "홈", []).map((i) => i.id), ["4"], "a Korean tag");
  assert.deepEqual(narrow(items, "비교".normalize("NFD"), []).map((i) => i.id), ["1"], "typed in jamo");
  assert.deepEqual(narrow(items, "  ", []).length, 4, "an empty search lets all through");
  assert.equal(matchesQuery(items[2], "없는"), false);
});

test("picked tags are AND-ed, as the tracks column has them, and go with the search", () => {
  assert.deepEqual(narrow(items, "", ["docs"]).map((i) => i.id), ["1", "2"]);
  assert.deepEqual(narrow(items, "", ["docs", "q4"]).map((i) => i.id), ["2"]);
  assert.deepEqual(narrow(items, "plan", ["docs"]).map((i) => i.id), ["2"]);
  assert.deepEqual(narrow(items, "비교", ["q4"]), []);
  assert.deepEqual(narrow(items, "", ["none"]), []);
});

test("a kept tag filter reads back as tag names, anything else as none", () => {
  assert.deepEqual(parseTags('["docs","q4"]'), ["docs", "q4"]);
  assert.deepEqual(parseTags('["docs",3,null]'), ["docs"]);
  for (const s of ["", null, undefined, "{}", "nope", "[", '"docs"']) assert.deepEqual(parseTags(s), [], String(s));
});
