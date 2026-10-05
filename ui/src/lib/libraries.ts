/**
 * Libraries of the knowledge page: named sets of documents. A document is in
 * one library; search (the conductor's, `@kb`) sees them all, the page shows
 * one or all. The rules the core keeps, mirrored here so the page can say
 * what is wrong before asking, and tested apart from the components.
 */

/** The library every document starts in, never deleted (the core's `GENERAL`). */
export const GENERAL = 1;
/** Its name as stored; shown in the person's language until they rename it. */
export const GENERAL_NAME = "General";
/** Longest name, in characters (the core's `MAX_LIBRARY_NAME`). */
export const MAX_NAME = 80;

/** Mirrors `orchestra_knowledge::Library`. */
export type KLibrary = { id: number; name: string; created_at: number; sources: number };

/** A library's name as shown: General's in the person's language until renamed. */
export function displayName(lib: Pick<KLibrary, "id" | "name">, generalLabel: string): string {
  return lib.id === GENERAL && lib.name === GENERAL_NAME ? generalLabel : lib.name;
}

/** The documents a view shows: of one library, or all of them (`null`). */
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

export type DeleteBlock = "" | "general" | "notEmpty";

/** Why a library cannot be deleted: General never is, nor one with documents. "" when it can. */
export function deleteBlock(lib: Pick<KLibrary, "id" | "sources">): DeleteBlock {
  if (lib.id === GENERAL) return "general";
  if (lib.sources > 0) return "notEmpty";
  return "";
}

/** Where a document added from a view goes: its library, or General from the view of all. */
export function addTarget(library: number | null): number {
  return library ?? GENERAL;
}

/** The library a view shows, as the graph conversation is told it (`l:<id>`); nothing for all. */
export function libraryPick(library: number | null): string[] {
  return library === null ? [] : [`l:${library}`];
}

/**
 * The library a view should show once the libraries are (re)loaded: the one
 * it showed if it is still there, else all of them.
 */
export function keepLibrary(library: number | null, libraries: Pick<KLibrary, "id">[]): number | null {
  return library !== null && libraries.some((l) => l.id === library) ? library : null;
}

/** A remembered choice ("all", or a library's id) read back; anything else is all. */
export function parseLibrary(saved: string | null | undefined): number | null {
  if (!saved || saved === "all") return null;
  const n = Number(saved);
  return Number.isInteger(n) && n > 0 ? n : null;
}
