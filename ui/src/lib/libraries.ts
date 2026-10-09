/**
 * Libraries of the knowledge page: named sets of documents. A document is in
 * one library; search (the conductor's, `@kb`) sees them all, the page shows
 * one at a time. The rules the core keeps, mirrored here so the page can say
 * what is wrong before asking, and tested apart from the components.
 */

/** The library a new library file starts with (the core's `GENERAL`); deleted like any other. */
export const GENERAL = 1;
/** Its name as stored; shown in the person's language until they rename it. */
export const GENERAL_NAME = "General";
/** Longest name, in characters (the core's `MAX_LIBRARY_NAME`). */
export const MAX_NAME = 80;

/** Mirrors `orchestra_knowledge::Library`. */
export type KLibrary = { id: number; name: string; created_at: number; sources: number; color: string; tags: string[] };

/** A library as a row of the list column: its tags and its colour (no count: the page body says it). */
export function libraryRow(lib: KLibrary, shownName: string): { id: string; name: string; tags: string[]; color: string } {
  return { id: String(lib.id), name: shownName, tags: lib.tags, color: lib.color };
}

/** The tag-dialog key for a library (`lib:<id>`), beside a track's `tr…` and an artifact's `ar…`. */
export function libraryTagKey(id: number): string {
  return `lib:${id}`;
}

/** The library a tag-dialog key names, or null for another kind of key. */
export function libraryOfTagKey(key: string): number | null {
  const m = /^lib:(\d+)$/.exec(key);
  return m ? Number(m[1]) : null;
}

/** A library's name as shown: General's in the person's language until renamed. */
export function displayName(lib: Pick<KLibrary, "id" | "name">, generalLabel: string): string {
  return lib.id === GENERAL && lib.name === GENERAL_NAME ? generalLabel : lib.name;
}

/** The documents a view shows: of one library, or all of them (`null`, before any library is loaded). */
export function sourcesIn<T extends { library_id: number }>(sources: T[], library: number | null): T[] {
  return library === null ? sources : sources.filter((s) => s.library_id === library);
}

/** A name as the core keeps it: spaces at the ends dropped, runs of them made one. */
export function cleanName(name: string): string {
  return name.trim().split(/\s+/).filter(Boolean).join(" ");
}

export type NameProblem = "" | "empty" | "long" | "taken";

/**
 * What is wrong with a name for a new library (or for `except` renamed):
 * empty, too long, or another library's (case aside). "" when it is fine.
 */
export function nameProblem(name: string, libraries: Pick<KLibrary, "id" | "name">[], except?: number): NameProblem {
  const clean = cleanName(name);
  if (!clean) return "empty";
  if ([...clean].length > MAX_NAME) return "long";
  const key = clean.toLowerCase();
  if (libraries.some((l) => l.id !== except && l.name.toLowerCase() === key)) return "taken";
  return "";
}

export type DeleteBlock = "" | "notEmpty";

/** Why a library cannot be deleted: it holds documents (General too). "" when it can. */
export function deleteBlock(lib: Pick<KLibrary, "sources">): DeleteBlock {
  return lib.sources > 0 ? "notEmpty" : "";
}

/** The library a view shows, as the graph conversation is told it (`l:<id>`); nothing when none is. */
export function libraryPick(library: number | null): string[] {
  return library === null ? [] : [`l:${library}`];
}

/**
 * The library a view should show once the libraries are (re)loaded: the one
 * it showed if it is still there, else the first (the core lists General
 * first while it exists, then the oldest). Null when there are none. (The
 * page once had a view of all libraries, remembered as "all"; it now opens
 * on the first.)
 */
export function keepLibrary(library: number | null, libraries: Pick<KLibrary, "id">[]): number | null {
  if (library !== null && libraries.some((l) => l.id === library)) return library;
  return libraries[0]?.id ?? null;
}

/** A remembered choice (a library's id) read back; anything else, the old "all" too, is none. */
export function parseLibrary(saved: string | null | undefined): number | null {
  if (!saved) return null;
  const n = Number(saved);
  return Number.isInteger(n) && n > 0 ? n : null;
}
