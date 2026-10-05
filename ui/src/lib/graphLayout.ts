/**
 * The knowledge graph's layout, apart from its drawing: where nodes start,
 * one step of the force layout, settling it before anyone sees it, and which
 * nodes a focused view keeps.
 *
 * Why it is shaped like this. Entering the graph tab used to drop every node
 * on a small circle at random and let the forces sort it out on screen. With
 * a few hundred nodes, neighbours on that circle start almost on top of each
 * other, the repulsion (strength / distance²) is enormous, and nothing capped
 * the speed: the plate exploded and then took some thirteen seconds to cool,
 * every time the tab was opened. Now nodes start spread out, no node moves
 * further than `MAX_STEP` in a tick, the layout is settled off screen before
 * the first frame, and positions are remembered so coming back to the tab
 * starts where it left off.
 */

export type Point = { x: number; y: number; vx: number; vy: number; pinned?: boolean };
export type Link<P> = { a: P; b: P };
export type Edge = { source: number; target: number };

/** Repulsion between every pair closer than `CUTOFF`. */
export const REPULSE = 1200;
/** The length a link pulls towards. */
export const LINK_LENGTH = 90;
const SPRING = 0.02;
const GRAVITY = 0.004;
const DAMPING = 0.82;
/** Pairs further apart than this (squared) do not push each other. */
const CUTOFF2 = 300 * 300;
/** No node moves further than this in one tick, however hard it is pushed. */
export const MAX_STEP = 12;

/** The golden angle, for a sunflower spiral. */
const GOLDEN = Math.PI * (3 - Math.sqrt(5));
/** Spiral spacing: neighbouring seeds start about half a link apart. */
const SPIRAL = 28;

/**
 * Where the i-th node starts: a sunflower spiral, the same every time, spaced
 * evenly however many nodes there are. Nothing starts on top of anything.
 */
export function seedPosition(i: number): { x: number; y: number } {
  const r = SPIRAL * Math.sqrt(i + 0.5);
  const a = i * GOLDEN;
  return { x: Math.cos(a) * r, y: Math.sin(a) * r };
}

/**
 * One step of the force layout at temperature `heat`. Returns the largest
 * distance any node moved, which says better than `heat` whether the layout
 * is still going anywhere.
 */
export function tick<P extends Point>(bodies: P[], links: Link<P>[], heat: number): number {
  const n = bodies.length;
  for (let i = 0; i < n; i++) {
    const a = bodies[i];
    for (let j = i + 1; j < n; j++) {
      const b = bodies[j];
      let dx = b.x - a.x;
      let dy = b.y - a.y;
      let d2 = dx * dx + dy * dy;
      if (d2 > CUTOFF2) continue;
      if (d2 < 0.01) {
        // Two nodes on the same spot: part them along a fixed direction, not a random one.
        dx = (i % 7) - 3 || 1;
        dy = (j % 5) - 2 || 1;
        d2 = dx * dx + dy * dy;
      }
      const f = (REPULSE / d2) * heat;
      const d = Math.sqrt(d2);
      a.vx -= (dx / d) * f;
      a.vy -= (dy / d) * f;
      b.vx += (dx / d) * f;
      b.vy += (dy / d) * f;
    }
  }
  for (const { a, b } of links) {
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const d = Math.sqrt(dx * dx + dy * dy) || 1;
    const f = (d - LINK_LENGTH) * SPRING * heat;
    a.vx += (dx / d) * f;
    a.vy += (dy / d) * f;
    b.vx -= (dx / d) * f;
    b.vy -= (dy / d) * f;
  }
  let moved = 0;
  for (const b of bodies) {
    b.vx -= b.x * GRAVITY * heat;
    b.vy -= b.y * GRAVITY * heat;
    if (b.pinned) {
      b.vx = 0;
      b.vy = 0;
      continue;
    }
    b.vx *= DAMPING;
    b.vy *= DAMPING;
    const speed = Math.hypot(b.vx, b.vy);
    if (speed > MAX_STEP) {
      b.vx *= MAX_STEP / speed;
      b.vy *= MAX_STEP / speed;
    }
    b.x += b.vx;
    b.y += b.vy;
    moved = Math.max(moved, Math.min(speed, MAX_STEP));
  }
  return moved;
}

/** Below this much movement in a tick (world units), the layout counts as still. */
export const STILL = 0.05;

/**
 * Run the layout off screen until it is still, or `maxTicks` have passed,
 * cooling from `heat` by `cooling` each tick. Returns the heat it ends at.
 * The cost is about n² per tick, so callers with many nodes pass fewer ticks.
 */
export function settle<P extends Point>(bodies: P[], links: Link<P>[], heat: number, maxTicks: number, cooling = 0.985): number {
  for (let i = 0; i < maxTicks; i++) {
    const moved = tick(bodies, links, heat);
    heat *= cooling;
    if (moved < STILL && i > 10) break;
  }
  for (const b of bodies) {
    b.vx = 0;
    b.vy = 0;
  }
  return heat;
}

/** Ticks to settle n nodes off screen: plenty for a small graph, fewer as the n² cost grows. */
export function settleBudget(n: number): number {
  if (n <= 0) return 0;
  // About eight million pair visits at most, which is a few tens of milliseconds.
  return Math.max(60, Math.min(400, Math.floor(8_000_000 / Math.max(1, (n * (n - 1)) / 2))));
}

/** The nodes within `depth` relations of `centre`, the centre included. */
export function neighbourhood(edges: Edge[], centre: number, depth: number): Set<number> {
  const next = new Map<number, number[]>();
  for (const { source, target } of edges) {
    (next.get(source) ?? next.set(source, []).get(source)!).push(target);
    (next.get(target) ?? next.set(target, []).get(target)!).push(source);
  }
  const seen = new Set<number>([centre]);
  let ring = [centre];
  for (let d = 0; d < depth; d++) {
    const out: number[] = [];
    for (const id of ring) for (const n of next.get(id) ?? []) if (!seen.has(n)) (seen.add(n), out.push(n));
    ring = out;
  }
  return seen;
}

/**
 * What a focused graph shows, or null for everything. A node chosen as the
 * centre wins; otherwise the shown query's related entities, when there are
 * any and the person has not asked to see the whole graph. Ids that are not
 * in the graph (it is capped at its most mentioned entities) are dropped, and
 * a focus left with nothing in it falls back to the whole graph.
 */
export function visibleIds(opts: {
  nodes: { id: number }[];
  edges: Edge[];
  related: number[];
  relatedOnly: boolean;
  centre: number | null;
  depth: number;
}): Set<number> | null {
  const present = new Set(opts.nodes.map((n) => n.id));
  let want: Set<number> | null = null;
  if (opts.centre !== null && present.has(opts.centre)) want = neighbourhood(opts.edges, opts.centre, opts.depth);
  else if (opts.relatedOnly && opts.related.length) want = new Set(opts.related);
  if (!want) return null;
  const kept = new Set([...want].filter((id) => present.has(id)));
  return kept.size ? kept : null;
}

/**
 * Starting places for nodes that have none: beside an already placed
 * neighbour when there is one (so a node added by a sync lands near what it
 * relates to), otherwise on the spiral past the nodes already there.
 */
export function placeNew<P extends Point & { id: number }>(bodies: P[], edges: Edge[], placed: Set<number>): void {
  const byId = new Map(bodies.map((b) => [b.id, b]));
  let slot = placed.size;
  for (const b of bodies) {
    if (placed.has(b.id)) continue;
    const near = edges
      .flatMap((e) => (e.source === b.id ? [e.target] : e.target === b.id ? [e.source] : []))
      .map((id) => (placed.has(id) ? byId.get(id) : undefined))
      .filter((p): p is P => !!p);
    if (near.length) {
      const cx = near.reduce((s, p) => s + p.x, 0) / near.length;
      const cy = near.reduce((s, p) => s + p.y, 0) / near.length;
      const a = (b.id % 12) * (Math.PI / 6);
      b.x = cx + Math.cos(a) * LINK_LENGTH * 0.6;
      b.y = cy + Math.sin(a) * LINK_LENGTH * 0.6;
    } else {
      const p = seedPosition(slot++);
      b.x = p.x;
      b.y = p.y;
    }
    b.vx = 0;
    b.vy = 0;
    placed.add(b.id);
  }
}

/** The words of a name or a query, lower-cased: split on anything that is not a letter or a digit. */
function words(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length >= 2);
}

/**
 * What a query reaches in the graph, for the focused view. The core's answer
 * (`seeds`, `related`: the entities the query names exactly, and two relations
 * out) is widened by name: a node whose name holds one of the query's words as
 * a whole word counts as named too, and its direct neighbours as related. So
 * "engine" reaches "Query Engine", "Batch Engine" and "engine-cli", not just
 * an entity called exactly that. Whole words only, so "the" does not light
 * up "theme".
 */
export function queryReach(
  core: { seeds: number[]; related: number[] },
  nodes: { id: number; name: string }[],
  edges: Edge[],
  query: string,
): { seeds: Set<number>; related: Set<number> } {
  const seeds = new Set(core.seeds);
  const related = new Set(core.related);
  const asked = new Set(words(query));
  if (asked.size) {
    for (const n of nodes) {
      if (!words(n.name).some((w) => asked.has(w))) continue;
      seeds.add(n.id);
      for (const id of neighbourhood(edges, n.id, 1)) related.add(id);
    }
  }
  for (const id of seeds) related.add(id);
  return { seeds, related };
}
