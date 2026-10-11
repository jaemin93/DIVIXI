/**
 * Whose folder the file browser shows (store.wsTarget): the routine's beside
 * a routine (`routine:<id>`), the design's own beside a design
 * (`artifact:<id>`), and the track's everywhere else. Pure, so it is tested:
 * a getter that fell back to itself once left the track's panel broken.
 */
export function workspaceTarget(view: string, routine: string, artifact: string, track: string, newRoutine = "new"): string {
  if (view === "routines") return routine && routine !== newRoutine ? `routine:${routine}` : "";
  if (view === "design") return artifact ? `artifact:${artifact}` : "";
  return track;
}
