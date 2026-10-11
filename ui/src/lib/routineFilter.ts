/**
 * The routines column's search and tag filter: the tracks column's tag rule
 * (every picked tag, trackFilter.ts) on routines, and the setting it is kept
 * under -- its own, apart from the tracks', the designs' and the libraries'.
 * Kept apart from the component so it is tested.
 */
import { hasAllTags } from "./trackFilter.ts";

/** Where the picked tags are kept: this column's own setting. */
export const ROUTINE_TAGS_KEY = "routinelist_tags";

type Filterable = { name: string; instruction?: string; tags?: string[] };

/** The routines that carry every picked tag and match the search (by name, instruction or tag). */
export function routinesShown<R extends Filterable>(routines: readonly R[], query: string, picked: readonly string[]): R[] {
  const q = query.trim().toLowerCase();
  return routines.filter((r) => {
    const tags = r.tags ?? [];
    if (!hasAllTags(tags, picked)) return false;
    if (!q) return true;
    return r.name.toLowerCase().includes(q) || (r.instruction ?? "").toLowerCase().includes(q) || tags.some((g) => g.toLowerCase().includes(q));
  });
}

/** How many routines carry a tag, for the filter menu. */
export function routineTagCount(routines: readonly Filterable[], tag: string): number {
  return routines.filter((r) => r.tags?.includes(tag)).length;
}
