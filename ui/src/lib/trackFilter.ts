/**
 * What the track list's filter narrows to, apart from the component so it can
 * be tested: whether any filter is on, and whether a track passes them.
 * Sort and fold arrange the list rather than narrow it, so they do not make
 * the filter "on" and clearing leaves them alone.
 */
import type { TrackFilter } from "./store.svelte";

/** What a track brings to the filter. */
export type FilterRow = { live: boolean; active: boolean; lastAt: number; tags: string[] };

/** The narrowing part of a filter set back to nothing. */
export const CLEARED: Pick<TrackFilter, "running" | "active" | "recent" | "tags"> = { running: false, active: false, recent: "", tags: [] };

/** Whether anything narrows the list: running, active, a recent window or a picked tag. */
export function anyFilterOn(f: TrackFilter): boolean {
  return f.running || f.active || f.recent !== "" || f.tags.length > 0;
}

/** Picked tags are AND-ed: a track must carry every one of them. */
export function hasAllTags(trackTags: readonly string[], picked: readonly string[]): boolean {
  return picked.every((g) => trackTags.includes(g));
}

/** Picks a tag, or unpicks it if it was picked; the order of the rest is kept. */
export function toggleTag(picked: readonly string[], tag: string): string[] {
  return picked.includes(tag) ? picked.filter((x) => x !== tag) : [...picked, tag];
}

/** Whether a track passes the filter. `recentMs` is the window `f.recent` stands for. */
export function passesFilter(row: FilterRow, f: TrackFilter, now: number, recentMs: number): boolean {
  if (f.running && !row.live) return false;
  if (f.active && !row.active) return false;
  if (f.recent && now - row.lastAt > recentMs) return false;
  return hasAllTags(row.tags, f.tags);
}
