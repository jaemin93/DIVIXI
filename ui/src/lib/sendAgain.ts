/**
 * Sending a failed conductor turn again (Timeline.svelte), kept out of the
 * component so it runs under `node --test`.
 *
 * A turn goes again as it was first sent: its stored prompt holds the
 * words, any knowledge passages and the list of files (`store.sendAgain`).
 * Whether it already went again is read from the conversation itself, so a
 * reload or another window sees the same: a later turn with the same
 * prompt, or a message waiting in the line that says it is this one again.
 */

/** What this needs of a turn. */
export type Turn = { id: string; session: string; status: string; prompt: string; startedAt: number };

/** What this needs of a message waiting in the line. */
export type Waiting = { again?: string };

/** The later turn that carried `run`'s words again, typed or sent with the button. */
export function sentAgainAs<T extends Turn>(run: Turn, conversation: T[]): T | undefined {
  return conversation.find((r) => r.id !== run.id && r.session === run.session && r.startedAt > run.startedAt && r.prompt === run.prompt);
}

/** Whether `run` went again already, or is waiting in the line to. */
export function sentAgain(run: Turn, conversation: Turn[], line: Waiting[]): boolean {
  return line.some((q) => q.again === run.id) || !!sentAgainAs(run, conversation);
}

/**
 * Whether a turn is one to offer sending again: the conductor's, failed on
 * the agent's side (a stop by the human ends a turn as done), with words to
 * send. `stale` lists how the turns nobody waits on any more begin (a
 * worker's permission question: the worker has moved on).
 */
export function offersSendAgain(run: Turn, stale: string[]): boolean {
  return run.session === "conductor" && run.status === "failed" && run.prompt.trim() !== "" && !stale.some((p) => run.prompt.startsWith(p));
}
