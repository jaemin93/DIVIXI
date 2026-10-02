/**
 * The log level as the settings page shows it. Mirrors `logging::Level` in
 * src-tauri/src/logging.rs, which names the preset a filter matches or says
 * `custom`.
 */

/** How much is being logged, and what set it. */
export type LogLevel = { filter: string; preset: string; source: string };

/** The presets the backend offers, in the order they read in. */
export const LEVEL_PRESETS = ["quiet", "info", "debug", "trace"] as const;

/** The filter to spell out: only one no preset button stands for. */
export function customFilter(level: LogLevel | null): string | null {
  if (level === null) return null;
  return (LEVEL_PRESETS as readonly string[]).includes(level.preset) ? null : level.filter;
}
