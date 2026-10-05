/**
 * The nodes of one kind, for the list a legend entry opens on the graph:
 * each with how many relations it has and how often it is mentioned, the
 * best-connected first, narrowed by what is typed in the list's filter.
 */

export type KindRow = { id: number; name: string; links: number; mentions: number };

export function nodesOfKind(
  nodes: { id: number; name: string; kind: string; mentions: number }[],
  edges: { source: number; target: number }[],
  kind: string,
  filter = "",
): KindRow[] {
  const links = new Map<number, number>();
  for (const e of edges) {
    if (e.source === e.target) continue;
    links.set(e.source, (links.get(e.source) ?? 0) + 1);
    links.set(e.target, (links.get(e.target) ?? 0) + 1);
  }
  const f = filter.trim().toLowerCase();
  return nodes
    .filter((n) => n.kind === kind && (!f || n.name.toLowerCase().includes(f)))
    .map((n) => ({ id: n.id, name: n.name, links: links.get(n.id) ?? 0, mentions: n.mentions }))
    .sort((a, b) => b.links - a.links || b.mentions - a.mentions || a.name.localeCompare(b.name));
}

/** How many nodes each kind has, most numerous first: the legend's order. */
export function kindCounts(nodes: { kind: string }[]): { kind: string; count: number }[] {
  const count = new Map<string, number>();
  for (const n of nodes) count.set(n.kind, (count.get(n.kind) ?? 0) + 1);
  return [...count].map(([kind, n]) => ({ kind, count: n })).sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind));
}
