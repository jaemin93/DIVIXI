import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

/**
 * How the UI reaches the app, wherever it runs. In the app's own window
 * that is Tauri's IPC, as always. In a browser on another device (remote
 * access, src-tauri/src/remote) it is the app's server: commands as POSTs
 * to /api/invoke/<name>, events over one WebSocket that picks up where it
 * left off after a drop. Everything else in the UI imports `invoke` and
 * `listen` from here, never from @tauri-apps/api.
 *
 * In the app's window a remote instance can be chosen in the header (as
 * Kiro Crew's): the window then shows that Divixi. Its commands go through
 * this app (`instance_invoke`), which holds the instance's tokens and
 * tunnel, and its events come back as `instance-event`; a few commands
 * stay with this app (the instance list itself). Switching reloads the page.
 */

/**
 * Whether this is the app's own window (Tauri IPC to this app). A page the
 * app's server sent (`<meta name="divixi-served">`) talks to that server,
 * even inside a Divixi window showing a remote Divixi.
 */
export const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !document.querySelector('meta[name="divixi-served"]');

const INSTANCE_KEY = "divixi.instance";

/** The remote instance this window shows, or null for this PC. */
export const instanceId: string | null = (() => {
  if (!inTauri) return null;
  try {
    return sessionStorage.getItem(INSTANCE_KEY);
  } catch {
    return null;
  }
})();

/** This PC's own Divixi: its folders, terminal and files are at hand. */
export const local = inTauri && instanceId === null;

/** Show another instance (or this PC, null) in this window. */
export function switchInstance(id: string | null) {
  try {
    if (id) sessionStorage.setItem(INSTANCE_KEY, id);
    else sessionStorage.removeItem(INSTANCE_KEY);
  } catch {
    // No storage: stay where we are.
    return;
  }
  location.reload();
}

/** Commands that are this app's even while an instance is shown. */
const OWN = new Set(["remote_hosts", "remote_host_save", "remote_host_delete", "remote_host_connect", "remote_host_disconnect", "pick_files"]);

/**
 * Files of this PC (picked or dropped) as the Divixi being shown can use
 * them: the paths themselves here, copies kept there for an instance.
 */
export function bring(paths: string[]): Promise<string[]> {
  if (!instanceId || paths.length === 0) return Promise.resolve(paths);
  return tauriInvoke<string[]>("instance_upload", { id: instanceId, paths });
}

/** Where a browser's connection to the app stands. */
export const remote = $state<{ state: "connecting" | "online" | "reconnecting" | "signed-out" | "dropped" }>({
  state: inTauri ? "online" : "connecting",
});

type Event<T> = { event: string; id: number; payload: T };
type Handler = (e: Event<unknown>) => void;

// ----- commands -----

async function post(path: string, body?: unknown): Promise<Response> {
  return fetch(path, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-Divixi": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

/** One try at new cookies; false when the device must pair again. */
let refreshing: Promise<boolean> | null = null;
function refresh(): Promise<boolean> {
  refreshing ??= post("/auth/refresh")
    .then(async (r) => {
      if (r.ok) return true;
      const why = await r.json().catch(() => ({}));
      remote.state = why?.error === "dropped" ? "dropped" : "signed-out";
      return false;
    })
    .catch(() => false)
    .finally(() => {
      refreshing = null;
    });
  return refreshing;
}

async function remoteInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  let res = await post(`/api/invoke/${encodeURIComponent(cmd)}`, args ?? {});
  if (res.status === 401 && (await refresh())) res = await post(`/api/invoke/${encodeURIComponent(cmd)}`, args ?? {});
  const body = await res.json().catch(() => ({ error: `${res.status} ${res.statusText}` }));
  if (res.status === 401) {
    remote.state = body?.error === "dropped" ? "dropped" : "signed-out";
  }
  if (!res.ok || "error" in body) throw body.error ?? `${res.status}`;
  return body.ok as T;
}

/** Call an app command: through IPC in the app, through the server in a browser. */
export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return remoteInvoke<T>(cmd, args);
  if (instanceId && !OWN.has(cmd)) return tauriInvoke<T>("instance_invoke", { id: instanceId, cmd, args: args ?? {} });
  return tauriInvoke<T>(cmd, args);
}

// ----- events -----

const handlers = new Map<string, Set<Handler>>();
let socket: WebSocket | null = null;
let lastSeq = 0;
let backoff = 1000;

/** The chosen instance's events, as this app passes them on. */
let relaying = false;
function relay() {
  if (relaying) return;
  relaying = true;
  void tauriListen<{ id: string; frame: { seq?: number; event?: string; payload?: unknown; reset?: boolean } }>("instance-event", (e) => {
    const { id, frame } = e.payload;
    if (id !== instanceId) return;
    if (frame.reset) {
      location.reload();
      return;
    }
    const set = frame.event ? handlers.get(frame.event) : undefined;
    if (!set) return;
    for (const h of set) h({ event: frame.event!, id: frame.seq ?? 0, payload: frame.payload });
  });
}

function connect() {
  if (socket || remote.state === "signed-out" || remote.state === "dropped") return;
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${scheme}://${location.host}/api/events?since=${lastSeq}`);
  socket = ws;
  ws.onopen = () => {
    remote.state = "online";
    backoff = 1000;
  };
  ws.onmessage = (m) => {
    let frame: { seq?: number; event?: string; payload?: unknown; reset?: boolean };
    try {
      frame = JSON.parse(String(m.data));
    } catch {
      return;
    }
    // Too far behind to catch up event by event: start over from the app's state.
    if (frame.reset) {
      location.reload();
      return;
    }
    if (typeof frame.seq === "number") lastSeq = frame.seq;
    const set = frame.event ? handlers.get(frame.event) : undefined;
    if (!set) return;
    for (const h of set) h({ event: frame.event!, id: frame.seq ?? 0, payload: frame.payload });
  };
  ws.onclose = async () => {
    socket = null;
    if (remote.state === "signed-out" || remote.state === "dropped") return;
    remote.state = "reconnecting";
    // A refused upgrade looks like a close: the cookie may have lapsed.
    const me = await fetch("/api/me", { credentials: "same-origin" }).catch(() => null);
    if (me?.status === 401 && !(await refresh())) return;
    setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 10_000);
  };
}

/** Listen to an app event; the returned function stops listening. */
export async function listen<T>(event: string, handler: (e: Event<T>) => void): Promise<() => void> {
  if (inTauri && !instanceId) return tauriListen<T>(event, handler);
  let set = handlers.get(event);
  if (!set) handlers.set(event, (set = new Set()));
  set.add(handler as Handler);
  if (inTauri) relay();
  else connect();
  return () => {
    set!.delete(handler as Handler);
  };
}

/**
 * In a browser: find out whether this device is paired before the app
 * starts asking for things, and where the event stream stands.
 */
export async function signedIn(): Promise<boolean> {
  if (inTauri && instanceId) {
    // Reach the chosen instance before the app asks it for anything.
    try {
      await tauriInvoke("remote_host_connect", { id: instanceId });
      return true;
    } catch (err) {
      instance.error = String(err);
      return false;
    }
  }
  if (inTauri) return true;
  let me = await fetch("/api/me", { credentials: "same-origin" }).catch(() => null);
  if (me?.status === 401 && (await refresh())) me = await fetch("/api/me", { credentials: "same-origin" }).catch(() => null);
  if (!me || !me.ok) {
    if (remote.state === "connecting") remote.state = "signed-out";
    return false;
  }
  const body = await me.json().catch(() => ({}));
  lastSeq = Number(body?.last ?? 0);
  remote.state = "online";
  return true;
}

/** Forget this device (its cookies go; the PC's list drops it). */
export async function signOut() {
  await post("/auth/logout").catch(() => null);
  remote.state = "signed-out";
  socket?.close();
}

/** Why the chosen instance could not be reached, if it could not. */
export const instance = $state<{ error: string }>({ error: "" });

/** An instance's files come through this app, under `/@<id>/`. */
const via = () => (instanceId ? `@${encodeURIComponent(instanceId)}/` : "");

/** Where a track file is served for previewing. */
export function previewBase(): string {
  if (!inTauri) return `${location.origin}/preview/`;
  return (navigator.userAgent.includes("Windows") ? "http://preview.localhost/" : "preview://localhost/") + via();
}

/** Where a design board's files are served. */
export function boardBase(): string {
  if (!inTauri) return `${location.origin}/board/`;
  return (navigator.userAgent.includes("Windows") ? "http://board.localhost/" : "board://localhost/") + via();
}
