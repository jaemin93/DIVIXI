/**
 * Sending a failed conductor turn again (Timeline.svelte), kept out of the
 * component so it runs under `node --test`.
 *
 * A turn goes again as it was first sent: its stored prompt holds the
 * words, any knowledge passages and the list of files (`store.sendAgain`).
 * Whether it already went again is read from the conversation: a later turn
 * with the same prompt, which a reload or another window sees too, or a
 * message in this window's line that says it is this one again.
 */

/** What this needs of a turn. */
export type Turn = { id: string; session: string; status: string; prompt: string; startedAt: number };

/** What this needs of a message in the line. */
export type Waiting = { again?: string; held?: boolean };

/** How the core heads the list of files it appends to a prompt (`ATTACH_MARK` in conductor.rs and store.svelte.ts). */
const MARK = "[attachments]";

/**
 * A stored prompt as `conductor_prompt` was handed it: the words, with any
 * knowledge passages, and the files. The core appends the files last, as
 * `- <path>` lines under the mark, so only a block like that at the very end
 * is taken for them. A worker's report or a schedule's message may quote the
 * mark anywhere, and none of those had files: they go whole.
 */
export function asSent(prompt: string): { text: string; files: string[] } {
  const at = prompt.lastIndexOf(`\n\n${MARK}\n`);
  const start = prompt.startsWith(`${MARK}\n`) && at < 0 ? 0 : at < 0 ? -1 : at + 2;
  if (start < 0) return { text: prompt, files: [] };
  const lines = prompt.slice(start + MARK.length + 1).split("\n");
  if (!lines.length || !lines.every((l) => l.startsWith("- ") && l.length > 2)) return { text: prompt, files: [] };
  return { text: prompt.slice(0, Math.max(0, start - 2)), files: lines.map((l) => l.slice(2).trim()) };
}

/** The later turn that carried `run`'s words again, typed or sent with the button. */
export function sentAgainAs<T extends Turn>(run: Turn, conversation: T[]): T | undefined {
  return conversation.find((r) => r.id !== run.id && r.session === run.session && r.startedAt > run.startedAt && r.prompt === run.prompt);
}

/**
 * Where sending `run` again stands: gone as a later turn, waiting in the
 * line for the turn in flight, held there (read back at startup, waiting
 * on the human), or not sent again at all.
 */
export function againState(run: Turn, conversation: Turn[], line: Waiting[]): "sent" | "waiting" | "held" | null {
  if (sentAgainAs(run, conversation)) return "sent";
  const queued = line.find((q) => q.again === run.id);
  if (queued) return queued.held ? "held" : "waiting";
  return null;
}

/**
 * Whether a turn is one to offer sending again: the conductor's, failed,
 * with words to send. A stop by the human ends a turn as done, unless the
 * agent ignored it and the app ended its session, which reads failed and is
 * offered too. `stale` lists how the turns nobody waits on any more begin
 * (a worker's permission question: the worker has moved on).
 */
export function offersSendAgain(run: Turn, stale: string[]): boolean {
  return run.session === "conductor" && run.status === "failed" && run.prompt.trim() !== "" && !stale.some((p) => run.prompt.startsWith(p));
}
