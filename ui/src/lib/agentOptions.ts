/**
 * The select options an agent advertised, ready for a picker: those with
 * something to pick, each choice once.
 *
 * Copilot CLI lists its models more than once (`auto` three times, every
 * other model twice). Detection drops the repeats now, but a status saved
 * before that, or one from an instance on an older build, still carries
 * them, and every picker keys its list by choice id: one repeat and the
 * list does not draw at all, leaving the last agent's models on screen.
 */

type Choice = { id: string };
type Option<C extends Choice> = { choices: C[] };

export function pickable<C extends Choice, O extends Option<C>>(options: O[]): O[] {
  return options
    .filter((o) => o.choices.length > 0)
    .map((o) => {
      const seen = new Set<string>();
      const choices = o.choices.filter((c) => {
        if (seen.has(c.id)) return false;
        seen.add(c.id);
        return true;
      });
      return choices.length === o.choices.length ? o : { ...o, choices };
    });
}

type Listed<C extends Choice> = { current: string; choices: C[] };

/**
 * The choice a model list shows as picked: what a session would run on.
 * Nothing saved is the agent's own current model (the list has no "agent
 * default" entry of its own); a saved one is that one, listed or not --
 * the run sends it as saved, so the list says so (see [`missingChoice`]).
 * Looking changes nothing that is saved.
 */
export function shownChoice<C extends Choice>(option: Listed<C>, chosen: string | undefined): string {
  return chosen || option.current;
}

/** A saved choice the agent no longer lists, shown as it is, or null. */
export function missingChoice<C extends Choice>(option: Listed<C>, chosen: string | undefined): string | null {
  return chosen && !option.choices.some((c) => c.id === chosen) ? chosen : null;
}

/**
 * A model list to draw: the agent's choices, the one shown as picked marked
 * `on` (✓) and the agent's own current one `now` (nothing saved, the same
 * one is both). A saved choice the agent no longer lists comes first,
 * marked `missing`.
 */
export function choiceList<C extends Choice & { name?: string }>(
  option: Listed<C>,
  chosen: string | undefined,
): (C & { on: boolean; now: boolean; missing: boolean })[] {
  const shown = shownChoice(option, chosen);
  const listed = option.choices.map((c) => ({ ...c, on: c.id === shown, now: c.id === option.current, missing: false }));
  const gone = missingChoice(option, chosen);
  if (!gone) return listed;
  return [{ ...({ id: gone, name: gone } as unknown as C), on: true, now: false, missing: true }, ...listed];
}
