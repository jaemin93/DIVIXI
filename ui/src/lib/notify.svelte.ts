import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { inTauri, instanceId, instanceName } from "./ipc.svelte";

/**
 * What wants the human: a decision or permission card, the conductor's
 * answer, a failed turn. Each goes into the bell's list at the top right
 * (as Kiro Crew's), always. The system's notifications are off unless
 * turned on per kind in the settings (the user does not want them by
 * default); when on, they come only while the human is not looking: the
 * window not focused (hidden in the tray, behind another app), or this
 * webview a remote instance kept warm behind the one on screen. A remote
 * instance's notice says which instance.
 *
 * The list and which kinds are on are this PC's, whatever Divixi is shown:
 * localStorage, shared by the app's webviews (Local and each instance) and
 * kept in step through `storage` events.
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

/** v2: system notifications became opt-in; earlier stored choices were the old all-on defaults. */
const PREFS = "divixi.notify.v2";
const INBOX = "divixi.inbox";
/** A notice to open in another webview: `{instance, track, at}`. */
export const GOTO = "divixi.goto";
const KEEP = 200;
/** System notifications per kind: all off until the user turns one on. */
const DEFAULTS: Record<NotifyKind, boolean> = { decision: false, reply: false, failed: false };

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

export const notifyPrefs = $state<Record<NotifyKind, boolean>>({ ...DEFAULTS, ...load(PREFS, {}) });
export const inbox = $state<{ items: Notice[] }>({ items: load<Notice[]>(INBOX, []) });

export function setNotify(kind: NotifyKind, on: boolean) {
  notifyPrefs[kind] = on;
  save(PREFS, notifyPrefs);
}

/** Change the list: read it fresh (another webview may have added), change, keep. */
function change(f: (items: Notice[]) => Notice[]) {
  inbox.items = f(load<Notice[]>(INBOX, inbox.items)).slice(0, KEEP);
  save(INBOX, inbox.items);
}

export const markRead = (id: string) => change((xs) => xs.map((x) => (x.id === id ? { ...x, read: true } : x)));
export const markAllRead = () => change((xs) => xs.map((x) => ({ ...x, read: true })));
export const remove = (id: string) => change((xs) => xs.filter((x) => x.id !== id));
export const clearAll = () => change(() => []);

// Another webview of the app changed them.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === PREFS) Object.assign(notifyPrefs, { ...DEFAULTS, ...load(PREFS, {}) });
    if (e.key === INBOX) inbox.items = load<Notice[]>(INBOX, []);
  });
}

/** Whether the human is looking at this webview now. */
function looking(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

let allowed: boolean | null = null;

async function mayNotify(): Promise<boolean> {
  if (allowed !== null) return allowed;
  allowed = (await isPermissionGranted().catch(() => false)) || (await requestPermission().catch(() => "denied")) === "granted";
  return allowed;
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/**
 * Something for the human: into the bell's list, and to the system when
 * that kind is turned on and they are not looking. `test`: the settings'
 * test button, which reaches the system even while looking, if any kind
 * is on there.
 */
export async function notify(kind: NotifyKind, title: string, body: string, track: string | null = null, opts: { test?: boolean } = {}) {
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
  const system = opts.test ? Object.values(notifyPrefs).some(Boolean) : notifyPrefs[kind] && !looking();
  if (!system) return;
  if (!(await mayNotify())) return;
  const where = instanceName ? `[${instanceName}] ` : "";
  sendNotification({ title: where + title, body: clip(body, 180) });
}
