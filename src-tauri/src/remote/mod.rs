//! Remote instances. See docs/design/remote-access.md.
//!
//! divixi-server runs an axum server on 127.0.0.1 that serves the app's own
//! UI and carries its commands (through [`bridge`], a listed few) and events
//! (through [`events`]) over HTTP and a WebSocket. Who may come in is
//! [`auth`]'s: windows paired with a one-time link. The desktop app reaches
//! it over an SSH tunnel ([`client`]).

pub mod auth;
pub mod bridge;
pub mod client;
pub mod events;
pub mod server;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
    *running = Some(Running { port: want, stop });
    tracing::info!(port = want, "remote instance server on");
    Ok(want)
}

/// At startup: the events hub listens and, in divixi-server, the server
/// comes up. The desktop app serves nothing: it only opens remote
/// instances (see [`client`]).
pub fn boot(app: &AppHandle) {
    let st = app.state::<AppState>();
    events::install(app, st.remote.events.clone());
    if cfg!(feature = "server") {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = start(&app).await {
                tracing::warn!(%err, "could not start the remote instance server");
            }
        });
    }
}
