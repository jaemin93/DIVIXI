/**
 * What the bell remembers so a core notice read here stays read: reads
 * not yet confirmed by the core they belong to, keyed by the Divixi the
 * notice came from (an instance id, or "" for this PC). Kept apart from
 * the bell's store so it can be tested on its own.
 */

/** Core notices read here, per Divixi, that its core has not confirmed yet. */
export type PendingReads = Record<string, number[]>;

/** The key a Divixi goes by in these lists. */
export const where = (instance: string | null) => instance ?? "";

/** A pending-reads list read back; anything that is not one is empty. */
export function parsePending(raw: string | null): PendingReads {
  try {
    const v: unknown = raw ? JSON.parse(raw) : null;
    if (!v || typeof v !== "object" || Array.isArray(v)) return {};
    const out: PendingReads = {};
    for (const [k, ids] of Object.entries(v as Record<string, unknown>)) {
      const nums = Array.isArray(ids) ? ids.filter((x): x is number => typeof x === "number") : [];
      if (nums.length) out[k] = nums;
    }
    return out;
  } catch {
    return {};
  }
}

/** These were read: keep them until their core says so. */
export function addPending(p: PendingReads, instance: string | null, ids: number[]): PendingReads {
  const k = where(instance);
  const merged = [...new Set([...(p[k] ?? []), ...ids])];
  return merged.length ? { ...p, [k]: merged } : p;
}

/** Their core has them read (or has no such notice unread): nothing left to send. */
export function dropPending(p: PendingReads, instance: string | null, ids: Iterable<number>): PendingReads {
  const k = where(instance);
  const gone = new Set(ids);
  const left = (p[k] ?? []).filter((id) => !gone.has(id));
  const { [k]: _, ...rest } = p;
  return left.length ? { ...rest, [k]: left } : rest;
}

/** The ones still to send to that Divixi's core. */
export const pendingOf = (p: PendingReads, instance: string | null): number[] => p[where(instance)] ?? [];
