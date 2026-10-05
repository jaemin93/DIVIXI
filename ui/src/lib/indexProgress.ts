/**
 * Whether the knowledge library is being indexed right now, and how far it
 * has got, for the small "Indexing N/M" mark the List and Graph tabs show in
 * place of the full status line (which lives on the Sources tab). Nothing to
 * say once indexing is done.
 *
 * Two kinds of work count: documents being synced (read, split, described),
 * counted in documents; and, after that, passages being embedded, counted in
 * passages. Syncing comes first, so it is the one reported while both run.
 */

export type IndexProgress = { kind: "sync" | "embed"; done: number; total: number };

export function indexProgress(
  sources: { status: string }[],
  embedding: { enabled: boolean; embedded: number; failed: number; total: number; error: string },
): IndexProgress | null {
  const syncing = sources.some((s) => s.status === "pending" || s.status === "indexing");
  if (syncing) {
    // Documents that will not index (missing, duplicate, failed) are not waited on.
    const counted = sources.filter((s) => s.status === "synced" || s.status === "pending" || s.status === "indexing");
    return { kind: "sync", done: counted.filter((s) => s.status === "synced").length, total: counted.length };
  }
  // Embedding: on, not stopped by an error, and passages still without a vector.
  const e = embedding;
  if (e.enabled && !e.error && e.total > 0 && e.embedded + e.failed < e.total) return { kind: "embed", done: e.embedded, total: e.total };
  return null;
}
