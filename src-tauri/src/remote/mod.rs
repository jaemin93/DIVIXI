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
pub mod bridge;
pub mod client;
pub mod events;
pub mod github;
pub mod server;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::AppHandle;

use crate::AppState;

/// The port unless the `remote.port` setting says another.
pub const DEFAULT_PORT: u16 = 7488;

pub struct Remote {
    pub auth: auth::Auth,
    pub events: Arc<events::Hub>,
    running: tokio::sync::Mutex<Option<Running>>,
    /// Shells another PC opened here, by terminal id: the device's.
    pub terms: parking_lot::Mutex<std::collections::HashMap<u64, String>>,
    /// Event sockets open, by device: a device with none for a while is gone.
    pub sockets: parking_lot::Mutex<std::collections::HashMap<String, usize>>,
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
        })
    }
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    app.state::<AppState>().store.get_meta(&format!("{}{key}", crate::SETTING_PREFIX)).ok().flatten()
}

fn set(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
    app.state::<AppState>().store.set_meta(&format!("{}{key}", crate::SETTING_PREFIX), value).map_err(|e| e.to_string())
}

fn port(app: &AppHandle) -> u16 {
    setting(app, "remote.port").and_then(|p| p.trim().parse().ok()).filter(|p| *p >= 1024).unwrap_or(DEFAULT_PORT)
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

fn enabled(app: &AppHandle) -> bool {
    cfg!(feature = "server") || setting(app, "remote.enabled").as_deref() == Some("true")
}

/// At startup: the events hub listens, and the server comes up if this
/// Divixi serves (divixi-server always; the desktop app if it was on).
pub fn boot(app: &AppHandle) {
    let st = app.state::<AppState>();
    events::install(app, st.remote.events.clone());
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
        enabled: enabled(&app),
        running: running.is_some(),
        port: running.unwrap_or_else(|| port(&app)),
        all: listen_all(&app),
        addresses: addresses(),
        owner: github::owner(&st.store).unwrap_or_default(),
        devices: st.remote.auth.devices(&st.store),
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
    if enabled {
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
