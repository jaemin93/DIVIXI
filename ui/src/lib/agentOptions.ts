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
 * The choice a model list shows as picked. Nothing saved (or a choice the
 * agent no longer lists) shows the agent's own current model, which is what
 * such a session runs on: the list has no "agent default" entry of its own,
 * and what is saved stays as it is, so a run is not changed by looking.
 */
export function shownChoice<C extends Choice>(option: Listed<C>, chosen: string | undefined): string {
  return chosen && option.choices.some((c) => c.id === chosen) ? chosen : option.current;
}

/** A model list to draw: the agent's choices only, the one shown as picked marked. */
export function choiceList<C extends Choice>(option: Listed<C>, chosen: string | undefined): (C & { on: boolean })[] {
  const shown = shownChoice(option, chosen);
  return option.choices.map((c) => ({ ...c, on: c.id === shown }));
}
