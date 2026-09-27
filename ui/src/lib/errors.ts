/**
 * Errors the human should see, wherever in the app they happened.
 *
 * Something goes wrong in fifty-odd places, and each says so the same way:
 * `store.lastError = String(err)`. That used to be one string shown in
 * three screens, so a failure in settings, the workspace panel, the
 * library or a worker view left no trace at all, and one that did show
 * stayed until something overwrote it. The store keeps a list now and
 * `ErrorToasts` shows it over every view.
 *
 * The rules live here, free of runes, so they can be read and tested
 * without a running app.
 */

/** Something that went wrong, waiting to be read. */
export type AppError = {
  id: number;
  text: string;
  /** Unix milliseconds it was raised, or last happened again at. */
  at: number;
  /** How many further times the same thing has happened since. */
  again: number;
};

/**
 * How long an error stays on screen before it goes by itself.
 *
 * Long enough to read, short enough not to become furniture. The floor is
 * six seconds; past about sixty characters every further character buys
 * 45ms of reading time, to a ceiling of twenty seconds for the long ones
 * (a path, or a process quoting itself). Hovering or focusing the stack
 * stops the clock, so nothing is taken away mid-sentence.
 */
export function errorLife(text: string): number {
  return Math.min(20_000, 6_000 + Math.max(0, text.length - 60) * 45);
}

/**
 * Add an error to the list.
 *
 * The same failure twice running is one error that happened twice, not
 * two to read: the last one's count goes up and its clock starts again,
 * which is what keeps a retry loop from burying the screen in copies.
 * `id` is only used when a new one is added.
 */
export function addError(list: AppError[], text: string, id: number, at: number): AppError[] {
  const said = text.trim();
  if (!said) return list;
  const last = list.at(-1);
  if (last && last.text === said) {
    return [...list.slice(0, -1), { ...last, at, again: last.again + 1 }];
  }
  return [...list, { id, text: said, at, again: 0 }];
}

/** Take one error off the screen (read, closed, or expired). */
export function dropError(list: AppError[], id: number): AppError[] {
  return list.filter((e) => e.id !== id);
}
