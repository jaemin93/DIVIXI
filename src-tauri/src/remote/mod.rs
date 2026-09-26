//! Reaching Divixi from a phone. See docs/design/remote-access.md.
//!
//! An axum server on 127.0.0.1 serves the app's own UI to a browser and
//! carries its commands (through [`bridge`], a listed few) and events
//! (through [`events`]) over HTTP and a WebSocket. Who may come in is
//! [`auth`]'s: devices paired with a one-time QR link. What is in front of
//! it (tailscale serve) is set up in a later step; on its own it answers
//! the PC only.

pub mod auth;
pub mod bridge;
pub mod events;
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
    /// The PC's name on the tailnet, once serve publishes it (`host:port` form as a browser sends it).
    public: parking_lot::RwLock<Option<String>>,
}

struct Running {
    port: u16,
    stop: tokio::sync::oneshot::Sender<()>,
}

/// What the settings page shows.
#[derive(Serialize)]
pub struct Status {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    /// The address on this PC.
    pub local_url: String,
    pub devices: Vec<auth::Device>,
}

impl Remote {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            auth: auth::Auth::open(data_dir)?,
            events: Arc::new(events::Hub::default()),
            running: tokio::sync::Mutex::new(None),
            public: parking_lot::RwLock::new(None),
        })
    }

    pub fn public_host(&self) -> Option<String> {
        self.public.read().clone()
    }
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    app.state::<AppState>().store.get_meta(&format!("{}{key}", crate::SETTING_PREFIX)).ok().flatten()
}

fn port(app: &AppHandle) -> u16 {
    setting(app, "remote.port").and_then(|p| p.trim().parse().ok()).filter(|p| *p >= 1024).unwrap_or(DEFAULT_PORT)
}

/// Start the server (again, on the set port), or leave it running.
pub async fn start(app: &AppHandle) -> Result<u16, String> {
    let st = app.state::<AppState>();
    let want = port(app);
    let mut running = st.remote.running.lock().await;
    if let Some(r) = running.as_ref() {
        if r.port == want {
            return Ok(want);
        }
    }
    if let Some(old) = running.take() {
        let _ = old.stop.send(());
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", want)).await.map_err(|e| format!("port {want}: {e}"))?;
    let router = server::router(server::Ctx { app: app.clone(), port: want });
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
        let serve = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            });
        if let Err(err) = serve.await {
            tracing::warn!(%err, "remote access server stopped");
        }
    });
    st.remote.events.on.store(true, Ordering::Relaxed);
    *running = Some(Running { port: want, stop });
    tracing::info!(port = want, "remote access on");
    Ok(want)
}

pub async fn stop(app: &AppHandle) {
    let st = app.state::<AppState>();
    if let Some(r) = st.remote.running.lock().await.take() {
        let _ = r.stop.send(());
        tracing::info!("remote access off");
    }
    st.remote.events.on.store(false, Ordering::Relaxed);
}

/// At startup: the events hub listens, and the server comes up if it was on.
pub fn boot(app: &AppHandle) {
    let st = app.state::<AppState>();
    events::install(app, st.remote.events.clone());
    if cfg!(feature = "server") || setting(app, "remote.enabled").as_deref() == Some("true") {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = start(&app).await {
                tracing::warn!(%err, "could not start remote access");
            }
        });
    }
}

// ----- commands for the PC's settings page (never reachable remotely) -----

#[tauri::command]
pub async fn remote_status(app: AppHandle) -> Status {
    let st = app.state::<AppState>();
    let running = st.remote.running.lock().await.as_ref().map(|r| r.port);
    let port = running.unwrap_or_else(|| port(&app));
    Status {
        enabled: setting(&app, "remote.enabled").as_deref() == Some("true"),
        running: running.is_some(),
        port,
        local_url: format!("http://127.0.0.1:{port}"),
        devices: st.remote.auth.devices(&st.store),
    }
}

#[tauri::command]
pub async fn remote_set_enabled(app: AppHandle, enabled: bool) -> Result<Status, String> {
    let st = app.state::<AppState>();
    st.store.set_meta(&format!("{}remote.enabled", crate::SETTING_PREFIX), if enabled { "true" } else { "false" }).map_err(|e| e.to_string())?;
    if enabled {
        start(&app).await?;
    } else {
        stop(&app).await;
    }
    Ok(remote_status(app).await)
}

/// A pairing link (and its QR, as SVG) good for five minutes.
#[derive(Serialize)]
pub struct Pairing {
    pub url: String,
    pub expires: i64,
    pub qr_svg: String,
}

#[tauri::command]
pub async fn remote_pair(app: AppHandle) -> Result<Pairing, String> {
    let st = app.state::<AppState>();
    let port = st.remote.running.lock().await.as_ref().map(|r| r.port).ok_or("remote access is off")?;
    let (token, expires) = st.remote.auth.pair_token(&st.store);
    let base = match st.remote.public_host() {
        Some(host) => format!("https://{host}"),
        None => format!("http://127.0.0.1:{port}"),
    };
    let url = format!("{base}/auth/pair?token={token}");
    let qr = qrcode::QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    let qr_svg = qr.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).quiet_zone(true).build();
    Ok(Pairing { url, expires, qr_svg })
}

/// Drop one paired device, or all of them.
#[tauri::command]
pub async fn remote_drop(app: AppHandle, device: Option<String>) -> Status {
    {
        let st = app.state::<AppState>();
        match device {
            Some(id) => st.remote.auth.drop_device(&st.store, &id),
            None => st.remote.auth.drop_all(&st.store),
        }
    }
    remote_status(app).await
}
