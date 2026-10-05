import test from "node:test";
import assert from "node:assert/strict";
import { MAX_ENTITIES, MAX_PASSAGES, contextChips, picksOf, toggleChip } from "./graphContext.ts";

const none = { picked: null, pickedItems: [], centre: null, focusIds: [], query: "", queryIds: [] };

test("nothing picked, nothing offered", () => {
  assert.deepEqual(contextChips(none), []);
  assert.deepEqual(picksOf([], new Set()), []);
});

test("the chips on offer: the entity, its passages, the centre's neighbourhood, the query's part", () => {
  const chips = contextChips({
    picked: { id: 7, name: "Query Engine" },
    pickedItems: [{ id: 70 }, { id: 71 }, { id: 70 }],
    centre: { id: 7, name: "Query Engine", depth: 2 },
    focusIds: [7, 8, 9],
    query: " scheduler ",
    queryIds: [9, 10],
  });
  assert.deepEqual(
    chips.map((c) => [c.key, c.kind, c.count]),
    [
      ["entity:7", "entity", 1],
      ["sources:7", "sources", 2],
      ["focus:7:2", "focus", 3],
      ["query:scheduler", "query", 2],
    ],
  );
  assert.deepEqual(chips[1].picks, ["i:70", "i:71"], "each passage once");
  assert.equal(chips[3].name, "scheduler", "the query as typed, trimmed");
});

test("a chip with nothing in it is not offered", () => {
  const chips = contextChips({ ...none, picked: { id: 1, name: "Router" }, centre: { id: 1, name: "Router", depth: 1 }, query: "x" });
  assert.deepEqual(
    chips.map((c) => c.kind),
    ["entity"],
    "no passages yet, no focus set, a query reaching nothing",
  );
});

test("what goes with the message: every chip not taken out, each pick once, entities then passages", () => {
  const chips = contextChips({
    picked: { id: 7, name: "Query Engine" },
    pickedItems: [{ id: 70 }],
    centre: null,
    focusIds: [],
    query: "engine",
    queryIds: [7, 8],
  });
  assert.deepEqual(picksOf(chips, new Set()), ["e:7", "e:8", "i:70"]);
  assert.deepEqual(picksOf(chips, new Set(["query:engine"])), ["e:7", "i:70"]);
  assert.deepEqual(picksOf(chips, new Set(["entity:7", "sources:7"])), ["e:7", "e:8"], "the query still brings the entity");
  assert.deepEqual(picksOf(chips, new Set(chips.map((c) => c.key))), []);
});

test("taking a chip out and putting it back", () => {
  const out = toggleChip(new Set(), "query:engine");
  assert.deepEqual([...out], ["query:engine"]);
  assert.deepEqual([...toggleChip(out, "query:engine")], []);
  assert.deepEqual([...out], ["query:engine"], "the set given is not changed in place");
});

test("a message carries at most so many entities and passages", () => {
  const chips = contextChips({
    ...none,
    picked: { id: 1, name: "Router" },
    pickedItems: Array.from({ length: 40 }, (_, i) => ({ id: 1000 + i })),
    centre: { id: 1, name: "Router", depth: 2 },
    focusIds: Array.from({ length: 300 }, (_, i) => i + 1),
  });
  const picks = picksOf(chips, new Set());
  assert.equal(picks.filter((p) => p.startsWith("e:")).length, MAX_ENTITIES);
  assert.equal(picks.filter((p) => p.startsWith("i:")).length, MAX_PASSAGES);
});
