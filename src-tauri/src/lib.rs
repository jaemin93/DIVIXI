//! Orchestra desktop shell.
//!
//! The webview is a view. Everything that decides anything lives in Rust: this
//! module owns run identity, spawns lanes, persists every lane event to the
//! store, and coalesces the event stream so the IPC channel carries frames,
//! not tokens.

use std::path::PathBuf;
use std::time::Duration;

use orchestra_acp::{run_lane, AgentSpec, LaneSpec};
use orchestra_core::{LaneEnvelope, LaneEvent};
use orchestra_store::{RunSummary, SearchHit, Store, StoredEvent};
use tauri::{AppHandle, Emitter, Manager, State};

/// How often accumulated message text is flushed to the webview and the store.
///
/// Agent chunks arrive far faster than a human reads. Emitting each one
/// individually floods IPC for no visible gain, and the store gets one row
/// per frame instead of one per token.
const FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Overrides where the event store lives. `:memory:` gives a throwaway store.
const DB_ENV: &str = "ORCHESTRA_DB";

/// Process-wide state.
pub struct AppState {
    store: Store,
}

/// Open a lane, run one prompt, and stream its events back as `lane` events.
///
/// Returns the run id immediately; the run proceeds in the background. The
/// run is registered in the store before this returns, so a `list_runs`
/// issued right after already includes it.
#[tauri::command]
async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    lane: String,
    prompt: String,
) -> Result<String, String> {
    let cwd = workspace_root();
    let run = state
        .store
        .begin_run(&lane, &prompt, &cwd.display().to_string())
        .map_err(|e| e.to_string())?;

    let spec = LaneSpec {
        agent: AgentSpec::claude_code(),
        cwd,
        prompt,
        mode: Some("bypassPermissions".to_string()),
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let lane_for_task = lane.clone();
    let run_for_task = run.clone();
    let app_for_pump = app.clone();

    tauri::async_runtime::spawn(async move {
        pump(app_for_pump, lane_for_task, run_for_task, rx).await;
    });

    let lane_for_run = lane.clone();
    let run_for_run = run.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_lane(spec, tx.clone()).await {
            // The lane channel may already be closed; emit through it anyway so
            // the failure lands in the same ordered stream as everything else.
            let _ = tx.send(LaneEvent::Failed {
                error: err.to_string(),
            });
            tracing::error!(lane = %lane_for_run, run = %run_for_run, %err, "lane failed");
        }
    });

    Ok(run)
}

/// Every run, oldest first: what the timeline is rebuilt from at startup.
#[tauri::command]
fn list_runs(state: State<'_, AppState>) -> Result<Vec<RunSummary>, String> {
    state.store.runs().map_err(|e| e.to_string())
}

/// One run's full event log, for the inspector to replay.
#[tauri::command]
fn run_events(state: State<'_, AppState>, run: String) -> Result<Vec<StoredEvent>, String> {
    state.store.events(&run).map_err(|e| e.to_string())
}

/// Full-text search over finished runs.
#[tauri::command]
fn search_runs(state: State<'_, AppState>, query: String) -> Result<Vec<SearchHit>, String> {
    state.store.search(&query).map_err(|e| e.to_string())
}

/// Persist lane events and forward them to the webview, coalescing message text.
///
/// The store write happens before the emit, so anything the webview has seen
/// is already durable. A store failure is logged and the event still reaches
/// the webview: losing history is bad, losing the live view is worse.
async fn pump(
    app: AppHandle,
    lane: String,
    run: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<LaneEvent>,
) {
    let started = std::time::Instant::now();
    let mut pending = String::new();
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let emit = |event: LaneEvent, at_ms: u64| {
        if let Err(err) = app.state::<AppState>().store.append(&run, at_ms, &event) {
            tracing::error!(run = %run, kind = event.kind(), %err, "failed to persist lane event");
        }
        let _ = app.emit(
            "lane",
            LaneEnvelope {
                lane: lane.clone(),
                run: run.clone(),
                at_ms,
                event,
            },
        );
    };

    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Some(LaneEvent::Message { text }) => pending.push_str(&text),
                Some(event) => {
                    let ms = started.elapsed().as_millis() as u64;
                    if !pending.is_empty() {
                        emit(LaneEvent::Message { text: std::mem::take(&mut pending) }, ms);
                    }
                    let terminal = event.is_terminal();
                    emit(event, ms);
                    if terminal {
                        break;
                    }
                }
                None => break,
            },
            _ = ticker.tick() => {
                if !pending.is_empty() {
                    let ms = started.elapsed().as_millis() as u64;
                    emit(LaneEvent::Message { text: std::mem::take(&mut pending) }, ms);
                }
            }
        }
    }

    if !pending.is_empty() {
        emit(
            LaneEvent::Message { text: pending },
            started.elapsed().as_millis() as u64,
        );
    }
}

/// Directory handed to lanes. Phase 0 uses the repo the app was launched from.
fn workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Open the event store: `ORCHESTRA_DB` if set, else `orchestra.db` in the
/// platform app-data directory.
fn open_store(app: &AppHandle) -> anyhow::Result<Store> {
    if let Some(explicit) = std::env::var_os(DB_ENV) {
        if explicit == ":memory:" {
            tracing::warn!("{DB_ENV}=:memory: — nothing will persist");
            return Store::in_memory();
        }
        let path = PathBuf::from(explicit);
        tracing::info!(path = %path.display(), "opening event store");
        return Store::open(path);
    }

    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("orchestra.db");
    tracing::info!(path = %path.display(), "opening event store");
    Store::open(path)
}

/// Entry point shared by the desktop binary.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            start_run,
            list_runs,
            run_events,
            search_runs
        ])
        .setup(|app| {
            let store = open_store(app.handle())?;
            app.manage(AppState { store });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Orchestra");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Orchestra");
}
