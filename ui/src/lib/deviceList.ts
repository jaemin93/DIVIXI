/**
 * The devices signed in to this Divixi, as Settings › Remote lists them.
 * Every pairing adds one and none leaves until it is dropped, so the list
 * only grows; it shows the few seen most lately, the rest a press away.
 * Apart from the component so it is tested.
 */

/** How many the list shows before "show all". */
export const RECENT = 5;

/**
 * The devices to show, most lately seen first: the `limit` newest, or all
 * of them when `all`; and how many are left out.
 */
export function recentDevices<T extends { last_seen: number }>(devices: T[], all: boolean, limit = RECENT): { shown: T[]; hidden: number } {
  const newest = [...devices].sort((a, b) => b.last_seen - a.last_seen);
  if (all || newest.length <= limit) return { shown: newest, hidden: 0 };
  return { shown: newest.slice(0, limit), hidden: newest.length - limit };
}
