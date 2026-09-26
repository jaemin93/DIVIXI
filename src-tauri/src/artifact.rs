//! Artifacts: what the human keeps beside tracks and attaches to them — a
//! design today, knowledge next. Every kind shares a list (name, colour,
//! tags, times), a conversation with an agent when the kind has one, and a
//! body the kind's own module owns (`design` for sketch boards).
//!
//! The conversation runs like a track conductor's: one long-lived agent
//! session per artifact with an MCP server of the kind's tools; its turns
//! are kept under the track key `artifact:<id>`, session `artifact`, so they
//! never mix with a track's.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use orchestra_acp::AgentSession;
use orchestra_core::AgentEvent;
use orchestra_mcp::McpServer;
use orchestra_store::ArtifactInfo;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::conductor::{fingerprint, session_options, with_attachments, Live};
use crate::{design, pump, AppState};

/// The session an artifact's agent turns are recorded under.
pub const SESSION: &str = "artifact";

/// The kinds there are.
pub const KINDS: [&str; 2] = [design::KIND, crate::knowledge::KIND];

/// Runs of an artifact are kept under this track key, apart from tracks.
pub fn run_key(id: &str) -> String {
    format!("artifact:{id}")
}

/// A session nobody has used for this long is closed; its conversation is
/// remembered, so the next message reopens it with that memory.
pub const IDLE_CLOSE: Duration = Duration::from_secs(60 * 60);
/// How often idle sessions are looked for.
const IDLE_SWEEP: Duration = Duration::from_secs(5 * 60);

/// An artifact's agent session and the MCP server that is its hands.
struct Agent {
    live: Live,
    /// Agent and options it was opened with; a change reopens it.
    fingerprint: String,
    /// When a turn last started or ended on it.
    used: Instant,
    _mcp: McpServer,
}

/// Every artifact's agent session, and which have a turn in flight.
#[derive(Default)]
pub struct Artifacts {
    sessions: Mutex<HashMap<String, Agent>>,
    busy: parking_lot::Mutex<HashSet<String>>,
}

impl Artifacts {
    pub fn is_busy(&self, id: &str) -> bool {
        self.busy.lock().contains(id)
    }

    pub async fn is_open(&self, id: &str) -> bool {
        self.sessions.lock().await.contains_key(id)
    }

    /// Close an artifact's session (it is being deleted).
    pub async fn close(&self, id: &str) {
        if let Some(a) = self.sessions.lock().await.remove(id) {
            a.live.session.cancel();
        }
    }

    /// Close sessions idle for `idle` or longer, other than those with a
    /// turn in flight. Returns the artifacts whose session was closed.
    pub async fn close_idle(&self, idle: Duration) -> Vec<String> {
        let mut sessions = self.sessions.lock().await;
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(id, a)| a.live.running.is_none() && !self.is_busy(id) && a.used.elapsed() >= idle)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            // Dropping the session ends the agent and its MCP server.
            sessions.remove(id);
        }
        stale
    }
}

/// Look for idle artifact sessions every few minutes, for as long as the app runs.
pub fn sweep_idle(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_SWEEP).await;
            let state = app.state::<AppState>();
            for id in state.artifacts.close_idle(IDLE_CLOSE).await {
                tracing::info!(artifact = %id, "closed an idle artifact session");
            }
        }
    });
}

/// The run an artifact's agent has in flight: what its tool calls hang off.
pub async fn running(app: &AppHandle, id: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let sessions = state.artifacts.sessions.lock().await;
    sessions.get(id).and_then(|a| a.live.running.clone())
}

/// Where an artifact's agent works: a folder of its own under the app's
/// data, which also keeps what is sent with messages (a board's picture).
pub fn workdir(state: &AppState, id: &str) -> Result<PathBuf, String> {
    let dir = state.artifacts_dir.join(id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn info(state: &AppState, id: &str) -> Result<ArtifactInfo, String> {
    Ok(state.store.artifact(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no artifact {id}"))?.0)
}

/// The artifact's agent session, opened (or reopened on another agent or
/// options) when needed. The second value says whether no turn has been
/// sent on it yet (the preamble goes with the first). The sessions lock is
/// not held while an agent starts, so other artifacts' tool calls, stops
/// and states do not wait on it; `busy` already keeps one turn per artifact.
async fn open(app: &AppHandle, a: &ArtifactInfo) -> Result<(Arc<AgentSession>, bool), String> {
    let state = app.state::<AppState>();
    if a.agent.is_empty() {
        return Err(format!("{} has no agent", a.id));
    }
    let wanted = fingerprint(&a.agent, &a.config);
    {
        let mut sessions = state.artifacts.sessions.lock().await;
        match sessions.get(&a.id) {
            Some(s) if s.fingerprint == wanted => return Ok((s.live.session.clone(), s.live.turns == 0)),
            Some(_) => {
                sessions.remove(&a.id);
            }
            None => {}
        }
    }
    {
        let tools = match a.kind.as_str() {
            design::KIND => design::tools(app.clone(), a.id.clone()),
            other => return Err(format!("{other} artifacts have no agent yet")),
        };
        let spec = state.spec_for(&a.agent)?;
        let mcp = McpServer::start("divixi", tools).await.map_err(|e| e.to_string())?;
        let cwd = workdir(&state, &a.id)?;
        let mut opts = session_options(&state, &a.agent, &cwd.display().to_string(), &a.config, Some(&mcp));
        let key = format!("artifact_session:{}:{}", a.id, a.agent);
        opts.resume = state.store.get_meta(&key).ok().flatten();
        tracing::info!(artifact = %a.id, kind = %a.kind, agent = %a.agent, resume = ?opts.resume, "opening artifact session");
        let session = AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?;
        let resumed = session.resumed();
        if let Err(err) = state.store.set_meta(&key, session.session_id()) {
            tracing::warn!(%err, "could not remember artifact session id");
        }
        let session = Arc::new(session);
        // A resumed session already had its preamble.
        let turns = u32::from(resumed);
        state.artifacts.sessions.lock().await.insert(
            a.id.clone(),
            Agent {
                live: Live { agent: a.agent.clone(), cwd: String::new(), session: session.clone(), turns, running: None, used: Instant::now() },
                fingerprint: wanted,
                used: Instant::now(),
                _mcp: mcp,
            },
        );
        Ok((session, turns == 0))
    }
}

/// One human message to an artifact's agent. For a design, `image` is a PNG
/// of its board (base64) when it has ink and `selected` the items picked on
/// it; `attached` are the human's files.
pub async fn turn(
    app: AppHandle,
    id: String,
    text: String,
    image: Option<String>,
    selected: Vec<String>,
    lang: String,
    attached: Vec<PathBuf>,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let a = info(&state, &id)?;
    if !state.artifacts.busy.lock().insert(id.clone()) {
        return Err("the agent is still responding".to_string());
    }
    let outcome: Result<String, String> = async {
        let (session, first) = open(&app, &a).await?;
        let cwd = workdir(&state, &id)?;
        // The human's files first, then what the kind adds.
        let mut files = attached.clone();
        let (preamble, context) = match a.kind.as_str() {
            design::KIND => {
                if let Some(data) = image.filter(|d| !d.is_empty()) {
                    // Decoding and writing a large picture is blocking work.
                    let path = cwd.join("board.png");
                    let target = path.clone();
                    tokio::task::spawn_blocking(move || -> Result<(), String> {
                        use base64::Engine;
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(data.as_bytes())
                            .map_err(|e| format!("bad board picture: {e}"))?;
                        std::fs::write(&target, bytes).map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|e| e.to_string())??;
                    files.push(path);
                }
                // Reference files the human picked go with the message itself.
                for f in design::reference_files(&state, &id, Some(&selected)) {
                    if !files.contains(&f) {
                        files.push(f);
                    }
                }
                (design::preamble(&lang, &a.title), design::context(&state, &id, &selected)?)
            }
            other => return Err(format!("{other} artifacts have no agent yet")),
        };
        let body = if text.trim().is_empty() { "(look at it)".to_string() } else { text.clone() };
        let sent = if first { format!("{preamble}\n\n---\n\n{body}\n\n{context}") } else { format!("{body}\n\n{context}") };
        let run = state
            .store
            .begin_run(&run_key(&id), SESSION, &a.agent, &with_attachments(&text, &attached), &cwd.display().to_string())
            .map_err(|e| e.to_string())?;
        // Counted only now it goes out: a turn that failed before this
        // point leaves the next one still the first (with the preamble).
        if let Some(s) = state.artifacts.sessions.lock().await.get_mut(&id) {
            s.live.running = Some(run.clone());
            s.live.turns += 1;
            s.used = Instant::now();
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tauri::async_runtime::spawn(pump(app.clone(), run_key(&id), SESSION.to_string(), run.clone(), rx));
        let _ = tx.send(AgentEvent::Started { session_id: session.session_id().to_string(), cwd: cwd.display().to_string() });

        let app_t = app.clone();
        let (id_t, run_t) = (id.clone(), run.clone());
        tauri::async_runtime::spawn(async move {
            let result = session.prompt_with(sent, files, tx.clone()).await;
            let st = app_t.state::<AppState>();
            {
                let mut sessions = st.artifacts.sessions.lock().await;
                if let Err(err) = result {
                    let _ = tx.send(AgentEvent::Failed { error: err.to_string() });
                    sessions.remove(&id_t);
                } else if let Some(s) = sessions.get_mut(&id_t) {
                    if s.live.running.as_deref() == Some(run_t.as_str()) {
                        s.live.running = None;
                    }
                    s.used = Instant::now();
                }
            }
            st.artifacts.busy.lock().remove(&id_t);
        });
        Ok(run)
    }
    .await;
    if outcome.is_err() {
        state.artifacts.busy.lock().remove(&id);
    }
    outcome
}

/// Answer a permission question an artifact's agent is waiting on.
pub async fn answer_permission(app: &AppHandle, id: &str, request: &str, option: Option<&str>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let sessions = state.artifacts.sessions.lock().await;
    let s = sessions.get(id).ok_or("the agent's session is closed; the question went with it")?;
    s.live.session.answer_permission(request, option).map_err(|e| e.to_string())
}

/// Stop the artifact agent's turn in flight.
pub async fn cancel(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let sessions = state.artifacts.sessions.lock().await;
    if let Some(s) = sessions.get(id) {
        s.live.session.cancel();
    }
}

/// Write an artifact out as files a track's conductor can be given: the
/// text the window made of it (a design's brief) and, for a design with
/// ink, a picture of its board. Returns their paths.
pub fn export(state: &AppState, id: &str, markdown: &str, png: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let a = info(state, id)?;
    let dir = workdir(state, id)?.join("export");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem: String = a
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let stem = if stem.is_empty() { a.id.clone() } else { format!("{}-{stem}", a.kind) };
    let md = dir.join(format!("{stem}.md"));
    std::fs::write(&md, markdown).map_err(|e| e.to_string())?;
    let mut out = vec![md];
    // A design's reference files go along to the track too.
    if a.kind == design::KIND {
        out.extend(design::reference_files(state, id, None).into_iter().take(10));
    }
    if let Some(data) = png.filter(|d| !d.is_empty()) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| format!("bad picture: {e}"))?;
        let path = dir.join(format!("{stem}.png"));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        out.push(path);
    }
    Ok(out)
}

