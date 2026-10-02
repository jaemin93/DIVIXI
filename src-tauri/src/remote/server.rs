//! The HTTP side of a remote instance: what another PC's Divixi app talks
//! to (remote/client.rs), over an SSH tunnel or at this machine's address.
//!
//! Two things come here. Another PC's Divixi app, over an SSH tunnel or at
//! this machine's address; and, since phone access, a phone's browser over
//! `tailscale serve`, which is served the app itself from this same server.
//!
//! Both sign in for tokens (a pairing token minted on this machine, the
//! owner's GitHub account, or a pairing link scanned from a code) and send
//! them as `Authorization: Bearer` with every request. Nothing rides on
//! cookies, so a page elsewhere has nothing to borrow even once a browser is
//! allowed to speak here at all -- which is what makes narrowing the origin
//! rule ([`guard`]) safe rather than merely narrow.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
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
        // `release` joined `build` here so a connecting app can tell an
        // instance it could update (an older release) from one it could not
        // (a build from a working copy). An instance older than this field
        // sends neither, which is itself the answer: it is too old to say.
        .route("/api/health", get(|| async { axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "build": env!("DIVIXI_BUILD"), "release": env!("DIVIXI_RELEASE") })) }))
        // An attachment (up to 50 MB) comes as base64 in a command.
        .route("/api/invoke/{cmd}", post(invoke).layer(axum::extract::DefaultBodyLimit::max(72 * 1024 * 1024)))
        .route("/api/events", get(events))
        .route("/preview/{*rest}", get(preview))
        .route("/board/{*rest}", get(board))
        .route("/raw/{*rest}", get(raw))
        // Anything else is the app: `/`, its assets, and whatever it routes
        // to. Last, so no API path can be shadowed by a file.
        .fallback(app_asset)
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx)
}

/// The origin a browser may name, or none.
///
/// Written when phone access publishes, from the MagicDNS name the daemon
/// gave; cleared when it stops. It is **configuration, not the request**:
/// comparing `Origin` to the request's own `Host` would let an attacker pick
/// the host and pass their own check.
fn allowed_origin(ctx: &Ctx) -> Option<String> {
    super::setting(&ctx.app, "phone.origin").filter(|o| !o.is_empty())
}

/// Every request: from a page, only from our own; and writes carry
/// `X-Divixi`, which a page elsewhere cannot add unasked.
///
/// Until phone access there were no browsers here at all and any `Origin`
/// was refused. A phone is a browser, so the rule narrows instead: the one
/// origin this machine publishes, and nothing else. What keeps that safe is
/// that nothing rides on cookies — tokens are held in `localStorage` and sent
/// as `Authorization`, so another page has no ambient credential to borrow
/// even if it could reach the socket.
async fn guard(State(ctx): State<Ctx>, request: Request<Body>, next: axum::middleware::Next) -> Response<Body> {
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let ours = allowed_origin(&ctx);
        if !ours.as_deref().is_some_and(|o| origin.as_bytes() == o.as_bytes()) {
            return (StatusCode::FORBIDDEN, "not for browsers").into_response();
        }
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

/// The subprotocol that marks our own event socket.
const WS_PROTOCOL: &str = "divixi.v1";

/// A browser cannot set `Authorization` on a WebSocket -- the API has no
/// header argument -- so the token rides as a second subprotocol:
/// `Sec-WebSocket-Protocol: divixi.v1, <token>`. A query parameter would also
/// work and is worse: it lands in logs and in `Referer`. Base64url and `.`
/// are all valid HTTP token characters, so nothing has to be re-encoded.
fn protocol_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("sec-websocket-protocol")?.to_str().ok()?;
    let mut parts = raw.split(',').map(str::trim);
    (parts.next()? == WS_PROTOCOL).then(|| parts.next().map(str::to_string)).flatten().filter(|t| !t.is_empty())
}

/// The requesting device, or why not.
///
/// Two checks, and they answer different questions. The token says this
/// device was let in once and has not been dropped; [`over_the_tailnet`] says
/// the request in front of us really came from somebody allowed to make it.
async fn device(ctx: &Ctx, from: Option<std::net::IpAddr>, headers: &HeaderMap) -> Result<super::auth::Device, Refused> {
    let token = bearer(headers).or_else(|| protocol_token(headers)).ok_or(Refused::SignIn)?;
    let found = {
        let st = ctx.state();
        let owner = super::github::owner(&st.store);
        st.remote.auth.check(&st.store, &token, owner.as_deref())?
    };
    over_the_tailnet(ctx, from, headers, &found).await?;
    Ok(found)
}

/// Whether a request that came through `tailscale serve` may be made.
///
/// Only forwarded requests are judged here: the app's own paths -- an SSH
/// tunnel, this machine's address -- carry no `X-Forwarded-For` and are
/// unchanged by any of this.
///
/// Two things that look alike and are not:
///
/// * **No peer resolved** falls through to the token. A stopped `tailscaled`
///   or a daemon that cannot answer must not lock the owner out of their own
///   machine, and the token is the floor it degrades to -- which is exactly
///   where this stood before identity existed.
/// * **A peer that resolved and is not allowed** is refused. That is not
///   ambiguity, it is an answer, and on a tailnet of 406 colleagues it is the
///   answer the allowlist exists to give.
async fn over_the_tailnet(ctx: &Ctx, from: Option<std::net::IpAddr>, headers: &HeaderMap, device: &super::auth::Device) -> Result<(), Refused> {
    let trust = super::peer::trust(&ctx.app);
    if !trust.on || !super::peer::forwarded(headers) {
        return Ok(());
    }
    let Some(found) = super::peer::resolve(&ctx.app, from, headers, &trust).await else {
        tracing::debug!("a forwarded request resolved to no tailnet peer; its token is the whole of the check");
        return Ok(());
    };
    if !trust.allows(&found.login) {
        tracing::warn!(login = %found.login, "a tailnet account that is not on the allowlist tried to come in");
        return Err(Refused::Dropped);
    }
    // A session pinned to a device cannot be replayed from another, even by
    // the same person inside the same tailnet.
    if let Some(pinned) = device.peer.as_deref() {
        if pinned != found.key(trust.pin) {
            tracing::warn!(device = %device.id, "a device's session came from another node; refusing it");
            return Err(Refused::Dropped);
        }
    }
    Ok(())
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

    /// What the device list will call this. A browser sends no name, so one
    /// is read from its user-agent: dropping a lost phone out of three rows
    /// all reading "Divixi app" is not possible.
    fn name(&self, headers: &HeaderMap) -> String {
        if let Some(given) = self.name.clone().filter(|n| !n.trim().is_empty()) {
            return given;
        }
        match headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()) {
            Some(ua) if !ua.is_empty() => device_name(ua),
            _ => "Divixi app".into(),
        }
    }
}

/// A pairing token minted on this machine (`divixi-server token`, over SSH).
///
/// The scope is not chosen here: it is signed into the token
/// ([`super::auth::Scope`]), so this endpoint cannot be used to widen a link
/// that was minted for a phone.
async fn token(State(ctx): State<Ctx>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, body: Bytes) -> Response<Body> {
    let Some(b) = SignIn::parse(&body) else { return (StatusCode::BAD_REQUEST, "expected {\"token\"}").into_response() };
    // Pinned to whoever is scanning, when the tailnet can say who that is.
    // Decided here, at the one moment a device is created: the pin is what
    // the device carries forever after.
    let pinned = signing_in_from(&ctx, Some(from.ip()), &headers).await;
    let st = ctx.state();
    match st.remote.auth.redeem(&st.store, &b.token, &b.name(&headers), pinned) {
        Ok(pair) => tokens(pair),
        Err(r) => refused(r),
    }
}

/// The peer key to pin a device being created to, or `None`.
///
/// `None` is the ordinary case and not a failure: the Divixi app does not
/// come over the tailnet, and identity trust is off until it is configured.
async fn signing_in_from(ctx: &Ctx, from: Option<std::net::IpAddr>, headers: &HeaderMap) -> Option<String> {
    let trust = super::peer::trust(&ctx.app);
    let found = super::peer::resolve(&ctx.app, from, headers, &trust).await?;
    trust.allows(&found.login).then(|| found.key(trust.pin))
}

/// A short, recognisable name for a phone, from what its browser says.
///
/// Used when a browser signs in and sends no name of its own.
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

/// The peer's address, for the log.
fn peer(headers_ext: &axum::http::Extensions) -> String {
    headers_ext.get::<axum::extract::ConnectInfo<SocketAddr>>().map(|c| c.0.to_string()).unwrap_or_default()
}

/// A GitHub token: in if GitHub says it is this instance's owner.
async fn github(State(ctx): State<Ctx>, request: Request<Body>) -> Response<Body> {
    let from = peer(request.extensions());
    let headers = request.headers().clone();
    let Ok(body) = axum::body::to_bytes(request.into_body(), 64 * 1024).await else { return StatusCode::BAD_REQUEST.into_response() };
    let Some(b) = SignIn::parse(&body) else { return (StatusCode::BAD_REQUEST, "expected {\"token\"}").into_response() };
    let Some(owner) = super::github::owner(&ctx.state().store) else {
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": "this Divixi has no owner yet: sign in to GitHub on it (Settings › Remote instances)" }))).into_response();
    };
    let pinned = signing_in_from(&ctx, from.parse().ok().map(|a: SocketAddr| a.ip()), &headers).await;
    let login = match super::github::login_of(&b.token).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": e }))).into_response(),
    };
    if !login.eq_ignore_ascii_case(&owner) {
        tracing::warn!(%login, %from, "a GitHub account that does not own this Divixi tried to sign in");
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": format!("{login} does not own this Divixi") }))).into_response();
    }
    let st = ctx.state();
    tokens(st.remote.auth.admit(&st.store, &b.name(&headers), Some(login), super::auth::Scope::Full, super::auth::REFRESH_SECS, pinned))
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

async fn invoke(State(ctx): State<Ctx>, Path(cmd): Path<String>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, body: Bytes) -> Response<Body> {
    let who = match device(&ctx, Some(from.ip()), &headers).await {
        Ok(d) => d.id,
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
    if let Err(why) = super::bridge::allowed(&cmd, &args) {
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

async fn events(State(ctx): State<Ctx>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, Query(q): Query<Since>, ws: WebSocketUpgrade) -> Response<Body> {
    // Once, at the upgrade, and never again: a socket lives for hours and
    // resolving per frame would be a daemon call per event.
    let who = match device(&ctx, Some(from.ip()), &headers).await {
        Ok(d) => d.id,
        Err(r) => return refused(r),
    };
    let hub = ctx.state().remote.events.clone();
    // Echoed back only if the browser offered it; the app sends no
    // subprotocol and gets none.
    let ws = if protocol_token(&headers).is_some() { ws.protocols([WS_PROTOCOL]) } else { ws };
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

// ----- the app itself, for a phone -----

/// The built UI, from the assets Tauri already embeds.
///
/// No second copy of `dist`: `asset_resolver()` reads the same bundle the
/// app's own window loads, so the phone and the window can never be showing
/// different builds.
///
/// Served without a token, and that is not an oversight. This is the shell —
/// the same JavaScript the repository ships — and it has to load before
/// there is anywhere to type a token. Everything the shell then asks for goes
/// through `/api`, which does need one.
async fn app_asset(State(ctx): State<Ctx>, uri: Uri) -> Response<Body> {
    let path = uri.path().trim_start_matches('/');
    let wanted = if path.is_empty() { "index.html" } else { path };
    let resolver = ctx.app.asset_resolver();
    // A path with no asset is a route inside the app (`/`, and whatever the
    // app routes to later), so the shell answers for it. A missing file under
    // `/assets/` is a genuine 404 and must not come back as HTML, or a broken
    // script tag reports itself as a parse error somewhere else entirely.
    let found = resolver.get(wanted.to_string()).or_else(|| {
        (!wanted.starts_with("assets/")).then(|| resolver.get("index.html".into())).flatten()
    });
    let Some(asset) = found else {
        return (StatusCode::NOT_FOUND, "no such thing").into_response();
    };
    let mut res = Response::builder().header(header::CONTENT_TYPE, asset.mime_type.clone());
    if let Some(csp) = web_csp(&ctx) {
        res = res.header(header::CONTENT_SECURITY_POLICY, csp);
    }
    // The shell is rebuilt with every release and its asset names are
    // hashed; the entry document is not, so it may not be kept.
    let cache = if wanted.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    res.header(header::CACHE_CONTROL, cache)
        .body(Body::from(asset.bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The policy for the page as a browser sees it.
///
/// Deliberately not the app window's (`tauri.conf.json > app > security >
/// csp`), which `asset.csp_header` would hand over: that one is written for
/// custom schemes a browser has never heard of — `ipc:` in `connect-src`,
/// `preview:` and `board:` in `frame-src` — and passing it on would be
/// shipping a policy nobody wrote for this page. This one names what is
/// actually reachable from here.
///
/// The socket is named explicitly rather than left to `'self'`. Whether
/// `'self'` covers a `wss:` upgrade of the same host has a history of
/// disagreement between browsers, and a policy that only works on some of
/// them is worse than one that says what it means.
fn web_csp(ctx: &Ctx) -> Option<String> {
    allowed_origin(ctx).map(|o| web_csp_for(&o))
}

fn web_csp_for(origin: &str) -> String {
    let socket = origin.replacen("https://", "wss://", 1);
    format!(
        "default-src 'self';          script-src 'self';          style-src 'self' 'unsafe-inline' https://fonts.googleapis.com;          font-src https://fonts.gstatic.com;          img-src 'self' data: blob:;          connect-src 'self' {socket};          frame-src 'self';          object-src 'none';          base-uri 'none';          form-action 'none'"
    )
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

async fn preview(State(ctx): State<Ctx>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, Some(from.ip()), &headers).await {
        return refused(r);
    }
    let app = ctx.app.clone();
    let req = rebuilt(&uri, "/preview");
    match tauri::async_runtime::spawn_blocking(move || crate::preview::handle(&app, req)).await {
        Ok(res) => from_protocol(res, true),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn board(State(ctx): State<Ctx>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, Some(from.ip()), &headers).await {
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
async fn raw(State(ctx): State<Ctx>, ConnectInfo(from): ConnectInfo<SocketAddr>, headers: HeaderMap, uri: Uri) -> Response<Body> {
    if let Err(r) = device(&ctx, Some(from.ip()), &headers).await {
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
    fn the_browsers_policy_names_the_socket_and_not_the_window_schemes() {
        // Built from a known origin rather than read off `asset.csp_header`,
        // which carries `ipc:` and `preview:` -- schemes that mean nothing in
        // a browser and would be a policy nobody wrote for this page.
        let csp = super::web_csp_for("https://laptop.tailb45a71.ts.net");
        assert!(csp.contains("connect-src 'self' wss://laptop.tailb45a71.ts.net"), "{csp}");
        for absent in ["ipc:", "preview:", "board:", "asset:"] {
            assert!(!csp.contains(absent), "{absent} is the window's, not a browser's: {csp}");
        }
        assert!(csp.contains("object-src 'none'") && csp.contains("base-uri 'none'"), "{csp}");
    }

    #[test]
    fn a_websocket_carries_its_token_as_a_subprotocol() {
        // A browser cannot set `Authorization` on a WebSocket, so the token
        // rides here instead. Anything that is not our protocol first is not
        // our socket.
        let mut h = HeaderMap::new();
        assert_eq!(protocol_token(&h), None);
        h.insert("sec-websocket-protocol", "divixi.v1, head.sig".parse().unwrap());
        assert_eq!(protocol_token(&h).as_deref(), Some("head.sig"));
        h.insert("sec-websocket-protocol", "divixi.v1,head.sig".parse().unwrap());
        assert_eq!(protocol_token(&h).as_deref(), Some("head.sig"), "spaces are optional");
        h.insert("sec-websocket-protocol", "divixi.v1".parse().unwrap());
        assert_eq!(protocol_token(&h), None, "no token offered");
        h.insert("sec-websocket-protocol", "divixi.v1, ".parse().unwrap());
        assert_eq!(protocol_token(&h), None, "an empty token is no token");
        h.insert("sec-websocket-protocol", "chat, head.sig".parse().unwrap());
        assert_eq!(protocol_token(&h), None, "another protocol's socket is not ours");
        h.insert("sec-websocket-protocol", "head.sig, divixi.v1".parse().unwrap());
        assert_eq!(protocol_token(&h), None, "the order is part of the shape");
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
