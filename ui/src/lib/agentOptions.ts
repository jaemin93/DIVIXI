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
