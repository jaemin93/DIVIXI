//! Orchestra desktop shell.
//!
//! The webview is a view. Everything that decides anything lives in Rust: this
//! module owns run identity, knows which agents are installed, spawns lanes,
//! persists every lane event to the store, and coalesces the event stream so
//! the IPC channel carries frames, not tokens.

use std::path::PathBuf;
use std::time::Duration;

use orchestra_acp::{run_lane, AgentSpec, LaneSpec};
use orchestra_agents::{AgentKind, AgentStatus, DetectOptions, Readiness};
use orchestra_core::{LaneEnvelope, LaneEvent};
use orchestra_store::{RunSummary, SearchHit, Store, StoredEvent};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// How often accumulated message text is flushed to the webview and the store.
///
/// Agent chunks arrive far faster than a human reads. Emitting each one
/// individually floods IPC for no visible gain, and the store gets one row
/// per frame instead of one per token.
const FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Overrides where the event store lives. `:memory:` gives a throwaway store.
const DB_ENV: &str = "ORCHESTRA_DB";

/// Store key holding the last agent detection result (JSON).
const AGENTS_META: &str = "agents";

/// Process-wide state.
pub struct AppState {
    store: Store,
    /// Where downloaded ACP servers live (`<app data>/adapters`).
    adapters_dir: PathBuf,
    /// Last detection result, mirrored from the store for quick lookups.
    agents: Mutex<Option<Vec<AgentStatus>>>,
}

impl AppState {
    fn detect_options(&self) -> DetectOptions {
        DetectOptions::new(&self.adapters_dir, workspace_root())
    }

    fn save_agents(&self, statuses: Vec<AgentStatus>) -> Result<Vec<AgentStatus>, String> {
        let json = serde_json::to_string(&statuses).map_err(|e| e.to_string())?;
        self.store.set_meta(AGENTS_META, &json).map_err(|e| e.to_string())?;
        *self.agents.lock() = Some(statuses.clone());
        Ok(statuses)
    }

    fn load_agents(&self) -> Result<Option<Vec<AgentStatus>>, String> {
        if let Some(cached) = self.agents.lock().clone() {
            return Ok(Some(cached));
        }
        let Some(json) = self.store.get_meta(AGENTS_META).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        // A stale or incompatible record just means "detect again".
        let parsed: Option<Vec<AgentStatus>> = serde_json::from_str(&json).ok();
        *self.agents.lock() = parsed.clone();
        Ok(parsed)
    }

    /// Replace one agent's status in the saved list.
    fn update_agent(&self, status: AgentStatus) -> Result<AgentStatus, String> {
        let mut list = self.load_agents()?.unwrap_or_default();
        match list.iter_mut().find(|s| s.kind == status.kind) {
            Some(slot) => *slot = status.clone(),
            None => list.push(status.clone()),
        }
        self.save_agents(list)?;
        Ok(status)
    }

    /// The launch spec for an agent id, if it was detected as ready.
    fn spec_for(&self, agent: &str) -> Result<AgentSpec, String> {
        let kind = AgentKind::parse(agent).ok_or_else(|| format!("unknown agent {agent}"))?;
        let list = self.load_agents()?.unwrap_or_default();
        match list.iter().find(|s| s.kind == kind) {
            Some(s) if s.readiness == Readiness::Ready => s
                .spec
                .clone()
                .ok_or_else(|| format!("{} has no launch spec", kind.name())),
            Some(s) => Err(format!("{} is not ready ({:?})", kind.name(), s.readiness)),
            // Never detected: Claude Code still works through the bundled adapter.
            None if kind == AgentKind::ClaudeCode => Ok(AgentSpec::claude_code()),
            None => Err(format!("{} has not been detected; run setup", kind.name())),
        }
    }
}

/// Open a lane on `agent`, run one prompt, and stream its events back as
/// `lane` events.
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
    agent: Option<String>,
) -> Result<String, String> {
    let agent = agent.unwrap_or_else(|| AgentKind::ClaudeCode.id().to_string());
    let agent_spec = state.spec_for(&agent)?;
    let cwd = workspace_root();
    let run = state
        .store
        .begin_run(&lane, &agent, &prompt, &cwd.display().to_string())
        .map_err(|e| e.to_string())?;

    let spec = LaneSpec {
        agent: agent_spec,
        cwd,
        prompt,
        // The most autonomous mode the agent offers is chosen per session.
        mode: None,
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

/// The last detection result, or `None` if setup never ran.
#[tauri::command]
fn agent_statuses(state: State<'_, AppState>) -> Result<Option<Vec<AgentStatus>>, String> {
    state.load_agents()
}

/// Detect every agent now and remember the result. Takes several seconds:
/// each agent is launched and probed over ACP.
#[tauri::command]
async fn detect_agents(state: State<'_, AppState>) -> Result<Vec<AgentStatus>, String> {
    let opts = state.detect_options();
    let statuses = orchestra_agents::detect_all(&opts).await;
    state.save_agents(statuses)
}

/// Run an agent's ACP login flow and re-probe it. May open a browser.
#[tauri::command]
async fn login_agent(
    state: State<'_, AppState>,
    agent: String,
    method: Option<String>,
) -> Result<AgentStatus, String> {
    let kind = AgentKind::parse(&agent).ok_or_else(|| format!("unknown agent {agent}"))?;
    let current = state
        .load_agents()?
        .and_then(|l| l.into_iter().find(|s| s.kind == kind))
        .ok_or_else(|| format!("{} has not been detected yet", kind.name()))?;
    let opts = state.detect_options();
    let next = orchestra_agents::login(&current, method.as_deref(), &opts).await;
    state.update_agent(next)
}

/// Progress of an adapter download, emitted as `agent_download` events.
#[derive(Clone, serde::Serialize)]
struct DownloadEvent {
    agent: String,
    #[serde(flatten)]
    progress: orchestra_agents::DownloadProgress,
}

/// Download an agent's ACP server (Antigravity only, for now) and re-detect it.
///
/// Progress goes out as `agent_download` events so the webview can draw a bar.
#[tauri::command]
async fn download_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    agent: String,
) -> Result<AgentStatus, String> {
    let kind = AgentKind::parse(&agent).ok_or_else(|| format!("unknown agent {agent}"))?;
    if kind != AgentKind::Antigravity {
        return Err(format!("{} needs no download", kind.name()));
    }
    let id = kind.id().to_string();
    orchestra_agents::download_antigravity_with(&state.adapters_dir, move |progress| {
        let _ = app.emit("agent_download", DownloadEvent { agent: id.clone(), progress });
    })
    .await
    .map_err(|e| e.to_string())?;
    let opts = state.detect_options();
    let next = orchestra_agents::detect(kind, &opts).await;
    state.update_agent(next)
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
fn open_store(data_dir: &std::path::Path) -> anyhow::Result<Store> {
    if let Some(explicit) = std::env::var_os(DB_ENV) {
        if explicit == ":memory:" {
            tracing::warn!("{DB_ENV}=:memory: — nothing will persist");
            return Store::in_memory();
        }
        let path = PathBuf::from(explicit);
        tracing::info!(path = %path.display(), "opening event store");
        return Store::open(path);
    }

    let path = data_dir.join("orchestra.db");
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
            search_runs,
            agent_statuses,
            detect_agents,
            login_agent,
            download_agent,
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = open_store(&data_dir)?;
            let adapters_dir = data_dir.join("adapters");
            app.manage(AppState {
                store,
                adapters_dir,
                agents: Mutex::new(None),
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Orchestra");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Orchestra");
}
