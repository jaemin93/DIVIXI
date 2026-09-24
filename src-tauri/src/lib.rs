//! Divixi desktop shell.
//!
//! The webview is a view. Everything that decides anything lives in Rust: this
//! module owns run identity, knows which agents are installed, spawns sessions,
//! persists every session event to the store, and coalesces the event stream so
//! the IPC channel carries frames, not tokens.

use std::path::PathBuf;
use std::time::Duration;

use orchestra_acp::{AgentSpec, ConfigOptionInfo};
use orchestra_agents::{AgentKind, AgentStatus, DetectOptions, Readiness};
use orchestra_core::{AgentEnvelope, AgentEvent};
use orchestra_store::{ArtifactInfo, ArtifactPatch, Decision, RunSummary, SearchHit, Store, StoredEvent, TrackInfo, TrackPatch};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

mod conductor;
pub mod artifact;
pub mod design;
mod metrics;
mod terminal;
mod workspace;

/// How often accumulated message text is flushed to the webview and the store.
///
/// Agent chunks arrive far faster than a human reads. Emitting each one
/// individually floods IPC for no visible gain, and the store gets one row
/// per frame instead of one per token.
const FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Overrides where the event store lives. `:memory:` gives a throwaway store.
const DB_ENV: &str = "DIVIXI_DB";

/// Store key holding the last agent detection result (JSON).
const AGENTS_META: &str = "agents";

/// Process-wide state.
pub struct AppState {
    pub(crate) store: Store,
    /// Where the store lives, for the settings page.
    db_path: String,
    /// Where downloaded ACP servers live (`<app data>/adapters`).
    adapters_dir: PathBuf,
    /// Where pasted images are kept so they can be attached by path.
    attachments_dir: PathBuf,
    /// One working folder per artifact, for its agent.
    pub(crate) artifacts_dir: PathBuf,
    /// Artifacts' agent sessions.
    pub(crate) artifacts: artifact::Artifacts,
    /// Designs' boards, cached from the store.
    pub(crate) boards: design::Boards,
    /// Last detection result, mirrored from the store for quick lookups.
    agents: Mutex<Option<Vec<AgentStatus>>>,
    /// Conductor and session sessions, across tracks.
    pub(crate) sessions: conductor::Sessions,
    /// CPU, memory and disk readings for the title bar.
    meter: metrics::Meter,
    /// Shells in the bottom panel.
    terminals: terminal::Terminals,
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

    /// The session options an agent advertised at detection (mode, model,
    /// effort, …); empty when it was never probed.
    pub(crate) fn config_options_for(&self, agent: &str) -> Vec<ConfigOptionInfo> {
        let Some(kind) = AgentKind::parse(agent) else { return Vec::new() };
        self.load_agents()
            .ok()
            .flatten()
            .and_then(|list| list.into_iter().find(|s| s.kind == kind))
            .and_then(|s| s.probe)
            .map(|p| p.config_options)
            .unwrap_or_default()
    }

    /// The launch spec for an agent id, if it was detected as ready.
    pub(crate) fn spec_for(&self, agent: &str) -> Result<AgentSpec, String> {
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

/// Every track, oldest first.
#[tauri::command]
fn list_tracks(state: State<'_, AppState>) -> Result<Vec<TrackInfo>, String> {
    state.store.tracks().map_err(|e| e.to_string())
}

/// Check the fields a patch sets. `cwd` must be a directory, agents must
/// be known; an empty worker agent means "the conductor's".
fn check_patch(patch: &mut TrackPatch) -> Result<(), String> {
    if let Some(n) = patch.name.as_deref() {
        if n.trim().is_empty() {
            return Err("a track needs a name".to_string());
        }
    }
    if let Some(c) = patch.cwd.as_mut() {
        *c = c.trim().to_string();
        if !std::path::Path::new(c.as_str()).is_dir() {
            return Err(format!("not a directory: {c}"));
        }
    }
    if let Some(a) = patch.agent.as_deref() {
        AgentKind::parse(a).ok_or_else(|| format!("unknown agent {a}"))?;
    }
    if let Some(w) = patch.worker_agent.as_deref().filter(|w| !w.is_empty()) {
        AgentKind::parse(w).ok_or_else(|| format!("unknown agent {w}"))?;
    }
    if let Some(c) = patch.color.as_mut() {
        *c = c.trim().to_lowercase();
        let hex = c.strip_prefix('#').unwrap_or("");
        if !c.is_empty() && !(hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit())) {
            return Err(format!("a colour is #rrggbb or empty, not {c}"));
        }
    }
    if let Some(tags) = patch.tags.as_ref() {
        if tags.iter().any(|t| t.trim().chars().count() > 24) {
            return Err("a tag is at most 24 characters".to_string());
        }
    }
    Ok(())
}

/// Create a track. An empty working directory means the repository the app
/// was launched from; an empty agent means Claude Code.
#[tauri::command]
fn create_track(state: State<'_, AppState>, mut patch: TrackPatch) -> Result<TrackInfo, String> {
    if patch.cwd.as_deref().map(str::trim).unwrap_or("").is_empty() {
        patch.cwd = Some(workspace_root().display().to_string());
    }
    if patch.agent.as_deref().unwrap_or("").is_empty() {
        patch.agent = Some(AgentKind::ClaudeCode.id().to_string());
    }
    check_patch(&mut patch)?;
    state.store.create_track(&patch).map_err(|e| e.to_string())
}

/// Change a track: name, intent, folder, or the conductor's and sessions'
/// agent and session options. The conductor reopens with the new options
/// at its next message (keeping its memory); open sessions keep theirs until
/// closed. A new folder closes every session and forgets their memory,
/// since a session belongs to the directory it was opened in.
#[tauri::command]
async fn update_track(state: State<'_, AppState>, id: String, mut patch: TrackPatch) -> Result<TrackInfo, String> {
    check_patch(&mut patch)?;
    let before = state
        .store
        .track(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no track {id}"))?;
    let moved = patch.cwd.as_deref().is_some_and(|c| c != before.cwd);
    if moved {
        if state.sessions.is_active(&id).await {
            return Err("the track is still working; wait before changing the folder".to_string());
        }
        state.sessions.close_track(&id).await;
        state.store.forget_track_sessions(&id).map_err(|e| e.to_string())?;
    }
    state.store.update_track(&id, &patch).map_err(|e| e.to_string())
}

/// Delete a track: its sessions close, its runs and memory go.
#[tauri::command]
async fn delete_track(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if state.sessions.is_active(&id).await {
        return Err("the track is still working; wait for it to finish".to_string());
    }
    state.sessions.close_track(&id).await;
    state.store.delete_track(&id).map_err(|e| e.to_string())
}

/// Let the human pick a folder for a track. `None` when they cancel.
#[tauri::command]
async fn pick_folder(app: AppHandle, start: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = app.dialog().file();
    if let Some(dir) = start.filter(|s| !s.is_empty() && std::path::Path::new(s).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx.await.map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// Send one human message to a track's conductor. Returns the conductor run
/// id; the turn streams under it, and any sessions it opens stream under
/// theirs. `agent`, when given, becomes the track's conductor agent.
#[tauri::command]
async fn conductor_prompt(
    app: AppHandle,
    track: String,
    prompt: String,
    agent: Option<String>,
    lang: Option<String>,
    files: Option<Vec<String>>,
) -> Result<String, String> {
    let files = check_attachments(files.unwrap_or_default())?;
    conductor::conductor_turn(app, track, prompt, agent.filter(|a| !a.is_empty()), lang.unwrap_or_default(), files).await
}

// ----- artifacts: designs (and knowledge, later) -----

/// Artifacts of one kind, or of every kind, most recently touched first.
#[tauri::command(async)]
fn list_artifacts(state: State<'_, AppState>, kind: Option<String>) -> Result<Vec<ArtifactInfo>, String> {
    state.store.artifacts(kind.as_deref()).map_err(|e| e.to_string())
}

/// Start an empty artifact of a kind, on an agent when the kind has one.
#[tauri::command(async)]
fn create_artifact(state: State<'_, AppState>, kind: String, title: String, agent: Option<String>) -> Result<ArtifactInfo, String> {
    if !artifact::KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown artifact kind {kind}"));
    }
    let agent = agent.unwrap_or_default();
    if !agent.is_empty() {
        state.spec_for(&agent)?;
    }
    let title = title.trim();
    let title = if title.is_empty() { "Untitled" } else { title };
    let body = match kind.as_str() {
        design::KIND => serde_json::to_string(&design::Doc::default()).map_err(|e| e.to_string())?,
        _ => "{}".to_string(),
    };
    state.store.create_artifact(&kind, title, &agent, &body).map_err(|e| e.to_string())
}

/// Rename an artifact, move it to another agent, change its options (from
/// the next message), its colour or its tags.
#[tauri::command(async)]
fn update_artifact(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    agent: Option<String>,
    config: Option<std::collections::BTreeMap<String, String>>,
    color: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<ArtifactInfo, String> {
    if let Some(a) = &agent {
        state.spec_for(a)?;
    }
    // The same rules (and tidying) as a track's colour and tags: the tags
    // are one vocabulary.
    let mut look = TrackPatch { color, tags, ..Default::default() };
    check_patch(&mut look)?;
    let title = title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    state
        .store
        .update_artifact(&id, &ArtifactPatch { body: None, title, agent, config, color: look.color, tags: look.tags })
        .map_err(|e| e.to_string())
}

/// Delete an artifact with its body, conversation and agent session.
#[tauri::command]
async fn delete_artifact(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.artifacts.is_busy(&id) {
        return Err("its agent is still responding; wait for it to finish".to_string());
    }
    state.artifacts.close(&id).await;
    state.store.delete_artifact(&id).map_err(|e| e.to_string())?;
    // Forgotten only once the record is gone, so a read in between cannot
    // bring the board back into the cache.
    state.boards.forget(&id);
    let dir = state.artifacts_dir.join(&id);
    if dir.exists() {
        if let Err(err) = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir)).await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string())) {
            tracing::warn!(%err, artifact = %id, "could not remove the artifact's folder");
        }
    }
    Ok(())
}

/// Whether an artifact's agent session is open, and busy.
#[derive(serde::Serialize)]
struct ArtifactSession {
    open: bool,
    busy: bool,
}

#[tauri::command]
async fn artifact_state(app: AppHandle, id: String) -> Result<ArtifactSession, String> {
    let state = app.state::<AppState>();
    Ok(ArtifactSession { open: state.artifacts.is_open(&id).await, busy: state.artifacts.is_busy(&id) })
}

/// One message to an artifact's agent (for a design: a picture of the board
/// when it has ink, and the items picked on it).
#[tauri::command]
async fn artifact_prompt(
    app: AppHandle,
    id: String,
    text: String,
    image: Option<String>,
    selected: Option<Vec<String>>,
    lang: Option<String>,
    files: Option<Vec<String>>,
) -> Result<String, String> {
    let files = check_attachments(files.unwrap_or_default())?;
    artifact::turn(app, id, text, image, selected.unwrap_or_default(), lang.unwrap_or_default(), files).await
}

/// Stop an artifact agent's turn.
#[tauri::command]
async fn artifact_cancel(app: AppHandle, id: String) {
    artifact::cancel(&app, &id).await;
}

/// Write an artifact out as files to attach to a track's message.
#[tauri::command(async)]
fn export_artifact(state: State<'_, AppState>, id: String, markdown: String, image: Option<String>) -> Result<Vec<String>, String> {
    Ok(artifact::export(&state, &id, &markdown, image.as_deref())?
        .into_iter()
        .map(|p| p.display().to_string())
        .collect())
}

/// A design's board as it is now.
#[tauri::command(async)]
fn design_doc(state: State<'_, AppState>, id: String) -> Result<design::Doc, String> {
    design::doc(&state, &id)
}

/// The human edits a design's board.
#[tauri::command(async)]
fn design_apply(app: AppHandle, id: String, ops: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>, String> {
    let ops: Vec<design::Op> = ops
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("bad edit: {e}"))?;
    design::apply(&app, &id, ops, design::Actor::Human)
}

/// Keep or revert the design agent's suggestions.
#[tauri::command(async)]
fn design_review(app: AppHandle, id: String, changes: Vec<u64>, keep: bool) -> Result<(), String> {
    design::review(&app, &id, &changes, keep)
}

/// How many files one message may carry.
const MAX_ATTACHMENTS: usize = 20;

/// Attached paths, each an existing file, without repeats.
fn check_attachments(files: Vec<String>) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in files {
        let path = PathBuf::from(&f);
        if !path.is_absolute() || !path.is_file() {
            return Err(format!("not a file: {f}"));
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    if out.len() > MAX_ATTACHMENTS {
        return Err(format!("at most {MAX_ATTACHMENTS} files per message"));
    }
    Ok(out)
}

/// One attachable file as the composer shows it.
#[derive(Clone, serde::Serialize)]
struct FileStat {
    path: String,
    name: String,
    size: u64,
}

/// Sizes and names for paths about to be attached; folders and missing
/// paths are left out.
#[tauri::command(async)]
fn file_stats(paths: Vec<String>) -> Vec<FileStat> {
    paths
        .into_iter()
        .filter_map(|p| {
            let path = PathBuf::from(&p);
            let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
            Some(FileStat {
                name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone()),
                path: p,
                size: meta.len(),
            })
        })
        .collect()
}

/// Let the human pick files to attach, starting in the track's folder.
#[tauri::command]
async fn pick_files(app: AppHandle, start: Option<String>) -> Result<Vec<String>, String> {
    let mut dialog = app.dialog().file();
    if let Some(dir) = start.filter(|s| !s.is_empty() && std::path::Path::new(s).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_files(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx.await.map_err(|e| e.to_string())?;
    Ok(picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
        .collect())
}

/// Keep a pasted image (or other clipboard file) so it can be attached by
/// path. Returns where it went.
#[tauri::command(async)]
fn save_attachment(state: State<'_, AppState>, name: String, data: String) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| format!("bad attachment data: {e}"))?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err("attachments over 50 MB are not kept".to_string());
    }
    let clean: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let clean = if clean.trim_matches(['.', '_']).is_empty() { "pasted".to_string() } else { clean };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::create_dir_all(&state.attachments_dir).map_err(|e| e.to_string())?;
    let path = state.attachments_dir.join(format!("{stamp}-{clean}"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// The conductor's session state for a track: open, busy, its commands.
#[tauri::command]
async fn conductor_state(app: AppHandle, track: String) -> Result<conductor::ConductorState, String> {
    Ok(conductor::conductor_state(&app, &track).await)
}

/// Every open conductor session, by track.
#[tauri::command]
async fn conductor_states(app: AppHandle) -> Result<Vec<(String, conductor::ConductorState)>, String> {
    Ok(conductor::conductor_states(&app).await)
}

/// Open a track's conductor session without sending anything.
#[tauri::command]
async fn conductor_open(app: AppHandle, track: String) -> Result<conductor::ConductorState, String> {
    conductor::conductor_open(app, track).await
}

/// Stop the conductor's turn in flight, as Ctrl+C would.
#[tauri::command]
async fn conductor_cancel(app: AppHandle, track: String) -> Result<(), String> {
    conductor::conductor_cancel(app, track).await
}

/// Close a track's conductor session; its memory stays for the next open.
#[tauri::command]
async fn conductor_close(app: AppHandle, track: String) -> Result<conductor::ConductorState, String> {
    conductor::conductor_close(app, track).await
}

/// Decisions of one track, or of every track, oldest first.
#[tauri::command(async)]
fn list_decisions(state: State<'_, AppState>, track: Option<String>) -> Result<Vec<Decision>, String> {
    state.store.decisions(track.as_deref()).map_err(|e| e.to_string())
}

/// The human answers a decision card: an option by index, or their own
/// words. The answer goes to the conductor as a turn.
#[tauri::command]
async fn answer_decision(
    app: AppHandle,
    id: i64,
    choice: Option<usize>,
    answer: Option<String>,
    note: Option<String>,
) -> Result<Decision, String> {
    conductor::answer_decision(app, id, choice, answer, note.unwrap_or_default()).await
}

/// The human sets a decision aside; the conductor is not told.
#[tauri::command]
async fn dismiss_decision(app: AppHandle, id: i64) -> Result<Decision, String> {
    let state = app.state::<AppState>();
    let decision = state.store.dismiss_decision(id).map_err(|e| e.to_string())?;
    // A permission card set aside is a refusal to the agent that asked.
    conductor::refuse_permission(&app, &decision).await;
    let _ = app.emit("decision", &decision);
    Ok(decision)
}

/// How the machine is doing, measured on the disk of `track`'s folder.
#[tauri::command]
async fn system_metrics(state: State<'_, AppState>, track: Option<String>) -> Result<metrics::Metrics, String> {
    let folder = track
        .and_then(|t| state.store.track(&t).ok().flatten())
        .map(|t| PathBuf::from(t.cwd));
    let (sessions, working) = state.sessions.counts().await;
    let meter = &state.meter;
    // Disk enumeration touches the OS; keep it off the async workers.
    let mut reading = tokio::task::block_in_place(|| meter.read(folder.as_deref()));
    reading.sessions = sessions;
    reading.working = working;
    Ok(reading)
}

/// Start a shell in a track's folder (or the workspace root) for the
/// bottom panel. Output arrives as `term` events.
#[tauri::command(async)]
fn term_open(app: AppHandle, state: State<'_, AppState>, track: Option<String>, cols: u16, rows: u16) -> Result<u32, String> {
    let cwd = track
        .and_then(|t| state.store.track(&t).ok().flatten())
        .map(|t| PathBuf::from(t.cwd))
        .unwrap_or_else(workspace_root);
    state.terminals.open(app.clone(), Some(&cwd), cols, rows)
}

#[tauri::command]
fn term_write(state: State<'_, AppState>, id: u32, data: String) -> Result<(), String> {
    state.terminals.write(id, &data)
}

#[tauri::command]
fn term_resize(state: State<'_, AppState>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    state.terminals.resize(id, cols, rows)
}

#[tauri::command(async)]
fn term_close(state: State<'_, AppState>, id: u32) {
    state.terminals.close(id);
}

/// Every run, oldest first: what the timeline is rebuilt from at startup.
#[tauri::command(async)]
fn list_runs(state: State<'_, AppState>) -> Result<Vec<RunSummary>, String> {
    state.store.runs().map_err(|e| e.to_string())
}

/// One run's full event log, for the inspector to replay.
#[tauri::command(async)]
fn run_events(state: State<'_, AppState>, run: String) -> Result<Vec<StoredEvent>, String> {
    state.store.events(&run).map_err(|e| e.to_string())
}

/// Full-text search over finished runs.
#[tauri::command(async)]
fn search_runs(state: State<'_, AppState>, query: String) -> Result<Vec<SearchHit>, String> {
    state.store.search(&query).map_err(|e| e.to_string())
}

/// The last detection result, or `None` if setup never ran.
#[tauri::command]
fn agent_statuses(state: State<'_, AppState>) -> Result<Option<Vec<AgentStatus>>, String> {
    state.load_agents()
}

/// Facts about this install, for the settings overview.
#[derive(Clone, serde::Serialize)]
struct AppInfo {
    version: String,
    db_path: String,
    adapters_dir: String,
    workspace: String,
    runs: usize,
}

#[tauri::command(async)]
fn app_info(state: State<'_, AppState>) -> Result<AppInfo, String> {
    let runs = state.store.runs().map_err(|e| e.to_string())?.len();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        db_path: state.db_path.clone(),
        adapters_dir: state.adapters_dir.display().to_string(),
        workspace: workspace_root().display().to_string(),
        runs,
    })
}

// ----- the track's working folder, for the side panel -----

/// A track's folder, checked to exist.
fn track_root(state: &AppState, track: &str) -> Result<PathBuf, String> {
    let info = state
        .store
        .track(track)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no track {track}"))?;
    let root = PathBuf::from(&info.cwd);
    if !root.is_dir() {
        return Err(format!("the track's folder is gone: {}", info.cwd));
    }
    Ok(root)
}

/// Every file and folder under the track's folder, `.gitignore` honoured.
#[tauri::command(async)]
fn workspace_tree(state: State<'_, AppState>, track: String) -> Result<Vec<workspace::Entry>, String> {
    workspace::tree(&track_root(&state, &track)?)
}

/// One file's contents for preview. `path` is relative to the folder.
#[tauri::command(async)]
fn workspace_read(state: State<'_, AppState>, track: String, path: String) -> Result<workspace::FileContent, String> {
    workspace::read(&track_root(&state, &track)?, &path)
}

/// What git says has changed in the track's folder.
#[tauri::command(async)]
fn workspace_git_status(state: State<'_, AppState>, track: String) -> Result<workspace::GitStatus, String> {
    workspace::status(&track_root(&state, &track)?)
}

/// One path's diff against HEAD (all added when untracked).
#[tauri::command(async)]
fn workspace_git_diff(state: State<'_, AppState>, track: String, path: String, untracked: bool) -> Result<String, String> {
    workspace::diff(&track_root(&state, &track)?, &path, untracked)
}

/// Select a file in the system file manager.
#[tauri::command(async)]
fn workspace_reveal(state: State<'_, AppState>, track: String, path: String) -> Result<(), String> {
    workspace::reveal(&track_root(&state, &track)?, &path)
}

/// Namespace for user preferences in the store's `meta` table.
const SETTING_PREFIX: &str = "setting:";

/// Read a user preference (`theme`, …).
#[tauri::command]
fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    state
        .store
        .get_meta(&format!("{SETTING_PREFIX}{key}"))
        .map_err(|e| e.to_string())
}

/// Write a user preference.
#[tauri::command]
fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    state
        .store
        .set_meta(&format!("{SETTING_PREFIX}{key}"), &value)
        .map_err(|e| e.to_string())
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

/// Persist session events and forward them to the webview, coalescing message text.
///
/// The store write happens before the emit, so anything the webview has seen
/// is already durable. A store failure is logged and the event still reaches
/// the webview: losing history is bad, losing the live view is worse.
pub(crate) async fn pump(
    app: AppHandle,
    track: String,
    session: String,
    run: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
) {
    let started = std::time::Instant::now();
    let mut pending = String::new();
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // A run whose row is gone (its track was deleted mid-turn) fails on
    // every frame; say so once.
    let mut persist_failed = false;
    let mut emit = |event: AgentEvent, at_ms: u64| {
        if let Err(err) = app.state::<AppState>().store.append(&run, at_ms, &event) {
            if !persist_failed {
                tracing::error!(run = %run, kind = event.kind(), %err, "failed to persist agent event");
            }
            persist_failed = true;
        }
        let _ = app.emit(
            "agent",
            AgentEnvelope {
                track: track.clone(),
                session: session.clone(),
                run: run.clone(),
                at_ms,
                event,
            },
        );
    };

    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Some(AgentEvent::Message { text }) => pending.push_str(&text),
                Some(event) => {
                    let ms = started.elapsed().as_millis() as u64;
                    if !pending.is_empty() {
                        emit(AgentEvent::Message { text: std::mem::take(&mut pending) }, ms);
                    }
                    let terminal = event.is_terminal();
                    // An agent asking before it acts: to whoever answers for it.
                    if matches!(event, AgentEvent::Permission { .. }) {
                        conductor::route_permission(&app, &track, &session, &run, &event);
                    }
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
                    emit(AgentEvent::Message { text: std::mem::take(&mut pending) }, ms);
                }
            }
        }
    }

    if !pending.is_empty() {
        emit(
            AgentEvent::Message { text: pending },
            started.elapsed().as_millis() as u64,
        );
    }
}

/// Default working directory for a new track: the repository the app was
/// launched from.
///
/// `tauri dev` starts the binary inside `src-tauri/`, so the plain working
/// directory would point agents at the wrong folder. Walk up to the nearest
/// `.git`; without one, the working directory itself.
pub(crate) fn workspace_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(PathBuf::from)
        .unwrap_or(cwd)
}

/// Open the event store: `DIVIXI_DB` if set, else `divixi.db` in the
/// platform app-data directory.
fn open_store(data_dir: &std::path::Path) -> anyhow::Result<(Store, String)> {
    if let Some(explicit) = std::env::var_os(DB_ENV) {
        if explicit == ":memory:" {
            tracing::warn!("{DB_ENV}=:memory: — nothing will persist");
            return Ok((Store::in_memory()?, ":memory:".to_string()));
        }
        let path = PathBuf::from(explicit);
        tracing::info!(path = %path.display(), "opening event store");
        return Ok((Store::open(&path)?, path.display().to_string()));
    }

    let path = data_dir.join("divixi.db");
    // The store was orchestra.db before the app was named Divixi.
    let old_db = data_dir.join("orchestra.db");
    if !path.exists() && old_db.exists() {
        match std::fs::rename(&old_db, &path) {
            Ok(()) => {
                // The journal files only make sense next to their database.
                for suffix in ["-wal", "-shm"] {
                    let old = data_dir.join(format!("orchestra.db{suffix}"));
                    if old.exists() {
                        if let Err(err) = std::fs::rename(&old, data_dir.join(format!("divixi.db{suffix}"))) {
                            tracing::warn!(%err, suffix, "could not move the old database journal");
                        }
                    }
                }
            }
            Err(err) => tracing::warn!(%err, "could not migrate orchestra.db; starting empty"),
        }
    }
    tracing::info!(path = %path.display(), "opening event store");
    Ok((Store::open(&path)?, path.display().to_string()))
}

/// Entry point shared by the desktop binary.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            list_tracks,
            create_track,
            update_track,
            delete_track,
            pick_folder,
            conductor_prompt,
            conductor_state,
            conductor_states,
            conductor_open,
            conductor_close,
            conductor_cancel,
            list_runs,
            run_events,
            list_decisions,
            list_artifacts,
            create_artifact,
            update_artifact,
            delete_artifact,
            artifact_state,
            artifact_prompt,
            artifact_cancel,
            export_artifact,
            design_doc,
            design_apply,
            design_review,
            file_stats,
            pick_files,
            save_attachment,
            system_metrics,
            term_open,
            term_write,
            term_resize,
            term_close,
            answer_decision,
            dismiss_decision,
            search_runs,
            agent_statuses,
            detect_agents,
            login_agent,
            download_agent,
            get_setting,
            set_setting,
            app_info,
            workspace_tree,
            workspace_read,
            workspace_git_status,
            workspace_git_diff,
            workspace_reveal,
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            // The app was Orchestra before it was Divixi; its data folder moves along.
            if !data_dir.exists() {
                if let Some(old) = data_dir.parent().map(|p| p.join("app.orchestra")) {
                    if old.is_dir() {
                        match std::fs::rename(&old, &data_dir) {
                            Ok(()) => tracing::info!(from = %old.display(), to = %data_dir.display(), "moved the data folder"),
                            Err(err) => tracing::warn!(%err, "could not move the old data folder; starting empty"),
                        }
                    }
                }
            }
            std::fs::create_dir_all(&data_dir)?;
            let (store, db_path) = open_store(&data_dir)?;
            let adapters_dir = data_dir.join("adapters");
            let attachments_dir = data_dir.join("attachments");
            let artifacts_dir = data_dir.join("artifacts");
            // Drafts became artifacts: their agents' folders move along
            // (dr001 → ar001), as the store moved their records.
            let old = data_dir.join("drafts");
            if old.is_dir() && !artifacts_dir.exists() {
                if let Err(err) = std::fs::rename(&old, &artifacts_dir) {
                    tracing::warn!(%err, "could not move the drafts folder");
                } else if let Ok(entries) = std::fs::read_dir(&artifacts_dir) {
                    for e in entries.flatten() {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if let Some(n) = name.strip_prefix("dr") {
                            let _ = std::fs::rename(e.path(), artifacts_dir.join(format!("ar{n}")));
                        }
                    }
                }
            }
            app.manage(AppState {
                store,
                db_path,
                adapters_dir,
                attachments_dir,
                artifacts_dir,
                artifacts: artifact::Artifacts::default(),
                boards: design::Boards::default(),
                agents: Mutex::new(None),
                sessions: conductor::Sessions::default(),
                meter: metrics::Meter::default(),
                terminals: terminal::Terminals::default(),
            });
            // Artifact agents nobody has talked to for an hour are closed.
            artifact::sweep_idle(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Divixi");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Divixi");
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    #[test]
    fn a_tags_only_patch_from_json_updates_the_track() {
        let store = Store::in_memory().unwrap();
        let created = store
            .create_track(&TrackPatch { name: Some("t".into()), cwd: Some(".".into()), agent: Some("claude_code".into()), ..Default::default() })
            .unwrap();
        let mut patch: TrackPatch = serde_json::from_str(r#"{"tags":["rust","study"]}"#).unwrap();
        check_patch(&mut patch).unwrap();
        let after = store.update_track(&created.id, &patch).unwrap();
        assert_eq!(after.tags, vec!["rust", "study"]);
        let mut patch: TrackPatch = serde_json::from_str(r##"{"color":"#7aa2f7"}"##).unwrap();
        check_patch(&mut patch).unwrap();
        let after = store.update_track(&created.id, &patch).unwrap();
        assert_eq!((after.color.as_str(), after.tags.len()), ("#7aa2f7", 2));
    }
}
