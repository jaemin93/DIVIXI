/**
 * Where a webview is on its way to showing its Divixi, kept apart from the
 * runes so it runs under `node --test`.
 *
 * Every instance shown has a webview of its own, and a new one starts with
 * an empty store: no tracks, because none have been asked for yet. Drawn as
 * it is, that is the first-track screen, which is wrong -- the instance may
 * well have tracks, they just have not arrived. So the page says which of
 * these it is in, and only `ready` with no tracks is "nothing here yet":
 *
 *   connecting  the remote instance is being reached (`ready()` in ipc)
 *   loading     reached; tracks and settings are being read (`restore()`)
 *   ready       read; what the store holds is what there is
 *   failed      could not be reached or read; `error` says why
 *
 * Waiting has a deadline, so the spinner never turns for ever. Past it a
 * remote instance fails (the Unreachable page, with its retry and its way
 * back to Local) and this PC simply shows what it has. A remote that still
 * answers after its deadline is let through: the wait was long, not lost.
 */

export type Phase = "connecting" | "loading" | "ready" | "failed";

export type Startup = {
  phase: Phase;
  /** Why it failed; empty otherwise. */
  error: string;
  /** The deadline passed, so `failed` may yet give way to an answer. */
  late: boolean;
};

export type StartupEvent =
  | { type: "connected" }
  | { type: "loaded"; ok: boolean; error?: string }
  | { type: "failed"; error: string }
  | { type: "deadline"; error: string };

/** How long reaching and reading an instance may take before giving up on it. */
export const DEADLINE_MS = 30_000;

/** How long before the waiting screen says it is taking a while. */
export const SLOW_MS = 8_000;

/** A remote instance starts by being reached; this PC and a phone's browser are already there. */
export function initial(remote: boolean): Startup {
  return { phase: remote ? "connecting" : "loading", error: "", late: false };
}

/** The next state. `remote` is whether the page shows another PC's instance. */
export function next(s: Startup, e: StartupEvent, remote: boolean): Startup {
  // Settled: nothing more changes it, except an answer after the deadline.
  if (s.phase === "ready") return s;
  if (s.phase === "failed" && !s.late) return s;
  switch (e.type) {
    case "connected":
      if (s.phase === "connecting" || s.late) return { phase: "loading", error: "", late: false };
      return s;
    case "loaded":
      if (e.ok || !remote) return { phase: "ready", error: "", late: false };
      return { phase: "failed", error: e.error || s.error, late: false };
    case "failed":
      return { phase: "failed", error: e.error, late: false };
    case "deadline":
      if (s.phase === "failed") return s;
      // This PC: stop waiting and show what is there.
      if (!remote) return { phase: "ready", error: "", late: false };
      return { phase: "failed", error: e.error, late: true };
  }
}

/** Still on the way: the connecting screen, not the app and not its empty state. */
export function waiting(phase: Phase): boolean {
  return phase === "connecting" || phase === "loading";
}

/** Nothing here yet, truly: read in full and no tracks. Only then the first-track screen. */
export function empty(phase: Phase, tracks: number): boolean {
  return phase === "ready" && tracks === 0;
}

/** Waited long enough that the screen should say so (and offer a way out). */
export function slow(phase: Phase, since: number, now: number): boolean {
  return waiting(phase) && now - since >= SLOW_MS;
}
