/**
 * The order of the instances in the switcher.
 *
 * One rule: the order the human dragged them into. Local is an instance
 * like any other and can sit anywhere; nothing jumps to the front because
 * it is on screen. The order is this PC's (localStorage, shared by every
 * webview of the app), and what is stored is only a list of ids, so the
 * instances themselves stay the backend's business.
 *
 * `arrange` is the whole of the sorting: the stored order, minus what is
 * gone, plus whatever is new at the end. Deleting an instance therefore
 * cannot leave a hole, and a new remote lands last. Pinning used to be
 * the other half of this and is gone; `savedOrder` reads what a pinned
 * setup implied so the order does not shuffle under people who had one.
 */

/** An instance in the order; `null` is Local, this PC's own Divixi. */
export type InstanceRef = string | null;

/** Where the order is kept. */
export const ORDER_KEY = "divixi.order";
/** What pinning left behind: read once by `savedOrder`, then deleted. */
export const PINS_KEY = "divixi.pins";
/** The old "keep tab order" switch, gone with pinning: deleted, never read. */
export const STABLE_KEY = "divixi.stableOrder";

function parse(raw: string | null): unknown {
  if (raw === null) return undefined;
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return undefined;
  }
}

/** A stored order: ids and at most one `null`, anything else thrown out. */
function refs(raw: string | null): InstanceRef[] | undefined {
  const v = parse(raw);
  if (!Array.isArray(v)) return undefined;
  return v.filter((e): e is InstanceRef => e === null || typeof e === "string");
}

/**
 * The order to start from: the stored one, or — for a setup from before
 * dragging, which had pins instead — Local and then what was pinned, so
 * the top of the list stays what those people were looking at. Neither:
 * an empty order, which `arrange` fills with Local and every host.
 */
export function savedOrder(order: string | null, pins: string | null): InstanceRef[] {
  const stored = refs(order);
  if (stored) return stored;
  const pinned = refs(pins)?.filter((p): p is string => typeof p === "string");
  return pinned?.length ? [null, ...pinned] : [];
}

/**
 * The list to draw: `saved` kept to the instances that exist, with Local
 * and any host it does not mention added. `hostIds` is the backend's own
 * order, newest last, so a remote just added shows up at the end.
 */
export function arrange(saved: InstanceRef[], hostIds: string[]): InstanceRef[] {
  const known = new Set(hostIds);
  const out: InstanceRef[] = [];
  const seen = new Set<string>();
  let local = false;
  for (const e of saved) {
    if (e === null) {
      if (local) continue;
      local = true;
      out.push(null);
    } else if (known.has(e) && !seen.has(e)) {
      seen.add(e);
      out.push(e);
    }
  }
  if (!local) out.unshift(null);
  for (const id of hostIds) if (!seen.has(id)) out.push(id);
  return out;
}

/**
 * `order` with the item at `from` taken out and put back at `to`, where
 * `to` counts the gaps of the list as it stands: 0 is before the first
 * item, `order.length` after the last. Dropping an item on either side of
 * itself leaves the order alone.
 */
export function move(order: InstanceRef[], from: number, to: number): InstanceRef[] {
  if (from < 0 || from >= order.length) return order;
  const at = Math.max(0, Math.min(order.length, to));
  if (at === from || at === from + 1) return order;
  const out = order.slice();
  const [item] = out.splice(from, 1);
  out.splice(at > from ? at - 1 : at, 0, item);
  return out;
}

/** One step up (-1) or down (+1), for the keyboard; the ends hold. */
export function nudge(order: InstanceRef[], from: number, step: number): InstanceRef[] {
  const to = from + step;
  if (from < 0 || from >= order.length || to < 0 || to >= order.length) return order;
  const out = order.slice();
  out[from] = order[to];
  out[to] = order[from];
  return out;
}
