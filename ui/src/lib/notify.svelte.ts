import { inTauri, instanceId, instanceName } from "./ipc.svelte";

/**
 * What wants the human: a decision or permission card, the conductor's
 * answer, a failed turn. Each goes into the bell's list at the top right
 * (as Kiro Crew's); nothing goes to the system's notifications (the user
 * does not want them).
 *
 * The list is this PC's, whatever Divixi is shown: localStorage, shared by
 * the app's webviews (Local and each instance) and kept in step through
 * `storage` events.
 */
export type NotifyKind = "decision" | "reply" | "failed";

export type Notice = {
  id: string;
  kind: NotifyKind;
  title: string;
  body: string;
  /** Unix ms of the latest. */
  at: number;
  read: boolean;
  /** The same notice again (title and body) counts up instead of adding. */
  count: number;
  /** Where it came from: an instance id (null for this PC) and a track. */
  instance: string | null;
  instanceName: string | null;
  track: string | null;
};

const INBOX = "divixi.inbox";
/** A notice to open in another webview: `{instance, track, at}`. */
export const GOTO = "divixi.goto";
const KEEP = 200;

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // No storage: it lasts until the page goes.
  }
}

export const inbox = $state<{ items: Notice[] }>({ items: load<Notice[]>(INBOX, []) });

/** Change the list: read it fresh (another webview may have added), change, keep. */
function change(f: (items: Notice[]) => Notice[]) {
  inbox.items = f(load<Notice[]>(INBOX, inbox.items)).slice(0, KEEP);
  save(INBOX, inbox.items);
}

export const markRead = (id: string) => change((xs) => xs.map((x) => (x.id === id ? { ...x, read: true } : x)));
export const markAllRead = () => change((xs) => xs.map((x) => ({ ...x, read: true })));
export const remove = (id: string) => change((xs) => xs.filter((x) => x.id !== id));
export const clearAll = () => change(() => []);

// Another webview of the app changed it.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === INBOX) inbox.items = load<Notice[]>(INBOX, []);
  });
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** Something for the human: into the bell's list. */
export function notify(kind: NotifyKind, title: string, body: string, track: string | null = null) {
  if (!inTauri) return;
  body = clip(body.trim(), 400);
  change((xs) => {
    const same = xs.findIndex((x) => x.title === title && x.body === body && x.instance === instanceId);
    if (same >= 0) {
      const again = { ...xs[same], at: Date.now(), read: false, count: xs[same].count + 1 };
      return [again, ...xs.filter((_, i) => i !== same)];
    }
    const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    return [{ id, kind, title, body, at: Date.now(), read: false, count: 1, instance: instanceId, instanceName, track }, ...xs];
  });
}
