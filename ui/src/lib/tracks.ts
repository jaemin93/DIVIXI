/** A track saved here or elsewhere, in the list once: in its place if it is there, else at the end. */
export function withTrack<T extends { id: string }>(tracks: T[], track: T): T[] {
  return tracks.some((t) => t.id === track.id) ? tracks.map((t) => (t.id === track.id ? track : t)) : [...tracks, track];
}
