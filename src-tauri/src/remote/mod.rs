//! Remote instances.
//!
//! Any Divixi can be a remote instance for other PCs' Divixi apps: the
//! headless divixi-server always is, the desktop app when its settings say
//! so. An axum server ([`server`]) carries the app's commands (through
//! [`bridge`], a listed few) and events (through [`events`]) over HTTP and a
//! WebSocket. Who may come in is [`auth`]'s, with this machine's pairing
//! tokens (over SSH) or its owner's GitHub account ([`github`]). The other
//! side, reaching instances from this app, is [`client`].
//!
//! What this looks like to whoever runs one -- installing the headless server,
//! connecting to it, keeping it running and updating it -- is
//! docs/divixi-server.md.

pub mod auth;
pub mod awake;
pub mod bridge;
pub mod client;
pub mod events;
pub mod github;
pub mod peer;
pub mod install;
pub mod server;
pub mod tailscale;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::AppHandle;

use crate::AppState;

/// The port a Divixi answers on unless the `remote.port` setting says another,
/// and the port another PC's server is expected on.
pub const DEFAULT_PORT: u16 = 7488;

/// The port this build listens on when nothing says otherwise.
///
/// A dev build takes the one above. It runs beside the installed app (its own
/// data folder, its own window title), and both used to reach for 7488: the
/// installed app held it, the dev build could not bind, and Tailscale kept
/// forwarding to a release build that had none of the code being tried.
/// Nothing about the dev build was visibly wrong; it simply was not the one
/// answering.
pub const LISTEN_PORT: u16 = if cfg!(debug_assertions) { DEFAULT_PORT + 1 } else { DEFAULT_PORT };

pub struct Remote {
    pub auth: auth::Auth,
    pub events: Arc<events::Hub>,
    running: tokio::sync::Mutex<Option<Running>>,
    /// Shells another PC opened here, by terminal id: the device's.
    pub terms: parking_lot::Mutex<std::collections::HashMap<u64, String>>,
    /// Event sockets open, by device: a device with none for a while is gone.
    pub sockets: parking_lot::Mutex<std::collections::HashMap<String, usize>>,
    /// Held while phone access is on, so this PC does not sleep on the phone.
    /// Dropping it releases, so there is one way back and no second place to
    /// forget: turning phone access off, and the app quitting, both go here.
    awake: parking_lot::Mutex<Option<awake::Guard>>,
    /// Resolved tailnet peers, by address. See [`peer`].
    pub peers: peer::Cache,
    /// The last reading of Tailscale, and when it was taken. See [`look`].
    #[allow(clippy::type_complexity)]
    looked: parking_lot::Mutex<Option<(std::time::Instant, tailscale::Probe, tailscale::ServeState)>>,
}

struct Running {
    port: u16,
    all: bool,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl Remote {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            auth: auth::Auth::open(data_dir)?,
            events: Arc::new(events::Hub::default()),
            running: tokio::sync::Mutex::new(None),
            terms: Default::default(),
            sockets: Default::default(),
            awake: Default::default(),
            peers: Default::default(),
            looked: Default::default(),
        })
    }
}

pub(super) fn setting(app: &AppHandle, key: &str) -> Option<String> {
    app.state::<AppState>().store.get_meta(&format!("{}{key}", crate::SETTING_PREFIX)).ok().flatten()
}

pub(super) fn set(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
    app.state::<AppState>().store.set_meta(&format!("{}{key}", crate::SETTING_PREFIX), value).map_err(|e| e.to_string())
}

fn port(app: &AppHandle) -> u16 {
    setting(app, "remote.port").and_then(|p| p.trim().parse().ok()).filter(|p| *p >= 1024).unwrap_or(LISTEN_PORT)
}

/// Whether to answer on every network (an address other PCs reach
/// directly, such as the Tailscale one) or on this machine only (SSH
/// tunnels arrive there).
fn listen_all(app: &AppHandle) -> bool {
    setting(app, "remote.listen").as_deref() == Some("all")
}

/// Start the server (again, as the settings now say), or leave it running.
pub async fn start(app: &AppHandle) -> Result<u16, String> {
    let st = app.state::<AppState>();
    let (want, all) = (port(app), listen_all(app));
    let mut running = st.remote.running.lock().await;
    if let Some(r) = running.as_ref() {
        if r.port == want && r.all == all {
            return Ok(want);
        }
    }
    if let Some(old) = running.take() {
        let _ = old.stop.send(());
    }
    let ip = if all { "0.0.0.0" } else { "127.0.0.1" };
    let listener = tokio::net::TcpListener::bind((ip, want)).await.map_err(|e| format!("port {want}: {e}"))?;
    let router = server::router(server::Ctx { app: app.clone() });
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
        let serve = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            });
        if let Err(err) = serve.await {
            tracing::warn!(%err, "remote instance server stopped");
        }
    });
    st.remote.events.on.store(true, Ordering::Relaxed);
    *running = Some(Running { port: want, all, stop });
    tracing::info!(port = want, ip, "remote instance server on");
    Ok(want)
}

pub async fn stop(app: &AppHandle) {
    let st = app.state::<AppState>();
    if let Some(r) = st.remote.running.lock().await.take() {
        let _ = r.stop.send(());
        tracing::info!("remote instance server off");
    }
    st.remote.events.on.store(false, Ordering::Relaxed);
}

/// Whether this Divixi serves at all. Two features share the one server:
/// remote instances (another PC's app) and phone access (this PC's own Divixi
/// in a phone browser). Either switch brings it up; it goes down when both are
/// off. They are kept apart everywhere a human can see them, and share only
/// the socket.
fn enabled(app: &AppHandle) -> bool {
    cfg!(feature = "server") || setting(app, "remote.enabled").as_deref() == Some("true") || phone_on(app)
}

/// Phone access: this Divixi, published on this machine's tailnet and opened
/// in a phone's browser. Not remote instances, which is the other way round.
fn phone_on(app: &AppHandle) -> bool {
    setting(app, "phone.enabled").as_deref() == Some("true")
}

/// At startup: the events hub listens, and the server comes up if this
/// Divixi serves (divixi-server always; the desktop app if it was on).
pub fn boot(app: &AppHandle) {
    let st = app.state::<AppState>();
    events::install(app, st.remote.events.clone());
    // `tailscale serve --bg` survives a reboot in Tailscale's own
    // configuration, so this Divixi is still published -- but the sleep
    // assertion is this process's and went with the last one. Without
    // retaking it, phone access comes back on after a restart over a PC that
    // quietly sleeps.
    if phone_on(app) {
        *st.remote.awake.lock() = Some(awake::hold());
    }
    if enabled(app) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = start(&app).await {
                tracing::warn!(%err, "could not start the remote instance server");
            }
        });
    }
}

/// This machine's addresses other PCs might use: its name, and each
/// network's address (a Tailscale one is 100.x).
fn addresses() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(name) = sysinfo::System::host_name() {
        out.push(name);
    }
    // The address the OS would send from, without sending anything.
    for probe in ["8.8.8.8:80", "100.100.100.100:80"] {
        if let Ok(ip) = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(probe).and_then(|_| s.local_addr())).map(|a| a.ip().to_string()) {
            if ip != "0.0.0.0" && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// This Divixi as a remote instance, for its settings page.
#[derive(Serialize)]
pub struct ServerStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    /// Answering on every network (`true`) or this machine only.
    pub all: bool,
    pub addresses: Vec<String>,
    /// The GitHub login that owns it.
    pub owner: String,
    pub devices: Vec<auth::Device>,
}

// ----- commands (this app's own page; never reachable remotely) -----

#[tauri::command]
pub async fn remote_server_status(app: AppHandle) -> ServerStatus {
    let st = app.state::<AppState>();
    let running = st.remote.running.lock().await.as_ref().map(|r| r.port);
    ServerStatus {
        // The remote-instances switch only. `enabled()` is also true when
        // phone access alone brought the server up, and reporting that here
        // would make this card claim a feature is on that nobody turned on.
        enabled: cfg!(feature = "server") || setting(&app, "remote.enabled").as_deref() == Some("true"),
        running: running.is_some(),
        port: running.unwrap_or_else(|| port(&app)),
        all: listen_all(&app),
        addresses: addresses(),
        owner: github::owner(&st.store).unwrap_or_default(),
        devices: st
            .remote
            .auth
            .devices(&st.store)
            .into_iter()
            .map(|mut d| {
                d.name = server::shown_name(&d.name).to_string();
                d
            })
            .collect(),
    }
}

/// Turn serving on or off, and say where and on which port.
#[tauri::command]
pub async fn remote_server_set(app: AppHandle, enabled: bool, all: bool, port: u16) -> Result<ServerStatus, String> {
    if port < 1024 {
        return Err("the port is 1024 or above".into());
    }
    set(&app, "remote.enabled", if enabled { "true" } else { "false" })?;
    set(&app, "remote.listen", if all { "all" } else { "local" })?;
    set(&app, "remote.port", &port.to_string())?;
    if enabled || phone_on(&app) {
        // Phone access may still be on and shares this socket, so the server
        // stays up -- but the bind can have to narrow, which `start` does by
        // restarting when `listen_all` changed.
        start(&app).await?;
    } else {
        stop(&app).await;
    }
    Ok(remote_server_status(app).await)
}

/// Drop one device that came in.
#[tauri::command]
pub async fn remote_server_drop(app: AppHandle, device: String) -> ServerStatus {
    {
        let st = app.state::<AppState>();
        st.remote.auth.drop_device(&st.store, &device);
    }
    remote_server_status(app).await
}

// ----- phone access -----
//
// This Divixi, opened in a phone's browser over this machine's tailnet. NOT
// remote instances, which is this app driving ANOTHER PC's Divixi. They share
// the server above and nothing else, and every string a human reads keeps them
// apart.

/// The one next thing to do about phone access.
///
/// Derived here and nowhere else: the card renders this and never works it out
/// again from the parts, so the two cannot come to disagree about what, say, a
/// signed-in machine with no MagicDNS name means.
///
/// Ordered by what blocks what, so nobody is sent to a switch that cannot help
/// yet. Each is a different errand -- "install Tailscale", "start it", "sign
/// in" and "turn MagicDNS on" are four jobs, not one "Tailscale is broken".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// No Tailscale CLI where the official packages put one.
    Install,
    /// The daemon does not answer, or answers that Tailscale is stopped. One
    /// errand (get Tailscale running); the probe's own words tell them apart.
    StartTailscale,
    SignIn,
    /// Signed in, but this machine has no MagicDNS name.
    EnableMagicDns,
    /// There is a name, but the tailnet will not get a certificate for it.
    /// Tailnet-wide consent that an app cannot give itself.
    EnableHttps,
    /// Serve holds 443/ for something else, or we could not tell. Publishing
    /// would REPLACE it, so this refuses; a mapping read as another's can be
    /// replaced on a confirmed press ([`PhoneStatus::replaceable`]).
    Occupied,
    /// Everything is in place; one press left.
    Publish,
    /// Published. Open the address on a phone.
    Ready,
}

/// Phone access, for its card. Every field is observed now, not remembered.
#[derive(Serialize)]
pub struct PhoneStatus {
    /// The stored intent (`phone.enabled`). `step` is the truth: if someone ran
    /// `tailscale serve off` by hand, this stays true while the step goes back
    /// to `Publish`, and what the card offers follows the step.
    pub on: bool,
    pub step: Step,
    /// `https://<name>`, or empty when there is no name yet.
    pub address: String,
    pub probe: tailscale::Probe,
    pub serve: tailscale::ServeState,
    /// Whether this PC is actually being kept awake, and if not why not.
    /// `None` means Divixi is holding nothing -- serve can be published by
    /// hand, and then the card must not promise a machine that will stay up.
    pub awake: Option<awake::Held>,
    pub port: u16,
    /// Divixi's own server. Phone access needs it, and turns it on itself.
    pub running: bool,
    /// Remote instances has this server answering on every network as well.
    /// Phone access never asks for that -- `tailscale serve` reaches loopback
    /// -- so it is surfaced rather than caused, and the card can say so.
    pub listen_all: bool,
    /// How long a phone paired from here stays signed in, in days.
    pub days: i64,
    /// No other device is online on this tailnet, so there is nothing to open
    /// the address FROM. Publishing still succeeds and the address still looks
    /// right, and then the phone cannot connect with nothing on this machine
    /// wrong -- so it is said before that happens, not after.
    pub alone: bool,
    /// Serve holds 443/ for something this check read and found not to be
    /// this Divixi, so the card may offer to put this one there instead.
    /// Never for a mapping it could not read.
    pub replaceable: bool,
}

fn replaceable(step: Step, serve: &tailscale::ServeState) -> bool {
    step == Step::Occupied && serve.published == Some(false) && serve.port_free == Some(false)
}

fn step_of(probe: &tailscale::Probe, serve: &tailscale::ServeState) -> Step {
    if !probe.installed {
        return Step::Install;
    }
    if !probe.reachable || probe.stopped {
        return Step::StartTailscale;
    }
    if !probe.logged_in {
        return Step::SignIn;
    }
    if probe.name.is_empty() {
        return Step::EnableMagicDns;
    }
    // An already-published mapping is stronger evidence than a possibly stale
    // CertDomains snapshot, and keeps a brief propagation delay after first
    // enablement from taking a working address away. For a NEW mapping an
    // explicit `false` is a hard stop: nothing here can grant tailnet-wide
    // consent for certificates.
    if serve.published != Some(true) && probe.https == Some(false) {
        return Step::EnableHttps;
    }
    if serve.published == Some(true) {
        return Step::Ready;
    }
    // Only a provably free port earns the button. `None` ("could not tell") and
    // `false` (something else holds the mount) are both the destructive
    // direction, so they land together -- the same refusal `publish` itself
    // makes, so the card never offers an action the write side will decline.
    if serve.published == Some(false) && serve.port_free == Some(true) {
        Step::Publish
    } else {
        Step::Occupied
    }
}

/// How long a reading of Tailscale is reused before it is taken again.
///
/// Every read is two subprocesses and up to ten seconds, and the card, the
/// rail's dialog and minting a code all want the same answer within a second
/// or two of each other. Without this, opening the dialog from a warm card
/// paid for the whole thing twice and showing a code paid for it a third
/// time. Short enough that a human who changed something and pressed Check
/// again gets the truth -- and that button asks for a fresh read anyway.
const LOOK_TTL: std::time::Duration = std::time::Duration::from_secs(10);

/// What Tailscale says, taken again only when the last answer is stale.
///
/// `fresh` is for the Check-again button and for just after acting: both are
/// moments when a cached answer would be the wrong one.
async fn look(app: &AppHandle, port: u16, fresh: bool) -> (tailscale::Probe, tailscale::ServeState) {
    if !fresh {
        let seen = app.state::<AppState>().remote.looked.lock().clone();
        if let Some((at, probe, serve)) = seen {
            if at.elapsed() < LOOK_TTL {
                return (probe, serve);
            }
        }
    }
    let probe = tailscale::probe().await;
    // Asking the daemon about serve is only meaningful once it can answer at
    // all; before that the read would be a second way of saying the same
    // thing, five seconds slower.
    let serve = if probe.reachable && !probe.stopped {
        tailscale::serve_state(port).await
    } else {
        tailscale::ServeState::default()
    };
    *app.state::<AppState>().remote.looked.lock() = Some((std::time::Instant::now(), probe.clone(), serve.clone()));
    (probe, serve)
}

/// Forget the last reading, so the next one is taken again. Called after
/// anything that changes what Tailscale would say.
fn forget_look(app: &AppHandle) {
    *app.state::<AppState>().remote.looked.lock() = None;
}

async fn phone_status_now(app: &AppHandle, fresh: bool) -> PhoneStatus {
    let want = port(app);
    let (probe, serve) = look(app, want, fresh).await;
    let step = step_of(&probe, &serve);
    let address = if probe.name.is_empty() { String::new() } else { format!("https://{}", probe.name) };
    // The one origin a browser may name, kept beside the state that decides
    // it rather than written at each place that could change it. Set only
    // while this Divixi is actually published: a phone's every command is a
    // POST, which carries `Origin`, so an origin left behind after turning
    // phone access off would be a door left open with the address hidden.
    let _ = set(app, "phone.origin", if step == Step::Ready { &address } else { "" });
    // `peer::trust` runs on the request path and cannot probe, so the login
    // the daemon reports for this machine is kept where it can read it.
    if !probe.login.is_empty() {
        let _ = set(app, "phone.self_login", &probe.login);
    }
    let st = app.state::<AppState>();
    let running = st.remote.running.lock().await.as_ref().map(|r| r.port);
    let awake = st.remote.awake.lock().as_ref().map(|g| g.held);
    PhoneStatus {
        on: phone_on(app),
        step,
        address,
        awake,
        // Signed in with no other device online: the one thing wrong with this
        // machine that is not on this machine.
        alone: probe.logged_in && !probe.name.is_empty() && probe.peers_online == 0,
        days: phone_days(app),
        port: running.unwrap_or(want),
        running: running.is_some(),
        listen_all: listen_all(app),
        replaceable: replaceable(step, &serve),
        probe,
        serve,
    }
}

/// Where phone access stands: what this machine can do next.
///
/// `fresh` asks Tailscale again rather than reusing the last answer, which is
/// what the Check-again button wants and what an ordinary render does not.
#[tauri::command]
pub async fn phone_status(app: AppHandle, fresh: Option<bool>) -> PhoneStatus {
    phone_status_now(&app, fresh.unwrap_or(false)).await
}

/// Turn phone access on or off.
///
/// On: bring the server up (on loopback -- phone access never asks for every
/// network), publish with `tailscale serve`, and hold this PC awake. Off: the
/// reverse, and the server stays up if remote instances is also using it.
///
/// Only ever from this press. Nothing here runs at startup or as a side effect
/// of something else: publishing puts this machine's Divixi on a tailnet, and
/// that is a thing a person decides.
#[tauri::command]
pub async fn phone_set(app: AppHandle, on: bool) -> Result<PhoneStatus, String> {
    switch(app, on, false).await
}

/// Turn phone access on in place of the mapping serve holds at 443/ for
/// something else. Only from the card's confirmed press.
#[tauri::command]
pub async fn phone_replace(app: AppHandle) -> Result<PhoneStatus, String> {
    switch(app, true, true).await
}

async fn switch(app: AppHandle, on: bool, replace: bool) -> Result<PhoneStatus, String> {
    let want = port(&app);
    if on {
        // The server first: publishing in front of a port nothing is listening
        // on would hand the tailnet a reachable address that refuses.
        set(&app, "phone.enabled", "true")?;
        if let Err(err) = start(&app).await {
            let _ = set(&app, "phone.enabled", "false");
            return Err(err);
        }
        let result = if replace { tailscale::replace(want).await } else { tailscale::publish(want).await };
        if !result.ok {
            // The stored intent goes back: a switch left reading "on" over an
            // address nobody can reach is the working-looking control this
            // whole card exists to avoid.
            let _ = set(&app, "phone.enabled", "false");
            if !enabled(&app) {
                stop(&app).await;
            }
            forget_look(&app);
            return Err(result.detail);
        }
        forget_look(&app);
        *app.state::<AppState>().remote.awake.lock() = Some(awake::hold());
    } else {
        set(&app, "phone.enabled", "false")?;
        // Dropping the guard releases the machine to sleep again.
        *app.state::<AppState>().remote.awake.lock() = None;
        let result = tailscale::unpublish(want).await;
        forget_look(&app);
        if !enabled(&app) {
            stop(&app).await;
        }
        if !result.ok {
            // Off locally either way -- the switch must not stick on because a
            // daemon would not answer -- but say what is still published, and
            // the command that undoes it by hand.
            return Err(result.detail);
        }
    }
    Ok(phone_status_now(&app, false).await)
}


/// How long a phone stays signed in, in days: 1, 7 or 30.
///
/// Read here and signed into every link minted, so the number on the screen
/// and the number in the token are one number.
fn phone_days(app: &AppHandle) -> i64 {
    setting(app, "phone.days")
        .and_then(|d| d.trim().parse().ok())
        .filter(|d| auth::PHONE_DAYS.contains(d))
        .unwrap_or(auth::PHONE_DAYS_DEFAULT)
}

/// A pairing link, and when it stops working.
#[derive(Serialize)]
pub struct PairLink {
    /// `https://<name>/auth/pair?token=…`. A URL and not a bare token, so a
    /// phone's camera offers to open it and the whole thing is one tap.
    pub url: String,
    /// Unix seconds. The card counts down to this and takes the code off the
    /// screen when it passes, rather than leaving a dead one up.
    pub expires: i64,
    /// The span this link grants, for the sentence beside the code.
    pub days: i64,
    /// Which link this is: what `phone_paired` names when a device spends it.
    pub id: String,
}

/// Mint a pairing link for a phone.
///
/// [`auth::Scope::Conversation`] and the configured span, both signed in. Only
/// ever from a press, and only while phone access is actually published: a
/// code for an address that answers nothing is a worse failure than no code,
/// because it fails on the phone where there is nothing to read.
///
/// Never reachable from another device -- it is on neither list in
/// [`bridge`], so a paired phone cannot mint a link for anyone else.
#[tauri::command]
pub async fn phone_pair_link(app: AppHandle) -> Result<PairLink, String> {
    let status = phone_status_now(&app, false).await;
    if status.step != Step::Ready {
        return Err("Phone access is not published yet, so a code would not open anything.".into());
    }
    let days = phone_days(&app);
    let st = app.state::<AppState>();
    let (token, expires) = st.remote.auth.pair_token(&st.store, auth::Scope::Conversation, days * 24 * 60 * 60);
    let id = st.remote.auth.link_id(&token).unwrap_or_default();
    // The app, with the token in the query. It takes it, keeps it and
    // takes it back out of the address bar; there is no page in between.
    Ok(PairLink { url: format!("{}/?token={token}", status.address), expires, days, id })
}

/// Choose how long a phone stays signed in. Links already minted keep the
/// span they were minted with; this is the span the next one gets.
#[tauri::command]
pub async fn phone_set_days(app: AppHandle, days: i64) -> Result<i64, String> {
    if !auth::PHONE_DAYS.contains(&days) {
        return Err(format!("{days} is not one of the spans a phone can be signed in for"));
    }
    set(&app, "phone.days", &days.to_string())?;
    Ok(days)
}

/// Sign out every device at once: the panic button for a lost phone.
///
/// A generation bump, so it ends access tokens, refresh tokens and any
/// pairing link already shown but not yet scanned, in one write.
#[tauri::command]
pub async fn remote_server_drop_all(app: AppHandle) -> ServerStatus {
    {
        let st = app.state::<AppState>();
        st.remote.auth.drop_all(&st.store);
    }
    tracing::info!("every paired device was signed out");
    remote_server_status(app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tailscale::{Probe, ServeState};

    fn ready_probe() -> Probe {
        Probe {
            installed: true,
            reachable: true,
            logged_in: true,
            name: "laptop.tail123456.ts.net".into(),
            https: Some(true),
            peers: 2,
            peers_online: 1,
            ..Default::default()
        }
    }

    fn free() -> ServeState {
        ServeState { published: Some(false), port_free: Some(true), detail: String::new() }
    }

    #[test]
    fn the_steps_are_ordered_by_what_blocks_what() {
        let none = ServeState::default();
        assert_eq!(step_of(&Probe::default(), &none), Step::Install);
        assert_eq!(step_of(&Probe { installed: true, ..Default::default() }, &none), Step::StartTailscale);
        assert_eq!(
            step_of(&Probe { installed: true, reachable: true, stopped: true, logged_in: true, ..Default::default() }, &none),
            Step::StartTailscale,
            "a stopped daemon answers, but nothing can reach this host: same errand",
        );
        assert_eq!(step_of(&Probe { installed: true, reachable: true, ..Default::default() }, &none), Step::SignIn);
        assert_eq!(
            step_of(&Probe { installed: true, reachable: true, logged_in: true, ..Default::default() }, &none),
            Step::EnableMagicDns,
        );
        assert_eq!(step_of(&Probe { https: Some(false), ..ready_probe() }, &free()), Step::EnableHttps);
        assert_eq!(step_of(&ready_probe(), &free()), Step::Publish);
    }

    #[test]
    fn an_unknown_serve_state_never_offers_the_button() {
        // `publish` refuses both of these, so the card must not offer an action
        // the write side will decline.
        let unknown = ServeState::default();
        assert_eq!(step_of(&ready_probe(), &unknown), Step::Occupied);
        let strangers = ServeState { published: Some(false), port_free: Some(false), detail: String::new() };
        assert_eq!(step_of(&ready_probe(), &strangers), Step::Occupied);
    }

    #[test]
    fn only_a_mapping_read_as_someone_elses_offers_replacing_it() {
        let strangers = ServeState { published: Some(false), port_free: Some(false), detail: String::new() };
        assert!(replaceable(step_of(&ready_probe(), &strangers), &strangers));
        let unread = ServeState { published: None, port_free: Some(false), detail: String::new() };
        assert!(!replaceable(step_of(&ready_probe(), &unread), &unread));
        assert!(!replaceable(step_of(&ready_probe(), &ServeState::default()), &ServeState::default()));
        assert!(!replaceable(step_of(&ready_probe(), &free()), &free()), "a free port is a plain publish");
        // Tailscale itself not ready: the errand is that, not the mapping.
        assert!(!replaceable(step_of(&Probe { https: Some(false), ..ready_probe() }, &strangers), &strangers));
    }

    #[test]
    fn being_published_outranks_a_stale_certificate_list() {
        let ours = ServeState { published: Some(true), port_free: Some(false), detail: String::new() };
        assert_eq!(step_of(&ready_probe(), &ours), Step::Ready);
        assert_eq!(
            step_of(&Probe { https: Some(false), ..ready_probe() }, &ours),
            Step::Ready,
            "it is demonstrably working; do not take the address away over a snapshot",
        );
        assert_eq!(step_of(&Probe { https: None, ..ready_probe() }, &free()), Step::Publish, "unknown is not false");
    }
}
