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
        .route("/api/health", get(|| async { axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })) }))
        // An attachment (up to 50 MB) comes as base64 in a command.
        .route("/api/invoke/{cmd}", post(invoke).layer(axum::extract::DefaultBodyLimit::max(72 * 1024 * 1024)))
        .route("/api/events", get(events))
        .route("/preview/{*rest}", get(preview))
        .route("/board/{*rest}", get(board))
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
async fn token(State(ctx): State<Ctx>, body: Bytes) -> Response<Body> {
    let Some(b) = SignIn::parse(&body) else { return (StatusCode::BAD_REQUEST, "expected {\"token\"}").into_response() };
    let st = ctx.state();
    match st.remote.auth.redeem(&st.store, &b.token, &b.name()) {
        Ok(pair) => tokens(pair),
        Err(r) => refused(r),
    }
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
    tokens(st.remote.auth.admit(&st.store, &b.name(), Some(login)))
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
    if let Err(r) = device(&ctx, &headers) {
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

async fn events(State(ctx): State<Ctx>, headers: HeaderMap, Query(q): Query<Since>, ws: WebSocketUpgrade) -> Response<Body> {
    if let Err(r) = device(&ctx, &headers) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
