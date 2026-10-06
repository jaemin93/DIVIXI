/**
 * Finding cards on a design's board (Ctrl+F / Cmd+F): by what they say (a
 * note's or question's text, a frame's title, a link's or file's name, a
 * question's answer, a link's address) or by the id the design agent uses
 * for them ("n441"). Kept apart from the board so it is tested.
 */

type Findable = { id: string; kind: string; x: number; y: number; text: string; name?: string; answer?: string; url?: string };

/** Text as compared: composed (a Korean syllable typed in jamo matches), case aside, spaces made one. */
export function fold(s: string): string {
  return s.normalize("NFC").toLocaleLowerCase().replace(/\s+/g, " ").trim();
}

/** What a card says, as searched. A sketch says nothing. */
function words(n: Findable): string {
  if (n.kind === "sketch") return "";
  return fold([n.text, n.name ?? "", n.answer ?? "", n.url ?? ""].join("\n"));
}

/**
 * The ids of the cards a query finds, in the order Enter steps through them:
 * a card whose id is the query exactly ("n441", any case) first, then the
 * cards whose words hold the query, top to bottom and left to right, as the
 * board reads. An empty query finds nothing.
 */
export function findCards(nodes: Findable[], query: string): string[] {
  const q = fold(query);
  if (!q) return [];
  const exact = nodes.filter((n) => n.id.toLowerCase() === q);
  const rest = nodes
    .filter((n) => !exact.includes(n) && words(n).includes(q))
    .sort((a, b) => a.y - b.y || a.x - b.x || a.id.localeCompare(b.id));
  return [...exact.map((n) => n.id), ...rest.map((n) => n.id)];
}

/** The next place in a list of `count` matches from `at` (-1: none yet), going forward or back, round. */
export function stepMatch(at: number, count: number, back: boolean): number {
  if (count <= 0) return -1;
  if (at < 0) return back ? count - 1 : 0;
  return (at + (back ? -1 : 1) + count) % count;
}

/** The id label is shown at this zoom and above; smaller, the labels would crowd the cards. */
export const ID_LABEL_MIN_ZOOM = 0.25;

/** Whether Ctrl+F (Cmd+F on macOS) was pressed, and nothing else with it but Shift. */
export function isFindKey(e: { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }, mac: boolean): boolean {
  return e.key.toLowerCase() === "f" && !e.altKey && (mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey);
}
