/**
 * What one of the core's notices opens, read from the data it carries.
 * Kept apart from the bell's store so it can be tested on its own.
 */

/** The routine a notice is about (a routine's run cut short), if any. */
export function routineOf(data: unknown): string | null {
  if (!data || typeof data !== "object") return null;
  const routine = (data as Record<string, unknown>).routine;
  return typeof routine === "string" && routine.trim() ? routine : null;
}
