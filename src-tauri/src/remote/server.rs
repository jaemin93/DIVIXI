//! The HTTP side of a remote instance: what another PC's Divixi app talks
//! to (remote/client.rs), over an SSH tunnel or at this machine's address.
//!
//! Only the app comes here, never a browser: a device signs in for tokens
//! (with a pairing token minted on this machine, or as the owner's GitHub
//! account) and sends them as `Authorization: Bearer` with every request.
//! Nothing rides on cookies, so a page elsewhere has nothing to borrow;
//! requests that name a browser origin are refused all the same.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use tauri::Manager;

use crate::AppHandle;

use super::auth::Refused;
use super::events::CatchUp;
use crate::AppState;

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
        .route("/auth/token", post(token))
        .route("/auth/github", post(github))
        .route("/auth/refresh", post(refresh))
        .route("/auth/pair", get(pair))
        .route("/api/health", get(|| async { axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "build": env!("DIVIXI_BUILD") })) }))
        // An attachment (up to 50 MB) comes as base64 in a command.
        .route("/api/invoke/{cmd}", post(invoke).layer(axum::extract::DefaultBodyLimit::max(72 * 1024 * 1024)))
        .route("/api/events", get(events))
        .route("/preview/{*rest}", get(preview))
        .route("/board/{*rest}", get(board))
        .route("/raw/{*rest}", get(raw))
        .fallback(|| async { (StatusCode::NOT_FOUND, "no such thing") })
        .layer(axum::middleware::from_fn(guard))
        .with_state(ctx)
}

/// Every request: not from a browser page (the app names no origin), and
/// writes carry `X-Divixi`, which a page elsewhere cannot add unasked.
async fn guard(request: Request<Body>, next: axum::middleware::Next) -> Response<Body> {
    if request.headers().contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "not for browsers").into_response();
    }
    let writes = request.method() != Method::GET && request.method() != Method::HEAD;
    if writes && request.headers().get("x-divixi").is_none() {
        return (StatusCode::FORBIDDEN, "missing X-Divixi").into_response();
    }
    next.run(request).await
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(|v| v.trim().to_string())
}

/// The requesting device, or why not.
fn device(ctx: &Ctx, headers: &HeaderMap) -> Result<super::auth::Device, Refused> {
    let token = bearer(headers).ok_or(Refused::SignIn)?;
    let st = ctx.state();
    let owner = super::github::owner(&st.store);
    st.remote.auth.check(&st.store, &token, owner.as_deref())
}

fn refused(r: Refused) -> Response<Body> {
    let why = match r {
        Refused::SignIn => "sign-in",
        Refused::Dropped => "dropped",
    };
    (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": why }))).into_response()
}

fn tokens((access, refresh): (String, String)) -> Response<Body> {
    axum::Json(json!({ "access": access, "refresh": refresh })).into_response()
}

// ----- sign-in -----

#[derive(serde::Deserialize)]
struct SignIn {
    token: String,
    #[serde(default)]
    name: Option<String>,
}

impl SignIn {
    fn parse(body: &[u8]) -> Option<Self> {
        serde_json::from_slice(body).ok()
    }

    fn name(&self) -> String {
        self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Divixi app".into())
    }
}

/// A pairing token minted on this machine (`divixi-server token`, over SSH).
///
/// The scope is not chosen here: it is signed into the token
/// ([`super::auth::Scope`]), so this endpoint cannot be used to widen a link
/// that was minted for a phone.
async fn token(State(ctx): State<Ctx>, body: Bytes) -> Response<Body> {
    let Some(b) = SignIn::parse(&body) else { return (StatusCode::BAD_REQUEST, "expected {\"token\"}").into_response() };
    let st = ctx.state();
    match st.remote.auth.redeem(&st.store, &b.token, &b.name()) {
        Ok(pair) => tokens(pair),
        Err(r) => refused(r),
    }
}

/// A short, recognisable name for a phone, from what its browser says.
///
/// The device list is how a lost phone gets dropped, and dropping the right
/// one out of three rows all reading "Divixi app" is not possible. Browsers
/// lie about most of a user-agent string; the few tokens read here are the
/// ones that survive that, and anything unrecognised is simply "Phone" rather
/// than a guess dressed up as a fact.
fn device_name(ua: &str) -> String {
    let os = [("iPhone", "iPhone"), ("iPad", "iPad"), ("Android", "Android"), ("Macintosh", "Mac"), ("Windows", "Windows"), ("Linux", "Linux")]
        .iter()
        .find(|(needle, _)| ua.contains(needle))
        .map(|(_, name)| *name);
    // Order matters: all of these say "Safari" too, and Edge says "Chrome",
    // so the most specific claim has to be tested first.
    let browser = [
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("SamsungBrowser", "Samsung Internet"),
        ("FxiOS", "Firefox"),
        ("CriOS", "Chrome"),
        ("Firefox", "Firefox"),
        ("Chrome", "Chrome"),
        ("Safari", "Safari"),
    ]
    .iter()
    .find(|(needle, _)| ua.contains(needle))
    .map(|(_, name)| *name);
    match (os, browser) {
        (Some(os), Some(b)) => format!("{os} · {b}"),
        (Some(os), None) => os.to_string(),
        (None, Some(b)) => b.to_string(),
        (None, None) => "Phone".to_string(),
    }
}

#[derive(serde::Deserialize)]
struct PairQuery {
    #[serde(default)]
    token: String,
}

/// Open a pairing link from a phone's browser.
///
/// A navigation, so it carries no `Origin` and passes the guard above without
/// the guard being widened for it. The link is spent here, and the page that
/// comes back keeps the tokens for this origin and takes the token out of the
/// address bar, so it is not left sitting in the phone's history.
async fn pair(State(ctx): State<Ctx>, headers: HeaderMap, Query(q): Query<PairQuery>) -> Response<Body> {
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let name = device_name(ua);
    let machine = sysinfo::System::host_name().unwrap_or_else(|| "this computer".into());
    let st = ctx.state();
    match st.remote.auth.redeem(&st.store, &q.token, &name) {
        Ok((access, refresh)) => {
            tracing::info!(device = %name, "a device paired from a browser");
            html(StatusCode::OK, &paired_page(&machine, &name, &access, &refresh))
        }
        // No reason beyond "it did not work". The two ways here are an expired
        // code and one already scanned, and telling them apart tells whoever
        // is holding a code they should not have which of the two it is.
        Err(_) => html(StatusCode::UNAUTHORIZED, &refused_page(&machine)),
    }
}

fn html(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        // This page carries tokens: nothing may keep a copy of it.
        .header(header::CACHE_CONTROL, "no-store")
        .header("Referrer-Policy", "no-referrer")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Text going into the page. A host name and a browser's own string are both
/// outside our control, and a token is base64url, so everything interpolated
/// below goes through here.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '/' => out.push_str("&#47;"),
            c => out.push(c),
        }
    }
    out
}

const PAGE_STYLE: &str = concat!(
    r#"<meta name="viewport" content="width=device-width,initial-scale=1">"#,
    "<style>body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;",
    "background:#111318;color:#e7e9ee;font:16px/1.6 system-ui,-apple-system,sans-serif}",
    "main{max-width:30rem;padding:2rem}h1{font-size:1.25rem;margin:0 0 .75rem;line-height:1.4}",
    "p{margin:0 0 .75rem;color:#a8adbb}b{color:#e7e9ee;font-weight:600}</style>",
);

/// What a phone sees when its scan worked.
///
/// The tokens are kept for this origin, where the app will look for them once
/// there is an app to serve here. Kept in `localStorage` and not a cookie, on
/// purpose: a cookie would be sent with every request to this origin whoever
/// caused it, and the whole reason this server has no CSRF problem is that
/// there is no ambient credential for another page to ride.
fn paired_page(machine: &str, name: &str, access: &str, refresh: &str) -> String {
    format!(
        concat!(
            "<!doctype html><html lang=\"en\"><head><title>Divixi</title>{style}</head><body><main>",
            "<h1>This phone is paired with <b>{machine}</b></h1>",
            "<p>Signed in as <b>{name}</b>. Divixi itself is not served here yet \u{2014} this address will open it when the next update lands.</p>",
            "<p>To undo this, open Divixi on {machine} and drop this device under Settings \u{203a} Remote instances.</p>",
            "</main><script>",
            "try{{localStorage.setItem('divixi.access','{access}');localStorage.setItem('divixi.refresh','{refresh}')}}catch(e){{}}",
            "history.replaceState(null,'','/');",
            "</script></body></html>",
        ),
        style = PAGE_STYLE,
        machine = escape(machine),
        name = escape(name),
        access = escape(access),
        refresh = escape(refresh),
    )
}

fn refused_page(machine: &str) -> String {
    format!(
        concat!(
            "<!doctype html><html lang=\"en\"><head><title>Divixi</title>{style}</head><body><main>",
            "<h1>This code did not work</h1>",
            "<p>A code lasts five minutes and can be scanned once. Open Divixi on <b>{machine}</b> and show a new one.</p>",
            "</main></body></html>",
        ),
        style = PAGE_STYLE,
        machine = escape(machine),
    )
}

/// The peer's address, for the log.
fn peer(headers_ext: &axum::http::Extensions) -> String {
    headers_ext.get::<axum::extract::ConnectInfo<SocketAddr>>().map(|c| c.0.to_string()).unwrap_or_default()
}

/// A GitHub token: in if GitHub says it is this instance's owner.
async fn github(State(ctx): State<Ctx>, request: Request<Body>) -> Response<Body> {
    let from = peer(request.extensions());
    let Ok(body) = axum::body::to_bytes(request.into_body(), 64 * 1024).await else { return StatusCode::BAD_REQUEST.into_response() };
    let Some(b) = SignIn::parse(&body) else { return (StatusCode::BAD_REQUEST, "expected {\"token\"}").into_response() };
    let Some(owner) = super::github::owner(&ctx.state().store) else {
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": "this Divixi has no owner yet: sign in to GitHub on it (Settings › Remote instances)" }))).into_response();
    };
    let login = match super::github::login_of(&b.token).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": e }))).into_response(),
    };
    if !login.eq_ignore_ascii_case(&owner) {
        tracing::warn!(%login, %from, "a GitHub account that does not own this Divixi tried to sign in");
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": format!("{login} does not own this Divixi") }))).into_response();
    }
    let st = ctx.state();
    tokens(st.remote.auth.admit(&st.store, &b.name(), Some(login), super::auth::Scope::Full, super::auth::REFRESH_SECS))
}

/// New tokens for a refresh token (as `Bearer`).
async fn refresh(State(ctx): State<Ctx>, headers: HeaderMap) -> Response<Body> {
    let Some(token) = bearer(&headers) else { return refused(Refused::SignIn) };
    let st = ctx.state();
    match st.remote.auth.refresh(&st.store, &token) {
        Ok(pair) => tokens(pair),
        Err(r) => refused(r),
    }
}

// ----- commands and events -----

async fn invoke(State(ctx): State<Ctx>, Path(cmd): Path<String>, headers: HeaderMap, body: Bytes) -> Response<Body> {
    let (who, scope) = match device(&ctx, &headers) {
        Ok(d) => (d.id, d.scope),
        Err(r) => return refused(r),
    };
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": format!("the arguments are not JSON: {e}") }))).into_response(),
        }
    };
    if let Err(why) = super::bridge::allowed(&cmd, &args, scope) {
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": why }))).into_response();
    }
    let closing = (cmd == "term_close").then(|| args.get("id").and_then(Value::as_u64)).flatten();
    match super::bridge::invoke(&ctx.app, &cmd, args).await {
        Ok(v) => {
            // A shell is the device's that opened it (see `orphans`).
            let terms = &ctx.state().remote.terms;
            if let (true, Some(id)) = (cmd == "term_open", v.as_u64()) {
                terms.lock().insert(id, who);
            }
            if let Some(id) = closing {
                terms.lock().remove(&id);
            }
            axum::Json(json!({ "ok": v })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

async fn events(State(ctx): State<Ctx>, headers: HeaderMap, Query(q): Query<Since>, ws: WebSocketUpgrade) -> Response<Body> {
    let who = match device(&ctx, &headers) {
        Ok(d) => d.id,
        Err(r) => return refused(r),
    };
    let hub = ctx.state().remote.events.clone();
    ws.on_upgrade(move |socket| async move {
        *ctx.state().remote.sockets.lock().entry(who.clone()).or_default() += 1;
        pump(socket, hub, q.since).await;
        let left = {
            let st = ctx.state();
            let mut sockets = st.remote.sockets.lock();
            let n = sockets.entry(who.clone()).or_default();
            *n = n.saturating_sub(1);
            *n
        };
        if left == 0 {
            tauri::async_runtime::spawn(orphans(ctx.app.clone(), who));
        }
    })
}

/// How long a device may be away (no event socket) before its shells here
/// are closed. The app pings every 10 s while it runs; this is a crash, a
/// lost network, a laptop asleep.
const AWAY: std::time::Duration = std::time::Duration::from_secs(120);

/// Close the shells of a device that did not come back.
async fn orphans(app: AppHandle, who: String) {
    tokio::time::sleep(AWAY).await;
    let st = app.state::<AppState>();
    if st.remote.sockets.lock().get(&who).copied().unwrap_or(0) > 0 {
        return;
    }
    let gone: Vec<u64> = {
        let mut terms = st.remote.terms.lock();
        let ids: Vec<u64> = terms.iter().filter(|(_, d)| **d == who).map(|(t, _)| *t).collect();
        for t in &ids {
            terms.remove(t);
        }
        ids
    };
    for t in &gone {
        if let Ok(id) = u32::try_from(*t) {
            st.terminals.close(id);
        }
    }
    if !gone.is_empty() {
        tracing::info!(device = %who, shells = gone.len(), "closed the shells of a device that went away");
    }
}

async fn pump(mut socket: WebSocket, hub: Arc<super::events::Hub>, since: u64) {
    // A word every 30 s, so the app can tell a quiet link from a dead one.
    let mut ping = tokio::time::interval(std::time::Duration::from_secs(30));
    ping.tick().await;
    let mut live = hub.subscribe_live();
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
            got = live.recv() => match got {
                Ok(text) => {
                    if socket.send(Message::Text(text.to_string().into())).await.is_err() {
                        return;
                    }
                }
                // Behind on a terminal's output: that much is skipped.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
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
    // A previewed page is the track's, not the app's: it runs in a sandbox
    // of its own (the app passes this policy on).
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

async fn preview(State(ctx): State<Ctx>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers) {
        return refused(r);
    }
    let app = ctx.app.clone();
    let req = rebuilt(&uri, "/preview");
    match tauri::async_runtime::spawn_blocking(move || crate::preview::handle(&app, req)).await {
        Ok(res) => from_protocol(res, true),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn board(State(ctx): State<Ctx>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers) {
        return refused(r);
    }
    let app = ctx.app.clone();
    let req = rebuilt(&uri, "/board");
    match tauri::async_runtime::spawn_blocking(move || crate::preview::handle_board(&app, req)).await {
        Ok(res) => from_protocol(res, false),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// A track's file as it is (`/raw/<track>/<path>`), for saving it on the
/// app's PC.
async fn raw(State(ctx): State<Ctx>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers) {
        return refused(r);
    }
    let path = uri.path().strip_prefix("/raw/").unwrap_or_default();
    let (track, rel) = path.split_once('/').unwrap_or((path, ""));
    let (track, rel) = (crate::preview::decode(track), crate::preview::decode(rel));
    let full = {
        let st = ctx.state();
        match crate::track_root(&st, &track).and_then(|root| crate::workspace::resolve(&root, &rel)) {
            Ok(f) if f.is_file() => f,
            _ => return (StatusCode::NOT_FOUND, "no such file").into_response(),
        }
    };
    match tokio::fs::read(&full).await {
        Ok(bytes) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_gets_a_name_somebody_could_pick_out_of_a_list() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1";
        assert_eq!(device_name(iphone), "iPhone · Safari");
        let android = "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";
        assert_eq!(device_name(android), "Android · Chrome", "Android is tested before the Linux it also claims");
        let edge = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36 Edg/120.0";
        assert_eq!(device_name(edge), "Windows · Edge", "Edge says Chrome and Safari too");
        let samsung = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 SamsungBrowser/23.0 Chrome/115.0 Mobile Safari/537.36";
        assert_eq!(device_name(samsung), "Android · Samsung Internet");
        // Nothing recognised is not guessed at.
        assert_eq!(device_name(""), "Phone");
        assert_eq!(device_name("curl/8.0"), "Phone");
    }

    #[test]
    fn nothing_borrowed_from_outside_can_break_out_of_the_page() {
        // A host name and a user-agent are not ours, and the page carries a
        // token inside a script.
        assert_eq!(escape("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;&#47;script&gt;");
        assert_eq!(escape("a'b\"c"), "a&#39;b&quot;c");
        assert_eq!(escape("a&b"), "a&amp;b");
        let page = paired_page("</script><b>pwn", "iPhone · Safari", "aa.bb", "cc.dd");
        assert!(!page.contains("</script><b>pwn"), "the host name is escaped");
        assert!(page.contains("aa.bb") && page.contains("cc.dd"), "the tokens still reach the phone");
        // The page must not be kept anywhere, and must not say which way the
        // scan failed.
        let refused = refused_page("desk");
        assert!(!refused.contains("expired") && !refused.contains("already"));
    }

    #[test]
    fn bearer_tokens_are_read() {
        let mut h = HeaderMap::new();
        assert_eq!(bearer(&h), None);
        h.insert(header::AUTHORIZATION, "Bearer tok.sig".parse().unwrap());
        assert_eq!(bearer(&h).as_deref(), Some("tok.sig"));
        h.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&h), None);
    }
}
