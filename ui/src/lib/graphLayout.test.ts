import test from "node:test";
import assert from "node:assert/strict";
import { MAX_STEP, STILL, neighbourhood, placeNew, queryReach, seedPosition, settle, settleBudget, tick, visibleIds, type Edge } from "./graphLayout.ts";

type B = { id: number; x: number; y: number; vx: number; vy: number; pinned?: boolean };

/** A ring of `n` nodes with a spoke from node 0 to every fifth one: something with structure. */
function web(n: number): { bodies: B[]; edges: Edge[]; links: { a: B; b: B }[] } {
  const bodies: B[] = Array.from({ length: n }, (_, i) => ({ id: i, ...seedPosition(i), vx: 0, vy: 0 }));
  const edges: Edge[] = [];
  for (let i = 0; i < n; i++) edges.push({ source: i, target: (i + 1) % n });
  for (let i = 5; i < n; i += 5) edges.push({ source: 0, target: i });
  const links = edges.map((e) => ({ a: bodies[e.source], b: bodies[e.target] }));
  return { bodies, edges, links };
}

test("seed positions are the same every time, and no two start on top of each other", () => {
  assert.deepEqual(seedPosition(7), seedPosition(7));
  const pts = Array.from({ length: 300 }, (_, i) => seedPosition(i));
  let nearest = Infinity;
  for (let i = 0; i < pts.length; i++)
    for (let j = i + 1; j < pts.length; j++) nearest = Math.min(nearest, Math.hypot(pts[i].x - pts[j].x, pts[i].y - pts[j].y));
  assert.ok(nearest > 15, `nearest pair ${nearest.toFixed(1)} apart`);
});

test("no node moves further than MAX_STEP in a tick, even from a pile-up", () => {
  // Everyone on (nearly) one spot: the old layout's worst case.
  const bodies: B[] = Array.from({ length: 50 }, (_, i) => ({ id: i, x: (i % 3) * 0.01, y: 0, vx: 0, vy: 0 }));
  const before = bodies.map((b) => ({ x: b.x, y: b.y }));
  const moved = tick(bodies, [], 1);
  assert.ok(moved <= MAX_STEP + 1e-9, `moved ${moved}`);
  bodies.forEach((b, i) => assert.ok(Math.hypot(b.x - before[i].x, b.y - before[i].y) <= MAX_STEP + 1e-9));
  assert.ok(bodies.every((b) => Number.isFinite(b.x) && Number.isFinite(b.y)));
});

test("a settled layout stays still: further ticks barely move it", () => {
  const { bodies, links } = web(120);
  const heat = settle(bodies, links, 1, settleBudget(bodies.length));
  // What the live loop does after showing it: cool on from where settling stopped.
  let worst = 0;
  let h = heat;
  for (let i = 0; i < 30; i++) {
    worst = Math.max(worst, tick(bodies, links, h));
    h *= 0.9;
  }
  assert.ok(worst < 1.5, `still moving ${worst.toFixed(2)} per tick after settling`);
  assert.ok(STILL > 0);
});

test("the settling budget shrinks as the graph grows, and never runs out", () => {
  assert.equal(settleBudget(0), 0);
  assert.equal(settleBudget(10), 400);
  assert.ok(settleBudget(300) < settleBudget(100));
  assert.ok(settleBudget(300) >= 60);
});

test("a neighbourhood is the centre and what lies within the given number of relations", () => {
  const edges: Edge[] = [
    { source: 1, target: 2 },
    { source: 2, target: 3 },
    { source: 3, target: 4 },
    { source: 9, target: 1 },
  ];
  assert.deepEqual([...neighbourhood(edges, 1, 0)], [1]);
  assert.deepEqual([...neighbourhood(edges, 1, 1)].sort(), [1, 2, 9]);
  assert.deepEqual([...neighbourhood(edges, 1, 2)].sort(), [1, 2, 3, 9]);
  assert.deepEqual([...neighbourhood(edges, 42, 2)], [42], "a node with no relations is its own neighbourhood");
});

test("what a focused graph shows: the centre wins, then the query, else everything", () => {
  const nodes = [1, 2, 3, 4, 5].map((id) => ({ id }));
  const edges: Edge[] = [
    { source: 1, target: 2 },
    { source: 2, target: 3 },
    { source: 4, target: 5 },
  ];
  const base = { nodes, edges, related: [4, 5], relatedOnly: true, centre: null, depth: 1 };
  assert.deepEqual([...visibleIds(base)!].sort(), [4, 5], "the query's related entities");
  assert.equal(visibleIds({ ...base, relatedOnly: false }), null, "'show all' shows all");
  assert.equal(visibleIds({ ...base, related: [] }), null, "a query that reaches nothing leaves the whole graph");
  assert.equal(visibleIds({ ...base, related: [99] }), null, "related ids outside the drawn graph count as nothing");
  assert.deepEqual([...visibleIds({ ...base, centre: 2 })!].sort(), [1, 2, 3], "a centre outranks the query");
  assert.deepEqual([...visibleIds({ ...base, centre: 1, depth: 2 })!].sort(), [1, 2, 3]);
  assert.deepEqual([...visibleIds({ ...base, centre: 99 })!].sort(), [4, 5], "a centre no longer drawn is ignored");
});

test("a node added later starts beside a neighbour that already has a place", () => {
  const bodies: B[] = [
    { id: 1, x: 500, y: 500, vx: 0, vy: 0 },
    { id: 2, x: 0, y: 0, vx: 0, vy: 0 },
    { id: 3, x: 0, y: 0, vx: 0, vy: 0 },
  ];
  placeNew(bodies, [{ source: 1, target: 2 }], new Set([1]));
  assert.ok(Math.hypot(bodies[1].x - 500, bodies[1].y - 500) < 100, "next to its neighbour");
  assert.ok(Math.hypot(bodies[2].x, bodies[2].y) < 100, "no neighbour: on the spiral near the middle");
  assert.notDeepEqual([bodies[1].x, bodies[1].y], [bodies[2].x, bodies[2].y]);
});

test("a query reaches nodes whose names hold its words, and their neighbours, beside the core's answer", () => {
  const nodes = [
    { id: 1, name: "Query Engine" },
    { id: 2, name: "Batch Query Engine" },
    { id: 3, name: "Gateway" },
    { id: 4, name: "Scheduler" },
    { id: 5, name: "Theme Store" },
    { id: 6, name: "query-engine-cli" },
  ];
  const edges: Edge[] = [
    { source: 2, target: 3 },
    { source: 3, target: 4 },
  ];
  const r = queryReach({ seeds: [], related: [] }, nodes, edges, "engine");
  assert.deepEqual([...r.seeds].sort(), [1, 2, 6], "whole words, in any part of the name, any case");
  assert.deepEqual([...r.related].sort(), [1, 2, 3, 6], "and one relation out from each");
  assert.equal(queryReach({ seeds: [], related: [] }, nodes, edges, "the").seeds.size, 0, "'the' is not a word of 'Theme'");
  const core = queryReach({ seeds: [4], related: [3, 4] }, nodes, edges, "nothing named so");
  assert.deepEqual([...core.seeds], [4], "the core's answer is kept as it is");
  assert.deepEqual([...core.related].sort(), [3, 4]);
  assert.equal(queryReach({ seeds: [], related: [] }, nodes, edges, "").related.size, 0);
});
