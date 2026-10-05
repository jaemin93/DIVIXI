/**
 * What the knowledge graph's conversation is given with a message: the parts
 * of the graph the person has in front of them, as chips they can take out
 * and put back. Kept apart from the component so the rules are tested.
 *
 * A chip carries picks in the form the core reads (`e:<entity id>` for an
 * entity, `i:<item id>` for a passage that mentions one); the core turns them
 * into names, kinds, relations and passage text from the library itself, so
 * the message carries ids, not copies of the data.
 */

export type ChipKind = "entity" | "focus" | "query" | "sources";

export type Chip = {
  /** Stable for what the chip stands for: a new pick is a new chip, and starts included. */
  key: string;
  kind: ChipKind;
  /** The entity, centre or query the chip is about. */
  name: string;
  /** How many entities or passages it carries. */
  count: number;
  picks: string[];
};

/** Most entities and passages one message carries (the core cuts at the same numbers). */
export const MAX_ENTITIES = 80;
export const MAX_PASSAGES = 12;

/**
 * The chips on offer, in the order they read best: the picked entity, its
 * passages, the centre's neighbourhood, the query's part of the graph. A
 * chip with nothing in it is not offered.
 */
export function contextChips(opts: {
  picked: { id: number; name: string } | null;
  pickedItems: { id: number }[];
  centre: { id: number; name: string; depth: number } | null;
  focusIds: number[];
  query: string;
  queryIds: number[];
}): Chip[] {
  const chips: Chip[] = [];
  const entities = (ids: number[]) => [...new Set(ids)].map((id) => `e:${id}`);
  if (opts.picked) chips.push({ key: `entity:${opts.picked.id}`, kind: "entity", name: opts.picked.name, count: 1, picks: [`e:${opts.picked.id}`] });
  if (opts.picked && opts.pickedItems.length) {
    const picks = [...new Set(opts.pickedItems.map((i) => i.id))].map((id) => `i:${id}`);
    chips.push({ key: `sources:${opts.picked.id}`, kind: "sources", name: opts.picked.name, count: picks.length, picks });
  }
  if (opts.centre && opts.focusIds.length) {
    const picks = entities(opts.focusIds);
    chips.push({ key: `focus:${opts.centre.id}:${opts.centre.depth}`, kind: "focus", name: opts.centre.name, count: picks.length, picks });
  }
  const q = opts.query.trim();
  if (q && opts.queryIds.length) {
    const picks = entities(opts.queryIds);
    chips.push({ key: `query:${q}`, kind: "query", name: q, count: picks.length, picks });
  }
  return chips;
}

/**
 * What goes with the message: the picks of every chip not taken out, each
 * once, entities first then passages, cut at the limits.
 */
export function picksOf(chips: Chip[], excluded: ReadonlySet<string>): string[] {
  const entities: string[] = [];
  const passages: string[] = [];
  for (const chip of chips) {
    if (excluded.has(chip.key)) continue;
    for (const p of chip.picks) {
      const list = p.startsWith("i:") ? passages : entities;
      const limit = p.startsWith("i:") ? MAX_PASSAGES : MAX_ENTITIES;
      if (list.length < limit && !list.includes(p)) list.push(p);
    }
  }
  return [...entities, ...passages];
}

/** Take a chip out, or put it back. */
export function toggleChip(excluded: ReadonlySet<string>, key: string): Set<string> {
  const next = new Set(excluded);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}
