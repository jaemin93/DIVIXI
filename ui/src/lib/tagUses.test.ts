import test from "node:test";
import assert from "node:assert/strict";
import { tagUses } from "./tagUses.ts";

const things = {
  tracks: [{ tags: ["docs"] }, { tags: [] }, { tags: ["docs", "q4"] }],
  artifacts: [
    { kind: "design", tags: ["docs"] },
    { kind: "knowledge", tags: ["docs", "q4"] },
    { kind: "graph", tags: ["docs"] },
  ],
  libraries: [{ tags: ["q4"] }, { tags: [] }],
};

test("a tag is counted on every kind of thing that carries it", () => {
  assert.deepEqual(tagUses("docs", things), { tracks: 2, designs: 1, documents: 1, libraries: 0, routines: 0, total: 4 });
  assert.deepEqual(tagUses("q4", things), { tracks: 1, designs: 0, documents: 1, libraries: 1, routines: 0, total: 3 });
});

test("the knowledge agent's conversation is not something a person tags", () => {
  assert.equal(tagUses("docs", { tracks: [], artifacts: [{ kind: "graph", tags: ["docs"] }], libraries: [] }).total, 0);
});

test("a routine's plan is not counted as a design or a document", () => {
  // The core leaves plans out of the list of every artifact; should one
  // still reach the window, it is counted nowhere.
  assert.equal(tagUses("docs", { tracks: [], artifacts: [{ kind: "plan", tags: ["docs"] }], libraries: [] }).total, 0);
});

test("a tag nothing carries counts nothing", () => {
  assert.deepEqual(tagUses("none", things), { tracks: 0, designs: 0, documents: 0, libraries: 0, routines: 0, total: 0 });
});

test("routines carrying a tag are counted as routines", () => {
  const uses = tagUses("ops", { tracks: [], artifacts: [], libraries: [], routines: [{ tags: ["ops"] }, { tags: [] }, {}] });
  assert.deepEqual(uses, { tracks: 0, designs: 0, documents: 0, libraries: 0, routines: 1, total: 1 });
});
