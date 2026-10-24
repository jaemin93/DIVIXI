/**
 * Worker runs this page shows working that the core has no turn of in
 * flight.
 *
 * The page learns that a turn ended from the turn's last event. An event
 * that never arrives (a remote instance's link went down as it was sent)
 * leaves the run working here for as long as the page lives: its worker's
 * dot pulses, Stop is offered and refused, Delete is held back. The worker
 * sessions read back from the core name the run each worker has in flight,
 * so a run working here and not there is worth asking the store about. The
 * store has the ending if there is one: an event is written before it is
 * sent.
 *
 * Pure, so it can be tested without a running app.
 */

/** One worker's session as the core reports it: the run it has in flight, if any. */
export type InFlight = { name: string; run: string | null };

/** The runs, of those given, that are working here and not in flight in the core. */
export function notInFlight<R extends { id: string; track: string; session: string; status: string }>(runs: R[], sessions: Record<string, InFlight[]>): R[] {
  return runs.filter((r) => (r.status === "running" || r.status === "connecting") && sessions[r.track]?.find((w) => w.name === r.session)?.run !== r.id);
}
