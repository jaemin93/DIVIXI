//! The HTTP side of remote access: an axum server on 127.0.0.1 only.
//!
//! What reaches it from outside comes through `tailscale serve` (which
//! proxies from loopback); the checks here hold whatever is in front:
//! the Host must be this server's (DNS rebinding), writes and the event
//! socket must come from this origin (CSRF) with the `X-Divixi` header,
//! and everything under /api needs a device's access cookie.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{any, get, post};
use axum::Router;
use serde_json::{json, Value};
use tauri::Manager;

use crate::AppHandle;

use super::auth::Refused;
use super::events::CatchUp;
use crate::AppState;

const ACCESS_COOKIE: &str = "divixi_a";
const REFRESH_COOKIE: &str = "divixi_r";

/// A cookie's name for this request: browsers share cookies across ports of
/// one host, so two Divixis a browser reaches on 127.0.0.1 (this one, and a
/// remote one through a tunnel) must not overwrite each other's (as Kiro's
/// `mc_token_<port>`). The port is the one the browser used.
fn named(base: &str, headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let port = host.rsplit_once(':').map(|(_, p)| p).filter(|p| p.chars().all(|c| c.is_ascii_digit())).unwrap_or("0");
    format!("{base}_{port}")
}

#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
}

impl Ctx {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

pub fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/auth/pair", get(pair))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .route("/api/health", get(|| async { axum::Json(json!({ "ok": true })) }))
        .route("/api/me", get(me))
        // An attachment (up to 50 MB) comes as base64 in a command.
        .route("/api/invoke/{cmd}", post(invoke).layer(axum::extract::DefaultBodyLimit::max(72 * 1024 * 1024)))
        .route("/api/events", get(events))
        .route("/preview/{*rest}", get(preview))
        .route("/board/{*rest}", get(board))
        .fallback(any(asset))
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx)
}

// ----- the checks every request passes -----

/// Hosts this server answers to: loopback on any port (an SSH tunnel from
/// another machine arrives on its own local port), and the PC's name on the
/// tailnet once serve publishes it. A rebinding attack needs a name of its
/// own, which is not among these.
fn host_ok(ctx: &Ctx, host: &str) -> bool {
    let name = host.rsplit_once(':').map(|(h, p)| if p.chars().all(|c| c.is_ascii_digit()) { h } else { host }).unwrap_or(host);
    matches!(name, "127.0.0.1" | "localhost" | "[::1]") || ctx.state().remote.public_host().is_some_and(|h| h == host)
}

/// Whether the request came through tailscale serve (or any proxy):
/// such a request is remote even though it arrives on loopback.
fn proxied(headers: &HeaderMap) -> bool {
    ["x-forwarded-for", "x-forwarded-proto", "forwarded", "x-real-ip", "tailscale-user-login"].iter().any(|h| headers.contains_key(*h))
}

/// The Tailscale login serve vouches for (it strips any a client sends).
fn login(headers: &HeaderMap, peer: &SocketAddr) -> Option<String> {
    if !peer.ip().is_loopback() {
        return None;
    }
    headers.get("tailscale-user-login").and_then(|v| v.to_str().ok()).map(str::to_string)
}

async fn guard(State(ctx): State<Ctx>, request: Request<Body>, next: axum::middleware::Next) -> Response<Body> {
    let host = request.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default().to_string();
    if !host_ok(&ctx, &host) {
        return (StatusCode::MISDIRECTED_REQUEST, "unknown host").into_response();
    }
    let writes = request.method() != Method::GET && request.method() != Method::HEAD;
    let socket = request.uri().path() == "/api/events";
    if writes || socket {
        // From this very origin: http on loopback, https through serve.
        let origin = request.headers().get(header::ORIGIN).and_then(|o| o.to_str().ok()).unwrap_or_default();
        let ok = origin == format!("http://{host}") || origin == format!("https://{host}");
        if !ok {
            return (StatusCode::FORBIDDEN, "cross-origin request").into_response();
        }
    }
    if writes && request.headers().get("x-divixi").is_none() {
        return (StatusCode::FORBIDDEN, "missing X-Divixi").into_response();
    }
    next.run(request).await
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// The requesting device, or why not.
fn device(ctx: &Ctx, headers: &HeaderMap, peer: &SocketAddr) -> Result<super::auth::Device, Refused> {
    let token = cookie(headers, &named(ACCESS_COOKIE, headers)).ok_or(Refused::SignIn)?;
    let st = ctx.state();
    st.remote.auth.check(&st.store, &token, login(headers, peer).as_deref())
}

fn refused(r: Refused) -> Response<Body> {
    let why = match r {
        Refused::SignIn => "sign-in",
        Refused::Dropped => "dropped",
    };
    (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": why }))).into_response()
}

fn secure(headers: &HeaderMap) -> bool {
    proxied(headers)
}

fn set_cookies(headers: &HeaderMap, access: &str, refresh: &str) -> [(header::HeaderName, String); 2] {
    let s = if secure(headers) { "; Secure" } else { "" };
    [
        (header::SET_COOKIE, format!("{}={access}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{s}", named(ACCESS_COOKIE, headers), super::auth::ACCESS_SECS)),
        (header::SET_COOKIE, format!("{}={refresh}; Path=/auth; HttpOnly; SameSite=Strict; Max-Age={}{s}", named(REFRESH_COOKIE, headers), super::auth::REFRESH_SECS)),
    ]
}

// ----- sign-in -----

#[derive(serde::Deserialize)]
struct PairQuery {
    token: String,
}

/// A pairing link opened: the device is paired, gets its cookies, and the
/// token leaves the address bar (Jupyter's way).
async fn pair(State(ctx): State<Ctx>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Query(q): Query<PairQuery>) -> Response<Body> {
    let name = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(device_name).unwrap_or_else(|| "Browser".into());
    let st = ctx.state();
    match st.remote.auth.redeem(&st.store, &q.token, &name, login(&headers, &peer)) {
        Ok((access, refresh)) => {
            let [a, r] = set_cookies(&headers, &access, &refresh);
            let mut res = Redirect::to("/").into_response();
            res.headers_mut().append(a.0, a.1.parse().expect("cookie is ascii"));
            res.headers_mut().append(r.0, r.1.parse().expect("cookie is ascii"));
            res
        }
        Err(_) => (StatusCode::UNAUTHORIZED, axum::response::Html(SIGN_IN_AGAIN)).into_response(),
    }
}

const SIGN_IN_AGAIN: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Divixi</title><body style='font-family:system-ui;padding:24px'><h3>이 링크는 쓸 수 없습니다</h3><p>링크는 5분 안에 한 번만 열 수 있습니다. PC의 Divixi에서 설정 › 원격 접속 › 휴대폰 연결로 새 QR을 만드세요.</p><p>This link can't be used: it works once, within five minutes. Make a new QR on the PC (Settings › Remote access).</p>";

/// A short name for the device list, from the browser's user agent.
fn device_name(ua: &str) -> String {
    let os = ["iPhone", "iPad", "Android", "Windows", "Macintosh", "Linux"].into_iter().find(|o| ua.contains(o)).unwrap_or("Browser");
    let browser = ["Edg", "Chrome", "Firefox", "Safari"].into_iter().find(|b| ua.contains(b)).unwrap_or("");
    let browser = if browser == "Edg" { "Edge" } else { browser };
    if browser.is_empty() {
        os.to_string()
    } else {
        format!("{os} · {browser}")
    }
}

async fn refresh(State(ctx): State<Ctx>, headers: HeaderMap) -> Response<Body> {
    let Some(token) = cookie(&headers, &named(REFRESH_COOKIE, &headers)) else { return refused(Refused::SignIn) };
    let st = ctx.state();
    match st.remote.auth.refresh(&st.store, &token) {
        Ok((access, refresh)) => {
            let mut res = StatusCode::NO_CONTENT.into_response();
            for (k, v) in set_cookies(&headers, &access, &refresh) {
                res.headers_mut().append(k, v.parse().expect("cookie is ascii"));
            }
            res
        }
        Err(r) => refused(r),
    }
}

async fn logout(State(ctx): State<Ctx>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response<Body> {
    if let Ok(d) = device(&ctx, &headers, &peer) {
        let st = ctx.state();
        st.remote.auth.drop_device(&st.store, &d.id);
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    for c in [format!("{}=; Path=/; Max-Age=0", named(ACCESS_COOKIE, &headers)), format!("{}=; Path=/auth; Max-Age=0", named(REFRESH_COOKIE, &headers))] {
        res.headers_mut().append(header::SET_COOKIE, c.parse().expect("ascii"));
    }
    res
}

async fn me(State(ctx): State<Ctx>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response<Body> {
    match device(&ctx, &headers, &peer) {
        Ok(d) => axum::Json(json!({ "device": d.id, "name": d.name, "last": ctx.state().remote.events.last() })).into_response(),
        Err(r) => refused(r),
    }
}

// ----- commands and events -----

async fn invoke(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(cmd): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers, &peer) {
        return refused(r);
    }
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": format!("the arguments are not JSON: {e}") }))).into_response(),
        }
    };
    if let Err(why) = super::bridge::allowed(&cmd, &args) {
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": why }))).into_response();
    }
    match super::bridge::invoke(&ctx.app, &cmd, args).await {
        Ok(v) => axum::Json(json!({ "ok": v })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

async fn events(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<Since>,
    ws: WebSocketUpgrade,
) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers, &peer) {
        return refused(r);
    }
    let hub = ctx.state().remote.events.clone();
    ws.on_upgrade(move |socket| pump(socket, hub, q.since))
}

async fn pump(mut socket: WebSocket, hub: Arc<super::events::Hub>, since: u64) {
    let (mut rx, catch) = hub.subscribe(since);
    let mut last = since;
    match catch {
        CatchUp::Reset => {
            if socket.send(Message::Text("{\"reset\":true}".into())).await.is_err() {
                return;
            }
        }
        CatchUp::Frames(frames) => {
            for f in frames {
                last = f.seq;
                if socket.send(Message::Text(f.text.to_string().into())).await.is_err() {
                    return;
                }
            }
        }
    }
    loop {
        tokio::select! {
            got = rx.recv() => match got {
                Ok(f) if f.seq > last => {
                    last = f.seq;
                    if socket.send(Message::Text(f.text.to_string().into())).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                // Too slow to keep up: it reloads rather than miss something.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = socket.send(Message::Text("{\"reset\":true}".into())).await;
                    return;
                }
                Err(_) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
}

// ----- files -----

fn from_protocol(res: axum::http::Response<Cow<'static, [u8]>>, sandbox: bool) -> Response<Body> {
    let (mut parts, body) = res.into_parts();
    // A previewed page is the track's, not the app's: on this origin it
    // runs in a sandbox of its own, cookies and all out of reach.
    if sandbox {
        parts.headers.insert(header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts".parse().expect("ascii"));
    }
    Response::from_parts(parts, Body::from(body.into_owned()))
}

fn rebuilt(uri: &Uri, prefix: &str) -> Request<Vec<u8>> {
    let rest = uri.path().strip_prefix(prefix).unwrap_or("/");
    let with_query = match uri.query() {
        Some(q) => format!("{rest}?{q}"),
        None => rest.to_string(),
    };
    Request::builder().uri(with_query).body(Vec::new()).expect("a path is a uri")
}

async fn preview(State(ctx): State<Ctx>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers, &peer) {
        return refused(r);
    }
    let app = ctx.app.clone();
    let req = rebuilt(&uri, "/preview");
    match tauri::async_runtime::spawn_blocking(move || crate::preview::handle(&app, req)).await {
        Ok(res) => from_protocol(res, true),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn board(State(ctx): State<Ctx>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers, &peer) {
        return refused(r);
    }
    let app = ctx.app.clone();
    let req = rebuilt(&uri, "/board");
    match tauri::async_runtime::spawn_blocking(move || crate::preview::handle_board(&app, req)).await {
        Ok(res) => from_protocol(res, false),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The app's own UI, as built into it. Paths without a file are the app
/// (it routes itself); the UI holds no secrets, so it needs no cookie.
async fn asset(State(ctx): State<Ctx>, uri: Uri) -> Response<Body> {
    let path = uri.path().trim_start_matches('/');
    let resolver = ctx.app.asset_resolver();
    let found = if path.is_empty() { None } else { resolver.get(path.to_string()) };
    let asset = match found.or_else(|| resolver.get("index.html".to_string())) {
        Some(a) => a,
        None => return (StatusCode::NOT_FOUND, "no such file").into_response(),
    };
    // The page says it came from here: in a Divixi window showing another
    // Divixi, Tauri's IPC is present but is not the way to this app.
    let bytes = if asset.mime_type.starts_with("text/html") { served(&asset.bytes) } else { asset.bytes };
    Response::builder()
        .header(header::CONTENT_TYPE, asset.mime_type)
        .header(header::CONTENT_SECURITY_POLICY, WEB_CSP)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The page with `<meta name="divixi-served">` first in its head.
fn served(html: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    match text.find("<head>") {
        Some(at) => format!("{}<meta name=\"divixi-served\" content=\"1\">{}", &text[..at + 6], &text[at + 6..]).into_bytes(),
        None => html.to_vec(),
    }
}

/// The app's policy, for a browser: its own origin for everything, the
/// fonts it uses, and no framing by others.
const WEB_CSP: &str = "default-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' data: blob:; connect-src 'self' ws: wss:; frame-src 'self'; frame-ancestors 'none'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names() {
        assert_eq!(device_name("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit Version/18.0 Mobile Safari/604.1"), "iPhone · Safari");
        assert_eq!(device_name("Mozilla/5.0 (Linux; Android 15) AppleWebKit Chrome/140.0 Mobile Safari/537.36"), "Android · Chrome");
        assert_eq!(device_name("curl/8"), "Browser");
    }

    #[test]
    fn cookies_are_read_by_name() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:7488".parse().unwrap());
        h.insert(header::COOKIE, "x=1; divixi_a_7489=other; divixi_a_7488=tok.sig; y=2".parse().unwrap());
        assert_eq!(named(ACCESS_COOKIE, &h), "divixi_a_7488");
        assert_eq!(cookie(&h, &named(ACCESS_COOKIE, &h)).as_deref(), Some("tok.sig"), "each port has its own");
        assert_eq!(cookie(&h, &named(REFRESH_COOKIE, &h)), None);
    }

    #[test]
    fn served_pages_say_so() {
        assert_eq!(served(b"<html><head><title>x</title>"), b"<html><head><meta name=\"divixi-served\" content=\"1\"><title>x</title>".to_vec());
    }

    #[test]
    fn a_proxied_request_is_remote() {
        let mut h = HeaderMap::new();
        assert!(!proxied(&h));
        h.insert("x-forwarded-for", "100.64.0.2".parse().unwrap());
        assert!(proxied(&h));
    }
}
