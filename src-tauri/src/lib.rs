//! Orchestra desktop shell.
//!
//! The webview is a view. Everything that decides anything lives in Rust: this
//! module owns run identity, spawns lanes, and coalesces the event stream so
//! the IPC channel carries frames, not tokens.

use std::path::PathBuf;
use std::time::Duration;

use orchestra_acp::{run_lane, AgentSpec, LaneSpec};
use orchestra_core::{LaneEnvelope, LaneEvent};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// How often accumulated message text is flushed to the webview.
///
/// Agent chunks arrive far faster than a human reads. Emitting each one
/// individually floods IPC for no visible gain.
const FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Process-wide state. Small on purpose: Phase 0 keeps runs in the webview.
#[derive(Default)]
pub struct AppState {
    next_run: Mutex<u32>,
}

impl AppState {
    fn allocate_run(&self) -> String {
        let mut n = self.next_run.lock();
        *n += 1;
        format!("t{:03}", *n)
    }
}

/// Open a lane, run one prompt, and stream its events back as `lane` events.
///
/// Returns the run id immediately; the run proceeds in the background.
#[tauri::command]
async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    lane: String,
    prompt: String,
) -> Result<String, String> {
    let run = state.allocate_run();
    let cwd = workspace_root();

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

/// Forward lane events to the webview, coalescing message text.
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
                    let terminal = matches!(event, LaneEvent::Finished { .. } | LaneEvent::Failed { .. });
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

/// Entry point shared by the desktop binary.
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_run])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Orchestra");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Orchestra");
}
