import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

/**
 * How the UI reaches the app, wherever it runs. In the app's own window
 * that is Tauri's IPC, as always. In a browser on another device (remote
 * access, src-tauri/src/remote) it is the app's server: commands as POSTs
 * to /api/invoke/<name>, events over one WebSocket that picks up where it
 * left off after a drop. Everything else in the UI imports `invoke` and
 * `listen` from here, never from @tauri-apps/api.
 */

/**
 * Whether this is the app's own window (Tauri IPC to this app). A page the
 * app's server sent (`<meta name="divixi-served">`) talks to that server,
 * even inside a Divixi window showing a remote Divixi.
 */
export const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !document.querySelector('meta[name="divixi-served"]');

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
  return inTauri ? tauriInvoke<T>(cmd, args) : remoteInvoke<T>(cmd, args);
}

// ----- events -----

const handlers = new Map<string, Set<Handler>>();
let socket: WebSocket | null = null;
let lastSeq = 0;
let backoff = 1000;

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
  if (inTauri) return tauriListen<T>(event, handler);
  let set = handlers.get(event);
  if (!set) handlers.set(event, (set = new Set()));
  set.add(handler as Handler);
  connect();
  return () => {
    set!.delete(handler as Handler);
  };
}

/**
 * In a browser: find out whether this device is paired before the app
 * starts asking for things, and where the event stream stands.
 */
export async function signedIn(): Promise<boolean> {
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

/** Where a track file is served for previewing. */
export function previewBase(): string {
  if (!inTauri) return `${location.origin}/preview/`;
  return navigator.userAgent.includes("Windows") ? "http://preview.localhost/" : "preview://localhost/";
}

/** Where a design board's files are served. */
export function boardBase(): string {
  if (!inTauri) return `${location.origin}/board/`;
  return navigator.userAgent.includes("Windows") ? "http://board.localhost/" : "board://localhost/";
}
