import { inTauri, instanceId, instanceName } from "./ipc.svelte";

/**
 * What wants the human: a decision card (a question the conductor put to
 * them, or an agent asking permission). Nothing else. The bell's list at the
 * top right is only a way to reach a card that waits for an answer, so a
 * conductor reply, a worker's report, a finished turn or a failed one make no
 * notice — the conversation itself already shows those. Nothing goes to the
 * system's notifications either (the user does not want them).
 *
 * A notice lives exactly as long as its card is open: answering or
 * dismissing the card takes the notice out of the list, so the badge counts
 * decisions still waiting.
 *
 * The list is this PC's, whatever Divixi is shown: localStorage, shared by
 * the app's webviews (Local and each instance) and kept in step through
 * `storage` events.
 */
export type Notice = {
  id: string;
  /** The decision card this notice takes the human to. */
  decision: number;
  title: string;
  body: string;
  /** Unix ms of the latest. */
  at: number;
  /** Where it came from: an instance id (null for this PC) and a track. */
  instance: string | null;
  instanceName: string | null;
  track: string | null;
};

const INBOX = "divixi.inbox";
/** A notice to open in another webview: `{instance, track, decision, at}`. */
export const GOTO = "divixi.goto";
const KEEP = 200;

/** Notices from before this policy (replies, failed turns) are dropped. */
function clean(items: unknown): Notice[] {
  if (!Array.isArray(items)) return [];
  return items.filter((x): x is Notice => !!x && typeof x === "object" && typeof (x as Notice).decision === "number");
}

function load(fallback: Notice[]): Notice[] {
  try {
    const v = localStorage.getItem(INBOX);
    return v === null ? fallback : clean(JSON.parse(v));
  } catch {
    return fallback;
  }
}

function save(items: Notice[]) {
  try {
    localStorage.setItem(INBOX, JSON.stringify(items));
  } catch {
    // No storage: it lasts until the page goes.
  }
}

export const inbox = $state<{ items: Notice[] }>({ items: load([]) });

/** Change the list: read it fresh (another webview may have added), change, keep. */
function change(f: (items: Notice[]) => Notice[]) {
  inbox.items = f(load(inbox.items)).slice(0, KEEP);
  save(inbox.items);
}

export const remove = (id: string) => change((xs) => xs.filter((x) => x.id !== id));
export const clearAll = () => change(() => []);

// Another webview of the app changed it.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === INBOX) inbox.items = load([]);
  });
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** A card is waiting for the human: into the bell's list. */
export function notifyDecision(decision: number, title: string, body: string, track: string | null) {
  if (!inTauri) return;
  body = clip(body.trim(), 400);
  change((xs) => {
    const rest = xs.filter((x) => !(x.decision === decision && x.instance === instanceId));
    const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    return [{ id, decision, title, body, at: Date.now(), instance: instanceId, instanceName, track }, ...rest];
  });
}

/** Those cards are answered or dismissed: their notices are done. */
export function resolveDecisions(ids: Iterable<number>) {
  const done = new Set(ids);
  if (!done.size) return;
  change((xs) => xs.filter((x) => !(x.instance === instanceId && done.has(x.decision))));
}

/**
 * Only these cards of this Divixi are still open: any other notice of ours
 * has been answered (here, or elsewhere while we were away).
 */
export function keepOnlyOpen(open: Iterable<number>) {
  const live = new Set(open);
  change((xs) => xs.filter((x) => x.instance !== instanceId || live.has(x.decision)));
}
