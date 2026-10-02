/**
 * Reaching this Divixi from a phone's browser.
 *
 * The third way the UI talks to a Divixi, after Tauri's IPC (this PC's own
 * window) and `instance_invoke` (another PC's). `ipc.svelte.ts` picks between
 * them; everything HTTP is here, so the seam stays one file wide.
 *
 * Tokens live in `localStorage` and travel as `Authorization: Bearer`. Not a
 * cookie, and that is the load-bearing choice: a cookie would be attached to
 * every request to this origin whoever caused it, and the reason this server
 * has no CSRF problem is that there is no ambient credential for another page
 * to ride. The `X-Divixi` header on writes is the second half of that — a
 * form post from elsewhere cannot add it, and a `fetch` that does triggers a
 * preflight nothing answers.
 */

import { pairingName } from "./deviceName";

const ACCESS = "divixi.access";
const REFRESH = "divixi.refresh";

/** Everything is on the origin that served this page. */
const base = () => location.origin;

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    // Private browsing, or storage disabled. Treated as signed out rather
    // than crashing on the first read.
    return null;
  }
}

function keep(access: string, refresh: string): void {
  try {
    localStorage.setItem(ACCESS, access);
    localStorage.setItem(REFRESH, refresh);
  } catch {
    /* nothing to do: the session lasts as long as the page does */
  }
}

export function forget(): void {
  try {
    localStorage.removeItem(ACCESS);
    localStorage.removeItem(REFRESH);
  } catch {
    /* already gone */
  }
}

export const signedIn = (): boolean => read(ACCESS) !== null;

/** Whether a scan has just put a pairing token in the address bar. */
export const pairingInUrl = (): boolean => new URL(location.href).searchParams.has("token");

/** The access token, for the event socket to carry as a subprotocol. */
export const access = (): string | null => read(ACCESS);

type Tokens = { access: string; refresh: string };

/**
 * Redeem a pairing token for this device's own.
 *
 * The same endpoint the Divixi app uses, because the scope and the span are
 * signed into the pairing token rather than chosen by whoever redeems it —
 * so a phone's link posted here comes back with a phone's permissions and
 * there is no second endpoint to keep in step.
 */
async function redeem(token: string): Promise<void> {
  const res = await fetch(`${base()}/auth/token`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-divixi": "1" },
    // No name: the server reads one from the user-agent. An iPad is the one
    // it cannot tell (it says Macintosh), so that one is named here.
    body: JSON.stringify({ token, name: pairingName(navigator.userAgent, navigator.maxTouchPoints ?? 0) }),
  });
  if (!res.ok) throw new Error(`pairing failed (${res.status})`);
  const got = (await res.json()) as Tokens;
  keep(got.access, got.refresh);
}

/**
 * Take a pairing token out of the address bar, if the QR put one there.
 *
 * Runs before anything else asks for data. The token is removed from the URL
 * whether it worked or not: it is spent either way, and leaving it there puts
 * it in the phone's history and in the next screenshot.
 */
export async function arrive(): Promise<string> {
  const url = new URL(location.href);
  const token = url.searchParams.get("token");
  if (!token) return "";
  let problem = "";
  try {
    await redeem(token);
  } catch (err) {
    problem = String(err);
  }
  url.searchParams.delete("token");
  history.replaceState(null, "", url.pathname + url.search + url.hash);
  return problem;
}

/** Swap the refresh token for a new pair. One at a time, however many
 * requests noticed the expiry at once. */
let renewing: Promise<boolean> | null = null;

function renew(): Promise<boolean> {
  renewing ??= (async () => {
    const token = read(REFRESH);
    if (!token) return false;
    try {
      const res = await fetch(`${base()}/auth/refresh`, {
        method: "POST",
        headers: { authorization: `Bearer ${token}`, "x-divixi": "1" },
      });
      if (!res.ok) {
        // The refresh chain is spent or the device was dropped. Nothing here
        // can recover it; the phone pairs again.
        forget();
        return false;
      }
      const got = (await res.json()) as Tokens;
      keep(got.access, got.refresh);
      return true;
    } catch {
      // A network fault, not a refusal: keep the tokens and let the caller
      // fail this one request.
      return false;
    } finally {
      renewing = null;
    }
  })();
  return renewing;
}

async function send(cmd: string, args: Record<string, unknown>): Promise<Response> {
  return fetch(`${base()}/api/invoke/${encodeURIComponent(cmd)}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-divixi": "1",
      authorization: `Bearer ${read(ACCESS) ?? ""}`,
    },
    body: JSON.stringify(args),
  });
}

/**
 * Call a command on the Divixi serving this page.
 *
 * A 401 is retried once behind a renewal. Access tokens last an hour and the
 * app is open for longer than that, so this is the ordinary path and not an
 * error path.
 */
export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  let res = await send(cmd, args);
  if (res.status === 401 && (await renew())) res = await send(cmd, args);
  const body = (await res.json().catch(() => null)) as { ok?: unknown; error?: unknown } | null;
  if (!res.ok) {
    // The server rejects with the command's own error, so a refusal reads the
    // same here as it does in the app's window.
    throw body?.error ?? new Error(`${cmd} failed (${res.status})`);
  }
  return body?.ok as T;
}

/** A track's file, as the app asks for it in the window. */
export const previewBase = (): string => `${base()}/preview/`;
export const boardBase = (): string => `${base()}/board/`;

// ----- events -----

/** Marks our own socket, and is the protocol the server echoes back. */
const WS_PROTOCOL = "divixi.v1";

type Frame = { seq?: number; event?: string; payload?: unknown; reset?: boolean };
type Handler = (e: { event: string; id: number; payload: unknown }) => void;

const handlers = new Map<string, Set<Handler>>();
let socket: WebSocket | null = null;
let seen = 0;
let backoff = 1000;

/**
 * The event socket, kept up.
 *
 * `?since=` is stage 1's and is what makes a phone work at all: a locked
 * screen, a tunnel walked out of range or a backgrounded tab all come back
 * and are sent what they missed, by number. When the server's ring no longer
 * reaches that far it says `reset` and the page reloads instead of showing
 * a conversation with a hole in it.
 */
function open(): void {
  const token = read(ACCESS);
  if (!token || socket) return;
  const url = new URL(base());
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/api/events";
  url.searchParams.set("since", String(seen));
  // A browser cannot set a header on a WebSocket, so the token rides as a
  // subprotocol. Base64url and `.` are valid token characters, so it needs
  // no re-encoding.
  const ws = new WebSocket(url, [WS_PROTOCOL, token]);
  socket = ws;

  ws.onopen = () => (backoff = 1000);
  ws.onmessage = (e) => {
    let frame: Frame;
    try {
      frame = JSON.parse(String(e.data)) as Frame;
    } catch {
      return;
    }
    if (frame.reset) {
      location.reload();
      return;
    }
    if (typeof frame.seq === "number") seen = frame.seq;
    const set = frame.event ? handlers.get(frame.event) : undefined;
    if (!set) return;
    for (const h of set) h({ event: frame.event!, id: frame.seq ?? 0, payload: frame.payload });
  };
  ws.onclose = () => {
    socket = null;
    // Backing off to half a minute: a phone is often away for minutes at a
    // time and reconnecting every second while it is drains the battery for
    // nothing.
    setTimeout(open, backoff);
    backoff = Math.min(backoff * 2, 30000);
  };
  ws.onerror = () => ws.close();
}

export async function listen<T>(event: string, handler: (e: { event: string; id: number; payload: T }) => void): Promise<() => void> {
  let set = handlers.get(event);
  if (!set) handlers.set(event, (set = new Set()));
  set.add(handler as Handler);
  open();
  return () => {
    set!.delete(handler as Handler);
  };
}
