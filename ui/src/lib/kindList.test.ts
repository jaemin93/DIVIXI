import test from "node:test";
import assert from "node:assert/strict";
import { kindCounts, nodesOfKind } from "./kindList.ts";

const nodes = [
  { id: 1, name: "Query Engine", kind: "service", mentions: 3 },
  { id: 2, name: "Router", kind: "service", mentions: 9 },
  { id: 3, name: "Scheduler", kind: "service", mentions: 1 },
  { id: 4, name: "Planner", kind: "concept", mentions: 5 },
  { id: 5, name: "Batch Engine", kind: "service", mentions: 3 },
  { id: 6, name: "Cache", kind: "technology", mentions: 2 },
];
const edges = [
  { source: 1, target: 4 },
  { source: 1, target: 6 },
  { source: 2, target: 4 },
  { source: 5, target: 5 },
];

test("a kind's nodes, the best-connected first, then the most mentioned, then by name", () => {
  const rows = nodesOfKind(nodes, edges, "service");
  assert.deepEqual(
    rows.map((r) => [r.name, r.links, r.mentions]),
    [
      ["Query Engine", 2, 3],
      ["Router", 1, 9],
      ["Batch Engine", 0, 3],
      ["Scheduler", 0, 1],
    ],
  );
  assert.equal(rows.find((r) => r.id === 5)!.links, 0, "a relation to itself is not a link");
});

test("the filter narrows by name, any case, anywhere in it", () => {
  assert.deepEqual(nodesOfKind(nodes, edges, "service", "ENGINE").map((r) => r.name), ["Query Engine", "Batch Engine"]);
  assert.deepEqual(nodesOfKind(nodes, edges, "service", "  rout ").map((r) => r.name), ["Router"]);
  assert.deepEqual(nodesOfKind(nodes, edges, "service", "nothing"), []);
  assert.deepEqual(nodesOfKind(nodes, edges, "org"), [], "a kind with no nodes");
});

test("the legend's kinds, most numerous first", () => {
  assert.deepEqual(kindCounts(nodes), [
    { kind: "service", count: 4 },
    { kind: "concept", count: 1 },
    { kind: "technology", count: 1 },
  ]);
  assert.deepEqual(kindCounts([]), []);
});
