import { instance, instanceId } from "./ipc.svelte";
import { initial, next, type StartupEvent } from "./startup";

/**
 * This webview's way to its Divixi, as the screen follows it (startup.ts
 * has the rules; main.ts sends the events). `since` is when it began, for
 * saying that it is taking a while.
 */
export const startup = $state({ ...initial(instanceId !== null), since: Date.now() });

export function step(e: StartupEvent): void {
  const s = next(startup, e, instanceId !== null);
  startup.phase = s.phase;
  startup.error = s.error;
  startup.late = s.late;
  // A remote's failure is the Unreachable page, which reads `instance.error`;
  // an answer after the deadline takes the page away again.
  if (instanceId !== null) instance.error = s.phase === "failed" ? s.error : "";
}
