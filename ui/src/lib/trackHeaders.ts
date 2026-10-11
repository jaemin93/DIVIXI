/**
 * Which tracks' headers are folded to one line, as kept in the
 * `track_headers` setting: a map from track id to folded (true) or open
 * (false).
 *
 * A track with no entry is folded: the conversation gets the room, and the
 * one-line header still shows the name and the first line of the intent.
 * Opening or folding a header writes an explicit entry, so the choice is
 * remembered either way. Older builds wrote only `true` entries, which read
 * back the same.
 */
export type HeaderFolds = Record<string, boolean>;

/** A stored value read back. Anything not a map of booleans counts as nothing stored. */
export function parseHeaderFolds(raw: string | null | undefined): HeaderFolds {
  if (!raw) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
  const out: HeaderFolds = {};
  for (const [id, folded] of Object.entries(parsed as Record<string, unknown>)) {
    if (typeof folded === "boolean") out[id] = folded;
  }
  return out;
}

/** Whether a track's header is folded: what was chosen, else folded. */
export function headerFolded(folds: HeaderFolds, track: string): boolean {
  return folds[track] ?? true;
}

/** The map with one track's choice written in. */
export function withHeaderFold(folds: HeaderFolds, track: string, folded: boolean): HeaderFolds {
  return { ...folds, [track]: folded };
}

/** The map without a track, once the track is gone. */
export function withoutTrack(folds: HeaderFolds, track: string): HeaderFolds {
  const { [track]: _gone, ...rest } = folds;
  return rest;
}
