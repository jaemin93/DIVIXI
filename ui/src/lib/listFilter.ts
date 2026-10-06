/**
 * Narrowing the designs and libraries columns as the tracks column narrows
 * its list: the search box (an item's name or one of its tags, case aside)
 * and the picked tags (AND-ed, as the tracks column has them). Kept apart
 * from the component so it is tested.
 */

type Narrowable = { name: string; tags: string[] };

/** Text as compared: composed, case aside, ends trimmed. */
function fold(s: string): string {
  return s.normalize("NFC").toLocaleLowerCase().trim();
}

/** Whether a search (as typed) matches an item: its name or one of its tags holds it. Empty matches all. */
export function matchesQuery(item: Narrowable, query: string): boolean {
  const q = fold(query);
  return !q || fold(item.name).includes(q) || item.tags.some((g) => fold(g).includes(q));
}

/** The items the search and the picked tags let through, in their order. */
export function narrow<T extends Narrowable>(items: T[], query: string, picked: readonly string[]): T[] {
  // Every picked tag, as trackFilter's hasAllTags (not imported: the tests run these files as they are).
  return items.filter((i) => matchesQuery(i, query) && picked.every((g) => i.tags.includes(g)));
}

/** A kept tag filter read back: a list of tag names, anything else none. */
export function parseTags(saved: string | null | undefined): string[] {
  if (!saved) return [];
  try {
    const v = JSON.parse(saved);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}
