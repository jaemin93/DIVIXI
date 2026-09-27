/**
 * Messages typed while a turn is in flight.
 *
 * The composer never closes: a message sent during a turn joins a line and
 * goes out by itself when the turn ends, the way Claude Code's terminal
 * queues what you type while it is answering. The line is one flat list for
 * the whole app, keyed by conversation, so a message queued on one track
 * does not hold up another.
 *
 * The line outlives the app: it is written to a setting and read back at
 * startup. A message that came back that way is **held** — it is in the
 * line, in its place, but nothing sends it until the human says so. Words
 * they wrote days ago must not go to an agent just because the app opened.
 *
 * Everything here is pure: the store owns the list and these say what the
 * next list is. That keeps the ordering testable without a running app.
 */

import type { KbPick } from "./store.svelte";

/** A message waiting for the turn in flight to end. */
export type Queued = {
  /** Stable id, so the line can be shown and an item taken back out. */
  id: string;
  /** The conversation it belongs to: `track:<id>` or `artifact:<id>`. */
  key: string;
  /** Track id, or the artifact id when the conversation is an artifact's. */
  target: string;
  /** Agent chosen when it was sent; a later change does not move it. */
  agent: string;
  /** What was typed, without attachment or knowledge markup. */
  text: string;
  /** Attachment paths that went with it. */
  files: string[];
  /** Knowledge passages picked for it. */
  picks: KbPick[];
  /** Board items picked for it (artifact conversations only). */
  selected: string[];
  /** Unix milliseconds it was sent at. */
  at: number;
  /**
   * Waiting on the human, not on the turn: a message the app read back at
   * startup. It keeps its place in the line but is never sent by itself.
   */
  held?: boolean;
};

/** The waiting messages of one conversation, in the order they were sent. */
export function queuedFor(list: Queued[], key: string): Queued[] {
  return list.filter((q) => q.key === key);
}

/** The messages that will go out on their own, across every conversation. */
export function liveQueued(list: Queued[]): Queued[] {
  return list.filter((q) => !q.held);
}

/** The messages of one conversation that will go out on their own. */
export function liveQueuedFor(list: Queued[], key: string): Queued[] {
  return list.filter((q) => q.key === key && !q.held);
}

/** Put a message at the back of the line. */
export function pushQueued(list: Queued[], item: Queued): Queued[] {
  return [...list, item];
}

/**
 * Take the oldest sendable message of a conversation out of the line. Held
 * messages are passed over: they are the human's to release.
 */
export function nextQueued(list: Queued[], key: string): { next: Queued | undefined; rest: Queued[] } {
  const next = list.find((q) => q.key === key && !q.held);
  if (!next) return { next: undefined, rest: list };
  return { next, rest: list.filter((q) => q !== next) };
}

/**
 * Put a message back at the front of its conversation's sendable line —
 * what a send that could not start does, so the order it was typed in
 * still holds. It goes after any held messages, which are not in that line.
 */
export function unshiftQueued(list: Queued[], item: Queued): Queued[] {
  const at = list.findIndex((q) => q.key === item.key && !q.held);
  if (at < 0) return [...list, item];
  return [...list.slice(0, at), item, ...list.slice(at)];
}

/** Take one message back out of the line (the human cancelled it). */
export function dropQueued(list: Queued[], id: string): Queued[] {
  return list.filter((q) => q.id !== id);
}

/** Let a held message go: the human asked for it to be sent after all. */
export function releaseQueued(list: Queued[], id: string): Queued[] {
  return list.map((q) => (q.id === id ? { ...q, held: false } : q));
}

/** The conversations with something to send, each once. */
export function queuedKeys(list: Queued[]): string[] {
  return [...new Set(liveQueued(list).map((q) => q.key))];
}

/**
 * Read the line back from the setting it was written to. Anything that is
 * not a message as this version writes them is dropped rather than guessed
 * at, and everything that survives is held: the human sends it, not us.
 */
export function parseQueued(raw: string | null | undefined): Queued[] {
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  return parsed
    .filter((q): q is Record<string, unknown> => !!q && typeof q === "object")
    .filter((q) => typeof q.id === "string" && typeof q.key === "string" && typeof q.target === "string")
    .map((q) => ({
      id: q.id as string,
      key: q.key as string,
      target: q.target as string,
      agent: typeof q.agent === "string" ? q.agent : "",
      text: typeof q.text === "string" ? q.text : "",
      files: strings(q.files),
      picks: Array.isArray(q.picks) ? (q.picks as KbPick[]) : [],
      selected: strings(q.selected),
      at: typeof q.at === "number" ? q.at : Date.now(),
      held: true,
    }))
    .filter((q) => q.text.trim() || q.files.length || q.picks.length || q.selected.length);
}

/** Whether a run the human stopped: it ended, but on their Ctrl+C. */
export function wasStopped(run: { status: string; stopReason?: string }): boolean {
  return run.status === "done" && (run.stopReason ?? "").toLowerCase() === "cancelled";
}
