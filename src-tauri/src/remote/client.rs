//! Using a Divixi on another machine from this app (Kiro Crew's remote
//! instances), in the same window: the header switches between this PC and
//! an instance. Each instance shown gets a webview of its own in the main
//! window, kept warm (hidden, not closed) when another is shown, so going
//! back is instant; [`WARM`] at most, the least recently shown closed
//! first. That webview's commands, events and file previews go to its
//! instance through this module (ui/src/lib/ipc.svelte.ts).
//!
//! Connecting, two ways. Over SSH: an SSH tunnel is opened to it, a pairing
//! token is minted there (`divixi-server token`), and that token is
//! exchanged for the device's access and refresh tokens. A divixi-server
//! that is *not* running is not started by connecting -- see the note on
//! [`connect`] -- so the interface offers that as a button of its own
//! ([`super::install::remote_server_start`]). At an address (a Divixi that
//! serves on its network, such as over Tailscale): this PC's GitHub token
//! is shown to it, and it lets in its owner. The tokens stay here, in Rust;
//! the page never holds them. Events come over one WebSocket per instance
//! and are re-sent to the page as `instance-event`.
//!
//! SSH runs non-interactively (BatchMode): the host must be reachable with
//! a key or an agent, as `ssh <host>` in a terminal would be. Its options
//! are Kiro Crew's for supervised tunnels.

use std::borrow::Cow;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::{AppHandle, AppState};

const HOSTS_KEY: &str = "setting:remote.hosts";

/// A Divixi on another machine, reached over SSH or at an address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    /// "ssh" (a tunnel, signed in over SSH) or "direct" (an address, signed
    /// in with this PC's GitHub account).
    #[serde(default = "default_kind")]
    pub kind: String,
    /// The address for "direct": `http://host:port`.
    #[serde(default)]
    pub url: String,
    /// What follows `ssh`: a name, `user@host`, or an ~/.ssh/config alias.
    pub ssh: String,
    /// divixi-server's port on that machine.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Where divixi-server is there (a login shell resolves `~`).
    #[serde(default = "default_bin")]
    pub bin: String,
    /// Put in front of PATH there when starting divixi-server, so it finds
    /// the agents (as Kiro Crew's "Remote PATH"). Empty: the SSH shell's own.
    #[serde(default)]
    pub path: String,
}

fn default_kind() -> String {
    "ssh".to_string()
}

fn default_port() -> u16 {
    super::DEFAULT_PORT
}

fn default_bin() -> String {
    "~/.local/bin/divixi-server".to_string()
}

/// How an instance's build stands to this app's.
///
/// The three cases are three different sentences, and the app used to show
/// one of them for all of them. An instance from before `/api/health`
/// carried a build answers with an empty one, and comparing that to this
/// app's hash says "built from other code" -- which reads as "someone built
/// the wrong commit" when what happened is that the server there is older
/// than the field itself. [`TooOld`](Freshness::TooOld) is that case, and
/// what it asks of the user (update it) is not what
/// [`Other`](Freshness::Other) asks (put the two on the same commit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Nothing is connected, so the instance has said nothing.
    Unknown,
    /// The same Rust sources as this app: every command matches.
    Same,
    /// Another build of them: some commands may not match.
    Other,
    /// It is answering and names no build at all, which only a
    /// divixi-server from before that field does.
    TooOld,
}

/// A host as the settings page and the switcher show it.
#[derive(Serialize)]
pub struct HostView {
    #[serde(flatten)]
    pub host: Host,
    /// The local port of its tunnel while connected over SSH.
    pub local_port: Option<u16>,
    pub connected: bool,
    /// Connected, and its link answering now.
    pub online: bool,
    /// Its version and build (a hash of its sources), once connected.
    pub version: Option<String>,
    pub build: Option<String>,
    /// Which release it is, when it says (a newer divixi-server does).
    pub release: Option<String>,
    /// Built from other code than this app: some commands may not match.
    /// True for both [`Freshness::Other`] and [`Freshness::TooOld`], which
    /// is why `freshness` is what says *why*.
    pub stale: bool,
    /// Why it is (or is not) out of step.
    pub freshness: Freshness,
}

/// This app's build (see build.rs): an instance with another is out of step.
pub const BUILD: &str = env!("DIVIXI_BUILD");

/// What an instance's build says, once it has answered.
fn freshness(build: &str) -> Freshness {
    if build.is_empty() {
        Freshness::TooOld
    } else if build == BUILD {
        Freshness::Same
    } else {
        Freshness::Other
    }
}

/// What the open connection's `/api/health` said, for [`super::install`] to
/// weigh against the release it would put there. `None` when nothing is
/// connected: then only the files on that machine can say anything.
pub(super) async fn seen(app: &AppHandle, id: &str) -> Option<super::install::Seen> {
    let conn = app.state::<AppState>().tunnels.open.lock().await.get(id).filter(|c| c.alive()).cloned()?;
    Some(super::install::Seen { build: conn.build.clone(), release: conn.release.clone() })
}

/// Point the instance at another divixi-server there: the app has just
/// installed one, and this is the path the next connection starts.
pub(super) fn set_bin(app: &AppHandle, id: &str, bin: &str) -> Result<(), String> {
    check_bin(bin)?;
    let mut list = hosts(app);
    let host = list.iter_mut().find(|h| h.id == id).ok_or("no such remote instance")?;
    if host.bin == bin {
        return Ok(());
    }
    host.bin = bin.to_string();
    save(app, &list)
}

/// Open connections, by host id.
#[derive(Default)]
pub struct Tunnels {
    open: tokio::sync::Mutex<HashMap<String, Arc<Conn>>>,
    /// One connect at a time per host; the map is only held for a moment,
    /// so a slow SSH never holds up the others (or the list).
    gates: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Instances with a webview, least recently shown first.
    panes: parking_lot::Mutex<Vec<String>>,
}

impl Tunnels {
    fn gate(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.gates.lock().entry(id.to_string()).or_default().clone()
    }
}

struct Conn {
    host: Host,
    local_port: Option<u16>,
    base: String,
    /// The SSH tunnel; none for an address.
    tunnel: tokio::sync::Mutex<Option<tokio::process::Child>>,
    tokens: parking_lot::Mutex<Tokens>,
    http: reqwest::Client,
    events: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Shells this app opened there, closed with the connection.
    terms: parking_lot::Mutex<Vec<u64>>,
    /// Renewals one at a time: a refresh token used twice drops the device.
    renewing: tokio::sync::Mutex<()>,
    /// The event socket is up and answering pings.
    online: std::sync::atomic::AtomicBool,
    /// What its /api/health said when connecting.
    version: String,
    build: String,
    release: String,
}

/// The header's indicator: `instance-status` `{id, online}` whenever an
/// instance's link comes up or goes down.
fn set_online(app: &AppHandle, conn: &Conn, online: bool) {
    if conn.online.swap(online, std::sync::atomic::Ordering::Relaxed) != online {
        let _ = app.emit("instance-status", json!({ "id": conn.host.id, "online": online }));
    }
}

#[derive(Clone, Deserialize)]
struct Tokens {
    access: String,
    refresh: String,
}

impl Conn {
    fn alive(&self) -> bool {
        self.tunnel.try_lock().map(|mut c| c.as_mut().is_none_or(|c| matches!(c.try_wait(), Ok(None)))).unwrap_or(true)
    }

    async fn close(&self, app: &AppHandle) {
        if let Some(h) = self.events.lock().take() {
            h.abort();
        }
        set_online(app, self, false);
        // Our shells there go with us (a moment each, at most).
        let terms = std::mem::take(&mut *self.terms.lock());
        for id in terms {
            let _ = tokio::time::timeout(Duration::from_secs(2), call(app, self, "term_close", &json!({ "id": id }))).await;
        }
        if let Some(c) = self.tunnel.lock().await.as_mut() {
            let _ = c.kill().await;
        }
    }
}

pub(super) fn hosts(app: &AppHandle) -> Vec<Host> {
    app.state::<AppState>().store.get_meta(HOSTS_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn save(app: &AppHandle, list: &[Host]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    app.state::<AppState>().store.set_meta(HOSTS_KEY, &json).map_err(|e| e.to_string())
}

/// An `ssh` argument that cannot be read as an option.
fn check_ssh(target: &str) -> Result<(), String> {
    if target.trim().is_empty() || target.split(['@', ':']).any(|p| p.starts_with('-')) || target.contains(char::is_whitespace) {
        return Err("the SSH host is a name, user@host or an ~/.ssh/config alias".into());
    }
    Ok(())
}

/// The remote command's path: letters, digits and the usual path signs only.
fn check_bin(bin: &str) -> Result<(), String> {
    let ok = !bin.is_empty() && bin.chars().all(|c| c.is_ascii_alphanumeric() || "~/._-".contains(c)) && !bin.contains("..");
    if ok {
        Ok(())
    } else {
        Err("the divixi-server path may hold letters, digits, ~ / . _ - only".into())
    }
}

/// The remote PATH: directories as in a path, joined by `:`.
fn check_path(path: &str) -> Result<(), String> {
    if path.chars().all(|c| c.is_ascii_alphanumeric() || "~/._-:".contains(c)) {
        Ok(())
    } else {
        Err("the remote PATH may hold letters, digits, ~ / . _ - : only".into())
    }
}

/// The shell line run there: start divixi-server if this user has none
/// running (detached, so it outlives the SSH session), then mint a link.
///
/// The first half never fires as things stand: [`connect`] only reaches this
/// once the tunnel has already answered, and a tunnel that answers means a
/// server is already up, so `pgrep` always finds one. See the note on
/// [`connect`].
fn remote_line(host: &Host) -> String {
    let path = if host.path.is_empty() { String::new() } else { format!("PATH={}:\"$PATH\"; export PATH; ", host.path) };
    let bin = &host.bin;
    format!(
        "{path}pgrep -u \"$(id -u)\" -x divixi-server >/dev/null || setsid -f {bin} serve >/dev/null 2>&1 </dev/null; {bin} token"
    )
}

pub(super) fn ssh() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("ssh");
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Never ask: no terminal to answer in. Never share a connection: a shared
    // one hands the forward over and exits, and the tunnel is no longer ours.
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

/// An instance's address: plain http to a host and port (the network
/// under it, Tailscale say, keeps it private).
fn check_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("http://").ok_or("the address starts with http:// (over Tailscale or a trusted network)")?;
    let ok = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c));
    if ok {
        Ok(())
    } else {
        Err("the address is http://host:port, nothing after it".into())
    }
}

/// A free local port (the OS picks one).
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())
}

#[derive(Deserialize, Default)]
struct Health {
    #[serde(default)]
    version: String,
    #[serde(default)]
    build: String,
    /// Which release that divixi-server is, empty for one built from a
    /// working copy -- and empty too from an instance older than this field.
    #[serde(default)]
    release: String,
}

/// The instance's /api/health, if it answers.
/// A transport failure as it can be shown or logged: without the URL.
///
/// reqwest's `Display` appends the address it was given, query string and
/// all (it strips only the userinfo, which it moves into a header). None of
/// these addresses carries a credential today — the tokens go in headers —
/// so this is a floor, not a fix: what is useful in a message here is the
/// failure, and the address is one the settings page already shows.
fn why(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

async fn health(http: &reqwest::Client, base: &str) -> Option<Health> {
    let r = http.get(format!("{base}/api/health")).send().await.ok().filter(|r| r.status().is_success())?;
    // An older instance says only {"ok":true}: no build to compare.
    Some(serde_json::from_slice(&r.bytes().await.ok()?).unwrap_or_default())
}

/// Wait until the tunnel answers, or say why not.
async fn wait_for(http: &reqwest::Client, base: &str, child: &mut tokio::process::Child) -> Result<Health, String> {
    for _ in 0..60 {
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            return Err(format!("ssh ended ({status}): {}", err.trim()));
        }
        if let Some(h) = health(http, base).await {
            return Ok(h);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("the remote instance did not answer through the tunnel: is divixi-server running there?".into())
}

/// Start the remote Divixi if need be and take a pairing token from it.
async fn start_and_token(host: &Host) -> Result<String, String> {
    let out = ssh()
        .arg(&host.ssh)
        .arg(remote_line(host))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("could not run ssh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("/auth/pair?token=")).ok_or_else(|| {
        format!("{} token gave no link: {}", host.bin, String::from_utf8_lossy(&out.stderr).trim())
    })?;
    Ok(line.split("/auth/pair?token=").nth(1).unwrap_or_default().trim().to_string())
}

/// This PC as the remote's device list names it.
fn device_name() -> String {
    format!("DIVIXI · {}", sysinfo::System::host_name().unwrap_or_else(|| "PC".into()))
}

/// Sign in for the device's tokens: a pairing token minted over SSH, or
/// this PC's GitHub token at an address.
async fn sign_in(app: &AppHandle, http: &reqwest::Client, base: &str, host: &Host) -> Result<Tokens, String> {
    let (path, token) = if host.kind == "direct" {
        let gh = super::github::account(app);
        if gh.token.is_empty() {
            return Err("sign in to GitHub first (Settings › Remote instances): an instance at an address lets in its owner's GitHub account".into());
        }
        ("github", gh.token)
    } else {
        ("token", start_and_token(host).await?)
    };
    let res = http
        .post(format!("{base}/auth/{path}"))
        .header("X-DIVIXI", "1")
        .header("Content-Type", "application/json")
        .body(json!({ "token": token, "name": device_name() }).to_string())
        .send()
        .await
        .map_err(why)?;
    let status = res.status();
    let body = res.bytes().await.map_err(why)?;
    if !status.is_success() {
        let why = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
        return Err(format!("the remote instance refused the sign-in ({status}){}", why.map(|w| format!(": {w}")).unwrap_or_default()));
    }
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// New tokens, after `stale` (the access token) was refused: from the
/// refresh token, or (refused too) a fresh sign-in. Callers that were
/// refused together renew once; the others find new tokens already there.
async fn renew(app: &AppHandle, conn: &Conn, stale: &str) -> Result<(), String> {
    let _one = conn.renewing.lock().await;
    if conn.tokens.lock().access != stale {
        return Ok(());
    }
    let refresh = conn.tokens.lock().refresh.clone();
    let res = conn.http.post(format!("{}/auth/refresh", conn.base)).header("X-DIVIXI", "1").bearer_auth(refresh).send().await;
    let tokens = match res {
        Ok(r) if r.status().is_success() => serde_json::from_slice(&r.bytes().await.map_err(why)?).map_err(|e| e.to_string())?,
        _ => sign_in(app, &conn.http, &conn.base, &conn.host).await?,
    };
    *conn.tokens.lock() = tokens;
    Ok(())
}

/// The open connection, if it is still alive.
async fn live(app: &AppHandle, id: &str) -> Option<Arc<Conn>> {
    app.state::<AppState>().tunnels.open.lock().await.get(id).filter(|c| c.alive()).cloned()
}

/// Reach the instance: an SSH tunnel and a signed-in device, or the same at
/// an address.
///
/// **A stopped divixi-server is not started here, though the order below
/// reads as if it would be.** The tunnel is opened, [`wait_for`] polls
/// `/api/health` for fifteen seconds, and only then does [`sign_in`] run
/// [`remote_line`] -- the line carrying the `pgrep || setsid` that would
/// have started one. Nothing is listening, so `wait_for` gives up first and
/// that half is never reached.
///
/// Left as it is on purpose, to be fixed against a real remote machine
/// rather than a guess. The fix moves the start ahead of the tunnel -- only
/// the start half of [`remote_line`], since the token still has to be
/// exchanged *through* the tunnel. Until then the interface offers a button
/// ([`super::install::remote_server_start`]).
async fn connect(app: &AppHandle, id: &str) -> Result<Arc<Conn>, String> {
    if let Some(c) = live(app, id).await {
        return Ok(c);
    }
    let tunnels = &app.state::<AppState>().tunnels;
    let gate = tunnels.gate(id);
    let _one = gate.lock().await;
    // Another call may have connected while this one waited.
    let dead = {
        let mut open = tunnels.open.lock().await;
        match open.get(id) {
            Some(c) if c.alive() => return Ok(c.clone()),
            // Its ssh has ended: the connection is gone.
            Some(_) => open.remove(id),
            None => None,
        }
    };
    if let Some(c) = dead {
        c.close(app).await;
    }
    let host = hosts(app).into_iter().find(|h| h.id == id).ok_or("no such remote instance")?;
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let (base, local_port, mut child, said) = if host.kind == "direct" {
        let base = host.url.trim_end_matches('/').to_string();
        let Some(said) = health(&http, &base).await else {
            return Err(format!("{base} did not answer: is that DIVIXI serving on its network?"));
        };
        (base, None, None, said)
    } else {
        let local_port = free_port()?;
        let base = format!("http://127.0.0.1:{local_port}");
        let mut child = ssh()
            .args(["-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3", "-L"])
            .arg(format!("127.0.0.1:{local_port}:127.0.0.1:{}", host.port))
            .arg(&host.ssh)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run ssh: {e}"))?;
        let said = wait_for(&http, &base, &mut child).await?;
        (base, Some(local_port), Some(child), said)
    };
    let tokens = match sign_in(app, &http, &base, &host).await {
        Ok(t) => t,
        Err(e) => {
            if let Some(c) = child.as_mut() {
                let _ = c.kill().await;
            }
            return Err(e);
        }
    };
    let conn = Arc::new(Conn {
        host,
        local_port,
        base,
        tunnel: tokio::sync::Mutex::new(child),
        tokens: parking_lot::Mutex::new(tokens),
        http,
        events: parking_lot::Mutex::new(None),
        terms: parking_lot::Mutex::new(Vec::new()),
        renewing: tokio::sync::Mutex::new(()),
        online: std::sync::atomic::AtomicBool::new(false),
        version: said.version,
        build: said.build,
        release: said.release,
    });
    let pump = tauri::async_runtime::spawn(pump_events(app.clone(), conn.clone()));
    *conn.events.lock() = Some(pump);
    tunnels.open.lock().await.insert(id.to_string(), conn.clone());
    Ok(conn)
}

/// A command on the instance, as the page would call it here.
async fn call(app: &AppHandle, conn: &Conn, cmd: &str, args: &Value) -> Result<Value, String> {
    let url = format!("{}/api/invoke/{cmd}", conn.base);
    let body = args.to_string();
    let mut renewed = false;
    loop {
        let access = conn.tokens.lock().access.clone();
        let res = conn
            .http
            .post(&url)
            .header("X-DIVIXI", "1")
            .header("Content-Type", "application/json")
            .bearer_auth(&access)
            .body(body.clone())
            .send()
            .await
            .map_err(|e| format!("the remote instance did not answer: {}", why(e)))?;
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && !renewed {
            renew(app, conn, &access).await?;
            renewed = true;
            continue;
        }
        let v: Value = serde_json::from_slice(&res.bytes().await.map_err(why)?).unwrap_or_else(|_| json!({ "error": format!("{status}") }));
        return match v.get("error") {
            Some(e) => Err(e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())),
            None => Ok(v.get("ok").cloned().unwrap_or(Value::Null)),
        };
    }
}

/// How often the app pings the instance over its event socket.
const PING: Duration = Duration::from_secs(10);
/// How long the socket may stay silent (a pong is due within a ping or two).
const SILENCE: Duration = Duration::from_secs(25);

/// The instance's events, re-sent to the page as `instance-event`
/// (`{id, frame}`), picking up where it left off after a drop.
async fn pump_events(app: AppHandle, conn: Arc<Conn>) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let mut since = 0u64;
    let mut backoff = Duration::from_secs(1);
    loop {
        let url = format!("{}/api/events?since={since}", conn.base.replacen("http://", "ws://", 1));
        let Ok(mut req) = url.into_client_request() else { return };
        let access = conn.tokens.lock().access.clone();
        if let Ok(v) = format!("Bearer {access}").parse() {
            req.headers_mut().insert("Authorization", v);
        }
        match tokio_tungstenite::connect_async(req).await {
            Ok((mut ws, _)) => {
                backoff = Duration::from_secs(1);
                set_online(&app, &conn, true);
                // A ping every 10 s; this long without a word (a pong at
                // least), the link is gone even if no one said so.
                let mut ping = tokio::time::interval(PING);
                let mut heard = tokio::time::Instant::now();
                loop {
                    tokio::select! {
                        _ = ping.tick() => {
                            if heard.elapsed() > SILENCE || ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                                break;
                            }
                        }
                        got = ws.next() => {
                            let Some(Ok(msg)) = got else { break };
                            heard = tokio::time::Instant::now();
                            let Message::Text(text) = msg else { continue };
                            let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
                            if let Some(seq) = frame.get("seq").and_then(Value::as_u64) {
                                since = seq;
                            }
                            let _ = app.emit("instance-event", json!({ "id": conn.host.id, "frame": frame }));
                        }
                    }
                }
                set_online(&app, &conn, false);
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) if res.status() == 401 => {
                if renew(&app, &conn, &access).await.is_ok() {
                    continue;
                }
                // Not let in again (each try may be an SSH sign-in): wait longer each time.
            }
            Err(_) => {}
        }
        if !conn.alive() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// `/@<id>/<rest>` on the preview and board schemes: an instance's file.
pub fn instance_path(uri: &tauri::http::Uri) -> Option<(String, String)> {
    let rest = uri.path().strip_prefix("/@")?;
    let (id, rest) = rest.split_once('/')?;
    let rest = match uri.query() {
        Some(q) => format!("{rest}?{q}"),
        None => rest.to_string(),
    };
    Some((id.to_string(), rest))
}

/// An instance's file for the preview (`kind` "preview") or board ("board")
/// scheme, fetched with the device's token and passed on with its type and
/// policy (a preview comes sandboxed).
pub async fn proxy(app: &AppHandle, id: &str, kind: &str, rest: &str) -> tauri::http::Response<Cow<'static, [u8]>> {
    let reply = |status: u16, text: &'static str| tauri::http::Response::builder().status(status).body(Cow::Borrowed(text.as_bytes())).expect("a plain reply");
    let Some(conn) = app.state::<AppState>().tunnels.open.lock().await.get(id).cloned() else {
        return reply(502, "not connected to that remote instance");
    };
    let url = format!("{}/{kind}/{rest}", conn.base);
    let mut renewed = false;
    loop {
        let access = conn.tokens.lock().access.clone();
        let Ok(res) = conn.http.get(&url).bearer_auth(&access).send().await else { return reply(502, "the remote instance did not answer") };
        if res.status() == reqwest::StatusCode::UNAUTHORIZED && !renewed && renew(app, &conn, &access).await.is_ok() {
            renewed = true;
            continue;
        }
        let mut out = tauri::http::Response::builder().status(res.status().as_u16());
        for h in ["content-type", "content-security-policy"] {
            if let Some(v) = res.headers().get(h).and_then(|v| v.to_str().ok()) {
                out = out.header(h, v);
            }
        }
        let body = res.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        return out.body(Cow::Owned(body)).unwrap_or_else(|_| reply(500, "bad reply"));
    }
}

async fn disconnect(app: &AppHandle, id: &str) {
    close_pane(app, id);
    let conn = app.state::<AppState>().tunnels.open.lock().await.remove(id);
    if let Some(c) = conn {
        c.close(app).await;
    }
}

// ----- a webview per instance shown -----

/// Webviews kept (hidden) besides the one shown, at most.
const WARM: usize = 3;

fn pane_label(id: &str) -> String {
    format!("inst-{id}")
}

fn close_pane(app: &AppHandle, id: &str) {
    app.state::<AppState>().tunnels.panes.lock().retain(|p| p != id);
    if let Some(w) = app.get_webview(&pane_label(id)) {
        let _ = w.close();
    }
}

/// Show an instance in the main window (its warm webview, or a new one), or
/// this PC (`None`: the window's own webview, under the others).
#[tauri::command]
pub async fn show_instance(app: AppHandle, id: Option<String>) -> Result<(), String> {
    let window = app.get_window("main").ok_or("the app has no window")?;
    let tunnels = &app.state::<AppState>().tunnels;
    let shown = id.as_deref().map(pane_label);
    if let Some(id) = &id {
        let host = hosts(&app).into_iter().find(|h| &h.id == id).ok_or("no such remote instance")?;
        let label = pane_label(id);
        if app.get_webview(&label).is_none() {
            // The page learns which instance it shows (and its name, for
            // notifications) before it runs.
            let script = format!(
                "window.__DIVIXI_INSTANCE__ = {}; window.__DIVIXI_INSTANCE_NAME__ = {};",
                serde_json::Value::String(id.clone()),
                serde_json::Value::String(host.name)
            );
            let builder = tauri::webview::WebviewBuilder::new(&label, tauri::WebviewUrl::App("index.html".into())).initialization_script(&script).auto_resize();
            let size = window.inner_size().map_err(|e| e.to_string())?;
            window.add_child(builder, tauri::PhysicalPosition::new(0, 0), size).map_err(|e| e.to_string())?;
        }
        let mut panes = tunnels.panes.lock();
        panes.retain(|p| p != id);
        panes.push(id.clone());
    }
    // The others hide; past WARM kept, the least recently shown close.
    let panes: Vec<String> = tunnels.panes.lock().clone();
    let keep = WARM + usize::from(id.is_some());
    let over = panes.len().saturating_sub(keep);
    for (i, p) in panes.iter().enumerate() {
        let label = pane_label(p);
        let Some(w) = app.get_webview(&label) else { continue };
        if Some(&label) == shown.as_ref() {
            let _ = w.show();
            let _ = w.set_focus();
        } else if i < over {
            close_pane(&app, p);
        } else {
            let _ = w.hide();
        }
    }
    if id.is_none() {
        if let Some(main) = app.get_webview("main") {
            let _ = main.set_focus();
        }
    }
    Ok(())
}

// ----- commands (this app's own page; never reachable remotely) -----

#[tauri::command]
pub async fn remote_hosts(app: AppHandle) -> Vec<HostView> {
    let open = app
        .state::<AppState>()
        .tunnels
        .open
        .lock()
        .await
        .iter()
        .filter(|(_, c)| c.alive())
        .map(|(k, c)| (k.clone(), c.clone()))
        .collect::<HashMap<_, _>>();
    hosts(&app)
        .into_iter()
        .map(|h| match open.get(&h.id) {
            Some(c) => HostView {
                local_port: c.local_port,
                connected: true,
                online: c.online.load(std::sync::atomic::Ordering::Relaxed),
                version: (!c.version.is_empty()).then(|| c.version.clone()),
                build: (!c.build.is_empty()).then(|| c.build.clone()),
                release: (!c.release.is_empty()).then(|| c.release.clone()),
                stale: c.build != BUILD,
                freshness: freshness(&c.build),
                host: h,
            },
            None => HostView { local_port: None, connected: false, online: false, version: None, build: None, release: None, stale: false, freshness: Freshness::Unknown, host: h },
        })
        .collect()
}

/// Add a host, or change one (same id).
#[tauri::command]
pub async fn remote_host_save(app: AppHandle, mut host: Host) -> Result<Vec<HostView>, String> {
    host.ssh = host.ssh.trim().to_string();
    host.bin = host.bin.trim().to_string();
    host.name = host.name.trim().to_string();
    host.path = host.path.trim().to_string();
    host.url = host.url.trim().trim_end_matches('/').to_string();
    if host.kind == "direct" {
        check_url(&host.url)?;
    } else {
        host.kind = "ssh".into();
        check_ssh(&host.ssh)?;
        check_bin(&host.bin)?;
        check_path(&host.path)?;
    }
    if host.name.is_empty() {
        host.name = if host.kind == "direct" { host.url.clone() } else { host.ssh.clone() };
    }
    if host.id.is_empty() {
        host.id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    } else {
        // A changed host is reached anew.
        disconnect(&app, &host.id).await;
    }
    let mut list = hosts(&app);
    match list.iter_mut().find(|h| h.id == host.id) {
        Some(h) => *h = host,
        None => list.push(host),
    }
    save(&app, &list)?;
    Ok(remote_hosts(app).await)
}

#[tauri::command]
pub async fn remote_host_delete(app: AppHandle, id: String) -> Result<Vec<HostView>, String> {
    disconnect(&app, &id).await;
    let mut list = hosts(&app);
    list.retain(|h| h.id != id);
    save(&app, &list)?;
    // What this PC remembered of its screen (its zoom, `instanceZoomKey` in
    // ui/src/lib/uiMemory.ts) goes with it.
    let _ = app.state::<AppState>().store.delete_meta(&format!("{}instance:{id}:zoom", crate::CLIENT_SETTING_PREFIX));
    Ok(remote_hosts(app).await)
}

/// Connect to the instance (or keep the open connection).
#[tauri::command]
pub async fn remote_host_connect(app: AppHandle, id: String) -> Result<Vec<HostView>, String> {
    connect(&app, &id).await?;
    Ok(remote_hosts(app).await)
}

#[tauri::command]
pub async fn remote_host_disconnect(app: AppHandle, id: String) -> Vec<HostView> {
    disconnect(&app, &id).await;
    remote_hosts(app).await
}

/// A command for the chosen instance, from the page (ipc.svelte.ts). The
/// instance's own rules decide what it runs (remote/bridge.rs).
#[tauri::command]
pub async fn instance_invoke(app: AppHandle, id: String, cmd: String, args: Value) -> Result<Value, String> {
    if cmd.is_empty() || !cmd.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("not a command name".into());
    }
    let conn = connect(&app, &id).await?;
    let out = call(&app, &conn, &cmd, &args).await?;
    // Shells opened there are ours to close when we go.
    match cmd.as_str() {
        "term_open" => conn.terms.lock().extend(out.as_u64()),
        "term_close" => {
            let gone = args.get("id").and_then(Value::as_u64);
            conn.terms.lock().retain(|t| Some(*t) != gone);
        }
        _ => {}
    }
    Ok(out)
}

/// A track's file on the instance, saved on this PC where the user says.
#[tauri::command]
pub async fn instance_save_as(app: AppHandle, id: String, track: String, path: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let conn = connect(&app, &id).await?;
    let enc = |s: &str| s.split('/').map(|p| p.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>()).collect::<Vec<_>>().join("/");
    let url = format!("{}/raw/{}/{}", conn.base, enc(&track), enc(&path));
    let mut renewed = false;
    let bytes = loop {
        let access = conn.tokens.lock().access.clone();
        let res = conn.http.get(&url).bearer_auth(&access).send().await.map_err(|e| format!("the remote instance did not answer: {}", why(e)))?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED && !renewed {
            renew(&app, &conn, &access).await?;
            renewed = true;
            continue;
        }
        if !res.status().is_success() {
            return Err(format!("{path}: {}", res.status()));
        }
        break res.bytes().await.map_err(why)?;
    };
    let name = path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("file").to_string();
    let mut dialog = app.dialog().file().set_file_name(&name);
    if let Some(dir) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from).map(|h| h.join("Downloads")).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.save_file(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(target) = rx.await.map_err(|e| e.to_string())?.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    tokio::fs::write(&target, &bytes).await.map_err(|e| format!("could not save {}: {e}", target.display()))?;
    Ok(Some(target.display().to_string()))
}

/// Files of this PC (picked or dropped) taken to the instance: each is kept
/// there as an attachment (`save_attachment`), and its path there returned
/// in the same order, for the command that wanted them.
#[tauri::command]
pub async fn instance_upload(app: AppHandle, id: String, paths: Vec<String>) -> Result<Vec<String>, String> {
    use base64::Engine;
    let conn = connect(&app, &id).await?;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let path = std::path::Path::new(&p);
        let meta = tokio::fs::metadata(path).await.map_err(|e| format!("{p}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{p}: only files can be sent to a remote instance"));
        }
        if meta.len() > 50 * 1024 * 1024 {
            return Err(format!("{p}: files over 50 MB are not sent"));
        }
        let bytes = tokio::fs::read(path).await.map_err(|e| format!("{p}: {e}"))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let there = call(&app, &conn, "save_attachment", &json!({ "name": name, "data": data })).await?;
        out.push(there.as_str().ok_or("the remote instance gave no path")?.to_string());
    }
    Ok(out)
}

/// End every connection (the app is quitting; a server has no tray to quit from).
#[cfg_attr(feature = "server", allow(dead_code))]
pub fn close_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let conns: Vec<Arc<Conn>> = match state.tunnels.open.try_lock() {
        Ok(mut open) => open.drain().map(|(_, c)| c).collect(),
        Err(_) => return,
    };
    // Shells opened there are closed first; the whole goodbye is kept short.
    let app2 = app.clone();
    let _ = tauri::async_runtime::block_on(async move {
        tokio::time::timeout(Duration::from_secs(3), async {
            for c in &conns {
                c.close(&app2).await;
            }
        })
        .await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_targets_and_paths_are_checked() {
        assert!(check_ssh("user@build-host").is_ok());
        assert!(check_ssh("my-alias").is_ok());
        assert!(check_ssh("-oProxyCommand=evil").is_err());
        assert!(check_ssh("user@-oevil").is_err());
        assert!(check_ssh("a b").is_err());
        assert!(check_bin("~/.local/bin/divixi-server").is_ok());
        assert!(check_bin("~/x; rm -rf /").is_err());
        assert!(check_bin("../../bin/sh").is_err());
        assert!(check_path("").is_ok());
        assert!(check_path("~/.local/bin:/usr/bin:/bin").is_ok());
        assert!(check_path("/bin;reboot").is_err());
        assert!(check_path("$(reboot)").is_err());
        assert!(check_url("http://my-pc:7488").is_ok());
        assert!(check_url("http://100.64.1.2:7488").is_ok());
        assert!(check_url("https://my-pc").is_err(), "the event socket is plain ws");
        assert!(check_url("http://my-pc:7488/x?y").is_err());
    }

    #[test]
    fn the_remote_line_starts_the_server_once() {
        let mut h = Host { id: "a".into(), name: "a".into(), kind: "ssh".into(), url: String::new(), ssh: "a".into(), port: 7488, bin: "~/x/divixi-server".into(), path: String::new() };
        assert_eq!(
            remote_line(&h),
            "pgrep -u \"$(id -u)\" -x divixi-server >/dev/null || setsid -f ~/x/divixi-server serve >/dev/null 2>&1 </dev/null; ~/x/divixi-server token"
        );
        h.path = "~/.local/bin".into();
        assert!(remote_line(&h).starts_with("PATH=~/.local/bin:\"$PATH\"; export PATH; pgrep"));
    }

    #[test]
    fn instance_paths() {
        let uri: tauri::http::Uri = "http://preview.localhost/@ab12/tr001/index.html?x=1".parse().unwrap();
        assert_eq!(instance_path(&uri), Some(("ab12".into(), "tr001/index.html?x=1".into())));
        let local: tauri::http::Uri = "http://preview.localhost/tr001/index.html".parse().unwrap();
        assert_eq!(instance_path(&local), None);
    }
}
