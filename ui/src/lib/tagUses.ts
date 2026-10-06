/**
 * How many of each kind of thing carry a tag: tracks, designs, documents of
 * the knowledge library, and libraries. The tag dialog shows the total, and
 * the count by kind on hover. Kept apart from the component so it is tested.
 */

export type TagUses = { tracks: number; designs: number; documents: number; libraries: number; total: number };

type Tagged = { tags: string[] };

export function tagUses(
  tag: string,
  things: { tracks: Tagged[]; artifacts: (Tagged & { kind: string })[]; libraries: Tagged[] },
): TagUses {
  const count = (list: Tagged[]) => list.filter((x) => x.tags.includes(tag)).length;
  const tracks = count(things.tracks);
  const designs = count(things.artifacts.filter((a) => a.kind === "design"));
  const documents = count(things.artifacts.filter((a) => a.kind === "knowledge"));
  const libraries = count(things.libraries);
  return { tracks, designs, documents, libraries, total: tracks + designs + documents + libraries };
}
