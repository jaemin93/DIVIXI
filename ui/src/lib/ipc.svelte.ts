import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

/**
 * How the UI reaches the Divixi it shows. Everything else in the UI imports
 * `invoke` and `listen` from here, never from @tauri-apps/api.
 *
 * Showing this PC (Local), that is Tauri's IPC to this app. A remote
 * instance can be chosen in the header instead (as in Kiro Crew): its
 * commands then go through this app (`instance_invoke`), which holds the
 * instance's tunnel and tokens (src-tauri/src/remote/client.rs), and its
 * events come back as `instance-event`. A few commands stay with this app
 * whatever is shown ([`OWN`]). Each instance shown has a webview of its
 * own in the window (kept warm when another is shown), told its instance
 * before it runs (`window.__DIVIXI_INSTANCE__`); the window's first
 * webview is Local.
 */

/** Whether the page runs in the app's window (not a bare browser in development). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** The remote instance this webview shows, or null for this PC. */
export const instanceId: string | null = inTauri ? ((window as unknown as { __DIVIXI_INSTANCE__?: string }).__DIVIXI_INSTANCE__ ?? null) : null;

/** That instance's name as this PC calls it (for notifications). */
export const instanceName: string | null = instanceId ? ((window as unknown as { __DIVIXI_INSTANCE_NAME__?: string }).__DIVIXI_INSTANCE_NAME__ ?? null) : null;

/** This PC's own Divixi: its folders, terminal and files are at hand. */
export const local = inTauri && instanceId === null;

/** Show another instance (or this PC, null) in this window: its webview comes to the front. */
export function switchInstance(id: string | null): Promise<void> {
  return tauriInvoke("show_instance", { id });
}

/**
 * Commands that are this app's whatever is shown: the instances and this
 * PC's serving and GitHub account, the file picker (files here are taken
 * there, see `bring`), and opening a link (in this PC's browser).
 */
const OWN = new Set([
  "show_instance",
  "remote_hosts",
  "remote_host_save",
  "remote_host_delete",
  "remote_host_connect",
  "remote_host_disconnect",
  "remote_server_status",
  "remote_server_set",
  "remote_server_drop",
  // Installing divixi-server on a remote machine is this app's doing, even
  // when the page asking is that instance's own webview (Unreachable.svelte).
  "remote_server_check",
  "remote_server_install",
  "remote_server_start",
  "remote_server_cancel",
  "remote_server_mute",
  "github_account",
  "github_set_client_id",
  "github_login_start",
  "github_login_wait",
  "github_logout",
  "pick_files",
  "open_url",
  // A fault in this webview happened on this PC, so it belongs in this
  // PC's log and in the report made from it — not in the log of whatever
  // Divixi the webview happens to be showing.
  "ui_log",
]);

/** Call a command of the Divixi shown. */
export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (instanceId && !OWN.has(cmd)) return tauriInvoke<T>("instance_invoke", { id: instanceId, cmd, args: args ?? {} });
  return tauriInvoke<T>(cmd, args);
}

/**
 * Files of this PC (picked or dropped) as the Divixi being shown can use
 * them: the paths themselves here, copies kept there for an instance.
 */
export function bring(paths: string[]): Promise<string[]> {
  if (!instanceId || paths.length === 0) return Promise.resolve(paths);
  return tauriInvoke<string[]>("instance_upload", { id: instanceId, paths });
}

// ----- events -----

type Event<T> = { event: string; id: number; payload: T };
type Handler = (e: Event<unknown>) => void;

const handlers = new Map<string, Set<Handler>>();

/** The chosen instance's events, as this app passes them on. */
let relaying = false;
function relay() {
  if (relaying) return;
  relaying = true;
  void tauriListen<{ id: string; frame: { seq?: number; event?: string; payload?: unknown; reset?: boolean } }>("instance-event", (e) => {
    const { id, frame } = e.payload;
    if (id !== instanceId) return;
    // Too far behind to catch up event by event: start over from its state.
    if (frame.reset) {
      location.reload();
      return;
    }
    const set = frame.event ? handlers.get(frame.event) : undefined;
    if (!set) return;
    for (const h of set) h({ event: frame.event!, id: frame.seq ?? 0, payload: frame.payload });
  });
}

/** Listen to an event of the Divixi shown; the returned function stops listening. */
export async function listen<T>(event: string, handler: (e: Event<T>) => void): Promise<() => void> {
  if (!instanceId) return tauriListen<T>(event, handler);
  let set = handlers.get(event);
  if (!set) handlers.set(event, (set = new Set()));
  set.add(handler as Handler);
  relay();
  return () => {
    set!.delete(handler as Handler);
  };
}

/** Save a track's file on this PC (a save dialog here, whichever Divixi is shown). */
export function saveAs(track: string, path: string): Promise<string | null> {
  if (!instanceId) return tauriInvoke<string | null>("workspace_save_as", { track, path });
  return tauriInvoke<string | null>("instance_save_as", { id: instanceId, track, path });
}

/** An instance's link came up or went down (this app's event, whatever is shown). */
export function onInstanceStatus(handler: (s: { id: string; online: boolean }) => void): Promise<() => void> {
  return tauriListen<{ id: string; online: boolean }>("instance-status", (e) => handler(e.payload));
}

/**
 * How an install of divixi-server on a remote machine is going (this app's
 * event, whatever is shown -- as `onInstanceStatus`, not through the chosen
 * instance's relay, which is exactly what may not be there yet).
 */
export function onServerInstall(handler: (p: ServerInstall) => void): Promise<() => void> {
  return tauriListen<ServerInstall>("server_install", (e) => handler(e.payload));
}

/** Mirrors the `server_install` event payload (remote/install.rs). */
export type ServerInstall = {
  host: string;
  phase: "checking" | "downloading" | "verifying" | "installing" | "starting" | "done" | "failed" | "cancelled";
  /** Where it had got to, when `phase` is "failed". */
  at: ServerInstall["phase"] | null;
  received: number;
  total: number | null;
  route: "remote" | "ssh" | null;
  release: string | null;
  error: string | null;
};

/** Why the chosen instance could not be reached, if it could not. */
export const instance = $state<{ error: string }>({ error: "" });

/** Reach the chosen instance before the app asks it for anything. */
export async function ready(): Promise<boolean> {
  if (!instanceId) return true;
  try {
    await tauriInvoke("remote_host_connect", { id: instanceId });
    return true;
  } catch (err) {
    instance.error = String(err);
    return false;
  }
}

/** An instance's files come through this app, under `/@<id>/`. */
const via = () => (instanceId ? `@${encodeURIComponent(instanceId)}/` : "");

/** Where a track file is served for previewing. */
export function previewBase(): string {
  return (navigator.userAgent.includes("Windows") ? "http://preview.localhost/" : "preview://localhost/") + via();
}

/** Where a design board's files are served. */
export function boardBase(): string {
  return (navigator.userAgent.includes("Windows") ? "http://board.localhost/" : "board://localhost/") + via();
}
