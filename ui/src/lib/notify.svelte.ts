import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { routineOf } from "./noticeTarget";
import { addPending, dropPending, parsePending, pendingOf, type PendingReads } from "./noticeMemory";
import { inTauri, instanceId, instanceName } from "./ipc.svelte";

/**
 * What the human has to be told, in the bell at the top right. Two things
 * qualify, and nothing else:
 *
 * - **A decision card** waiting for an answer — a question the conductor put
 *   to them, or an agent asking permission. Its notice lives exactly as long
 *   as the card is open: answering or dismissing takes it out.
 * - **A routine that finished.** A routine runs with nobody watching and
 *   reports into no conversation, so without this there is nothing to notice
 *   that it is done. It is read, not answered: clicking takes them to the
 *   run, which is also what takes the notice out.
 *
 * A conductor reply, a worker's report, a finished turn or a failed one
 * still make no notice — the conversation already shows those. Nothing goes
 * to the system's notifications either (the user does not want them).
 *
 * So the badge is "things you have not dealt with": questions unanswered,
 * plus routine results unread.
 *
 * The list is this PC's, whatever Divixi is shown: localStorage, shared by
 * the app's webviews (Local and each instance) and kept in step through
 * `storage` events.
 */
export type Notice = {
  id: string;
  title: string;
  body: string;
  /** Unix ms of the latest. */
  at: number;
  /** Where it came from: an instance id (null for this PC) and a track. */
  instance: string | null;
  instanceName: string | null;
  track: string | null;
} & (
  | {
      kind: "decision";
      /** The decision card this notice takes the human to. */
      decision: number;
    }
  | {
      kind: "routine";
      /** The routine, and the run of it to open. */
      routine: string;
      run: string;
      /** How it ended, for the icon and the wording. */
      status: string;
    }
  | {
      /**
       * A notice the core keeps in its record until read (a run cut short
       * by a restart, a background job that died, a track where nothing
       * has run for hours). It is the core's, so it reaches every device
       * and survives this PC's storage going; reading it here tells the
       * core (`notice_read`), and that takes it off every bell.
       */
      kind: "server";
      notice: number;
      /** What the core calls it: `interrupted`, `job`, `stall`, … */
      what: string;
      /** The routine it is about, for a routine's run cut short: opening the notice opens it. */
      routine?: string;
    }
);

/** Narrowing helpers, so a caller never reads a field of the other kind. */
export const isDecision = (n: Notice): n is Notice & { kind: "decision"; decision: number } => n.kind === "decision";
export const isRoutine = (n: Notice): n is Notice & { kind: "routine"; routine: string; run: string; status: string } =>
  n.kind === "routine";
export const isServer = (n: Notice): n is Notice & { kind: "server"; notice: number; what: string; routine?: string } => n.kind === "server";

const INBOX = "divixi.inbox";
/** A notice to open in another webview: `{instance, track, decision, at}`. */
export const GOTO = "divixi.goto";
/**
 * Core notices read on this PC that their core has not confirmed yet, per
 * Divixi (`noticeMemory.ts`). Only that core can take a notice off every
 * device, and it may be out of reach when it is read: until it confirms,
 * the read is kept here, sent again, and the notice is not put back.
 */
const PENDING = "divixi.readPending";
const KEEP = 200;

/**
 * Notices from before this policy (replies, failed turns) are dropped, and
 * so is anything of a kind this build does not know. A decision notice
 * written before routines existed carries no `kind`; it is still a decision,
 * so it is read as one rather than thrown away.
 */
function clean(items: unknown): Notice[] {
  if (!Array.isArray(items)) return [];
  return items
    .map((x) => (!!x && typeof x === "object" && !("kind" in (x as object)) ? { ...(x as object), kind: "decision" } : x))
    .filter((x): x is Notice => {
      if (!x || typeof x !== "object") return false;
      const n = x as Notice;
      if (n.kind === "decision") return typeof n.decision === "number";
      if (n.kind === "routine") return typeof n.routine === "string" && typeof n.run === "string";
      if (n.kind === "server") return typeof n.notice === "number";
      return false;
    });
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

function loadPending(): PendingReads {
  try {
    return parsePending(localStorage.getItem(PENDING));
  } catch {
    return {};
  }
}

function changePending(f: (p: PendingReads) => PendingReads) {
  try {
    const next = f(loadPending());
    if (Object.keys(next).length) localStorage.setItem(PENDING, JSON.stringify(next));
    else localStorage.removeItem(PENDING);
  } catch {
    // No storage: the read is sent once, now.
  }
}

/**
 * Mark core notices read where they belong, from whichever webview: this
 * PC's core directly, another instance's through this PC's connection to
 * it -- not through that instance's webview, which may not be open. Kept
 * as pending until the core answers, so one out of reach now is read the
 * next time the bell hears from it, and its notice is not put back.
 */
export function readNotices(instance: string | null, ids: number[]) {
  if (!inTauri || !ids.length) return;
  changePending((p) => addPending(p, instance, ids));
  for (const id of ids) {
    const call =
      instance === null
        ? tauriInvoke("notice_read", { id })
        : tauriInvoke("instance_invoke", { id: instance, cmd: "notice_read", args: { id } });
    call.then(
      () => changePending((p) => dropPending(p, instance, [id])),
      () => {
        // Out of reach (or no such notice): kept, and tried again when its list is read.
      },
    );
  }
}

// Another webview of the app changed it.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === INBOX) inbox.items = load([]);
  });
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

const newId = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

/** A card is waiting for the human: into the bell's list. */
export function notifyDecision(decision: number, title: string, body: string, track: string | null) {
  if (!inTauri) return;
  body = clip(body.trim(), 400);
  change((xs) => {
    const rest = xs.filter((x) => !(isDecision(x) && x.decision === decision && x.instance === instanceId));
    return [
      { id: newId(), kind: "decision", decision, title, body, at: Date.now(), instance: instanceId, instanceName, track },
      ...rest,
    ];
  });
}

/**
 * A routine finished: into the bell's list. One notice per routine — a
 * routine run four times while nobody looked is one thing to read, not
 * four, and it is the latest run they would open.
 */
export function notifyRoutine(routine: string, run: string, status: string, title: string, body: string) {
  if (!inTauri) return;
  body = clip(body.trim(), 400);
  change((xs) => {
    const rest = xs.filter((x) => !(isRoutine(x) && x.routine === routine && x.instance === instanceId));
    return [
      {
        id: newId(),
        kind: "routine",
        routine,
        run,
        status,
        title,
        body,
        at: Date.now(),
        instance: instanceId,
        instanceName,
        track: null,
      },
      ...rest,
    ];
  });
}

/**
 * A card that is open has a notice, whether or not this window saw it
 * open: one asked in a session that is gone, or on a day this PC's
 * storage was cleared, is still waiting on the human. Dated when it was
 * asked, so it sorts where it belongs rather than as news.
 */
export function ensureDecisionNotice(decision: number, title: string, body: string, track: string | null, at: number) {
  if (!inTauri) return;
  body = clip(body.trim(), 400);
  change((xs) => {
    if (xs.some((x) => isDecision(x) && x.decision === decision && x.instance === instanceId)) return xs;
    const item: Notice = { id: newId(), kind: "decision", decision, title, body, at, instance: instanceId, instanceName, track };
    return [...xs, item].sort((a, b) => b.at - a.at);
  });
}

/** One of the core's notices, from a `notice` event or the list read on opening. */
export function notifyServer(n: { id: number; track: string; kind: string; title: string; body: string; data?: unknown; created_at: number }, trackName: string) {
  if (!inTauri) return;
  // Read here already; its core has not heard yet.
  if (pendingOf(loadPending(), instanceId).includes(n.id)) return;
  const title = trackName && !n.title.includes(trackName) ? `${n.title} · ${trackName}` : n.title;
  const body = clip(n.body.trim(), 400);
  const routine = routineOf(n.data);
  change((xs) => {
    if (xs.some((x) => isServer(x) && x.notice === n.id && x.instance === instanceId)) return xs;
    const item: Notice = { id: newId(), kind: "server", notice: n.id, what: n.kind, title, body, at: n.created_at, instance: instanceId, instanceName, track: n.track || null, ...(routine ? { routine } : {}) };
    return [item, ...xs].sort((a, b) => b.at - a.at);
  });
}

/**
 * Only these of the core's notices are unread: the rest were read
 * elsewhere, and any of them this window does not have yet goes on.
 */
export function keepServerNotices(unread: { id: number; track: string; kind: string; title: string; body: string; data?: unknown; created_at: number }[]) {
  if (!inTauri) return;
  const live = new Set(unread.map((n) => n.id));
  // Reads its core has not taken yet: sent again, and the rest are done.
  const pending = pendingOf(loadPending(), instanceId);
  const resend = pending.filter((id) => live.has(id));
  changePending((p) => dropPending(p, instanceId, pending.filter((id) => !live.has(id))));
  readNotices(instanceId, resend);
  change((xs) => xs.filter((x) => !isServer(x) || x.instance !== instanceId || live.has(x.notice)));
  for (const n of unread) notifyServer(n, "");
}

/** The core's notice was read (here or on another device). */
export function resolveServerNotice(id: number) {
  change((xs) => xs.filter((x) => !(isServer(x) && x.instance === instanceId && x.notice === id)));
}

/** Those cards are answered or dismissed: their notices are done. */
export function resolveDecisions(ids: Iterable<number>) {
  const done = new Set(ids);
  if (!done.size) return;
  change((xs) => xs.filter((x) => !(isDecision(x) && x.instance === instanceId && done.has(x.decision))));
}

/**
 * Only these cards of this Divixi are still open: any other notice of ours
 * has been answered (here, or elsewhere while we were away).
 */
export function keepOnlyOpen(open: Iterable<number>) {
  const live = new Set(open);
  // Routine notices are not cards and are not in that list; they are taken
  // out by being read, so they are left alone here.
  change((xs) => xs.filter((x) => !isDecision(x) || x.instance !== instanceId || live.has(x.decision)));
}
