//! The knowledge library in the app. The human adds files (from the file
//! viewer or a picker); each becomes an artifact of kind `knowledge` and a
//! source in the library's own database. A single loop syncs sources one at
//! a time: read, chunk, have an agent describe every chunk (title, summary,
//! category, entities, relations) and the document (topic, themes), store.
//! A watcher re-queues files that changed. The conductor searches the
//! library through its MCP tools; workers get what they need in their task.
//!
//! The describing agent is the one chosen in the settings, run as a small
//! pool of plain sessions (no MCP, a read-only mode when the agent has one,
//! every permission question refused). Sessions start fresh after
//! [`RESET_AFTER`] calls and close when the library has been idle for
//! [`POOL_IDLE`].

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use orchestra_acp::{AgentSession, ConfigOptionInfo};
use orchestra_core::AgentEvent;
use orchestra_knowledge::store::status;
use orchestra_knowledge::{chunk_document, extract, read, Extraction, FileState, Graph, Item, KnowledgeDb, NewItem, Shape, Source, Stats};
use orchestra_store::ArtifactInfo;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

use crate::{embed, AppState, SETTING_PREFIX};

/// The artifact kind a library document is listed as.
pub const KIND: &str = "knowledge";
/// Window event carrying a source whenever it changes.
const EVENT: &str = "knowledge";
/// Window event when embeddings moved on (or failed).
const EMBED_EVENT: &str = "knowledge-embedding";
/// Window event when a source is gone.
const REMOVED_EVENT: &str = "knowledge-removed";

/// How often files are checked for changes.
const WATCH_EVERY: Duration = Duration::from_secs(5 * 60);
/// Describing sessions close after this long without work.
const POOL_IDLE: Duration = Duration::from_secs(5 * 60);
/// A describing session starts over after this many calls, so its context
/// does not grow without end.
const RESET_AFTER: usize = 20;
/// One describing call may take this long.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Settings, under the app's `setting:` prefix.
pub const SETTING_AGENT: &str = "knowledge.agent";
pub const SETTING_CONFIG: &str = "knowledge.config";
pub const SETTING_POOL: &str = "knowledge.pool";
pub const SETTING_EXTRACT: &str = "knowledge.extract";
const DEFAULT_POOL: usize = 2;
const MAX_POOL: usize = 5;

/// The library and its sync queue.
pub struct Library {
    pub db: Arc<KnowledgeDb>,
    queue: mpsc::UnboundedSender<String>,
    rx: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<String>>>,
    /// Sources waiting in the queue, so one is never queued twice.
    queued: parking_lot::Mutex<HashSet<String>>,
    /// The describing agents' working folder: empty, and not a project.
    work_dir: PathBuf,
    /// Wakes the embedding loop (a sync landed, the settings changed).
    embed_wake: tokio::sync::Notify,
    /// Why the last embedding request failed, "" when it did not.
    embed_error: parking_lot::Mutex<String>,
}

impl Library {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let work_dir = dir.join("work");
        std::fs::create_dir_all(&work_dir)?;
        let db = KnowledgeDb::open(dir.join("knowledge.db"))?;
        let (queue, rx) = mpsc::unbounded_channel();
        Ok(Self {
            db: Arc::new(db),
            queue,
            rx: parking_lot::Mutex::new(Some(rx)),
            queued: Default::default(),
            work_dir,
            embed_wake: tokio::sync::Notify::new(),
            embed_error: Default::default(),
        })
    }

    /// Queue a source for syncing, unless it is queued already.
    pub fn enqueue(&self, id: &str) {
        if self.queued.lock().insert(id.to_string()) {
            let _ = self.queue.send(id.to_string());
        }
    }
}

/// How documents are described, from the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    agent: String,
    config: BTreeMap<String, String>,
    pool: usize,
    extract: bool,
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.store.get_meta(&format!("{SETTING_PREFIX}{key}")).ok().flatten().filter(|v| !v.trim().is_empty())
}

fn settings(state: &AppState) -> Settings {
    let agent = setting(state, SETTING_AGENT).filter(|a| state.spec_for(a).is_ok()).unwrap_or_else(|| {
        // Unset: the first agent that is ready.
        state
            .load_agents()
            .ok()
            .flatten()
            .unwrap_or_default()
            .iter()
            .map(|s| s.kind.id().to_string())
            .find(|id| state.spec_for(id).is_ok())
            .unwrap_or_default()
    });
    // Never chosen: the agent's cheapest model and least effort.
    let config = setting(state, SETTING_CONFIG)
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| cheapest_config(&state.config_options_for(&agent)));
    Settings {
        agent,
        config,
        pool: setting(state, SETTING_POOL).and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_POOL).clamp(1, MAX_POOL),
        extract: setting(state, SETTING_EXTRACT).as_deref() != Some("off"),
    }
}

/// Model families that cost least, cheapest first, as whole words of a
/// model's id or name ("gemini" is not "mini").
const CHEAP_FAMILIES: [&str; 6] = ["nano", "mini", "lite", "haiku", "flash", "small"];
/// Effort levels, least first.
const LOW_EFFORT: [&str; 4] = ["none", "minimal", "low", "medium"];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// The first version-like number in an id ("gpt-5.4-mini" → 5.4).
fn version(id: &str) -> f64 {
    let start = id.find(|c: char| c.is_ascii_digit());
    start
        .map(|i| id[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>())
        .and_then(|v| v.trim_end_matches('.').parse().ok())
        .unwrap_or(f64::MAX)
}

/// The agent's cheapest model by its name: the cheapest family, a low
/// effort variant, then the oldest version. `None` when no name says.
pub fn cheapest_model(choices: &[orchestra_acp::ConfigChoice]) -> Option<String> {
    choices
        .iter()
        .filter_map(|c| {
            let w = words(&format!("{} {}", c.id, c.name));
            let tier = CHEAP_FAMILIES.iter().position(|f| w.iter().any(|x| x == f))?;
            let low = w.iter().any(|x| x == "low");
            Some((tier, !low, version(&c.id), c.id.clone()))
        })
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)))
        .map(|(_, _, _, id)| id)
}

/// Options for describing documents cheaply: the cheapest model the agent
/// names, and its least effort.
pub fn cheapest_config(options: &[ConfigOptionInfo]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(m) = options.iter().find(|o| o.category == "model") {
        if let Some(id) = cheapest_model(&m.choices) {
            out.insert(m.id.clone(), id);
        }
    }
    if let Some(t) = options.iter().find(|o| o.category == "thought_level") {
        if let Some(c) = LOW_EFFORT.iter().find_map(|l| t.choices.iter().find(|c| c.id.eq_ignore_ascii_case(l))) {
            out.insert(t.id.clone(), c.id.clone());
        }
    }
    out
}

/// A mode that reads but does not act, when the agent offers one.
fn read_only_mode(options: &[ConfigOptionInfo]) -> Option<String> {
    let modes = options.iter().find(|o| o.category == "mode")?;
    let key = |id: &str| id.rsplit(['#', '/']).next().unwrap_or(id).to_ascii_lowercase();
    ["plan", "read-only", "readonly", "ask", "default"]
        .iter()
        .find_map(|want| modes.choices.iter().find(|c| key(&c.id) == *want).map(|c| c.id.clone()))
}

/// One describing session.
struct Worker {
    session: AgentSession,
    calls: usize,
}

/// The describing sessions, opened on first use.
struct Pool {
    settings: Settings,
    workers: Vec<Option<Worker>>,
}

async fn open_worker(app: &AppHandle, s: &Settings, dir: &Path) -> Result<Worker, String> {
    if s.agent.is_empty() {
        return Err("no agent is ready to describe documents".to_string());
    }
    let state = app.state::<AppState>();
    let spec = state.spec_for(&s.agent)?;
    let mut opts = crate::conductor::session_options(&state, &s.agent, &dir.to_string_lossy(), &s.config, None);
    if opts.mode.is_none() {
        opts.mode = read_only_mode(&state.config_options_for(&s.agent));
    }
    let session = AgentSession::open(&spec, opts).await.map_err(|e| format!("could not start {}: {e}", s.agent))?;
    Ok(Worker { session, calls: 0 })
}

/// One prompt to a describing session, its reply text. Permission questions
/// are refused: describing needs no tools. A failure closes the session so
/// the next call starts a fresh one.
async fn ask(app: &AppHandle, s: &Settings, dir: &Path, slot: &mut Option<Worker>, prompt: String) -> Result<String, String> {
    if slot.as_ref().is_some_and(|w| w.calls >= RESET_AFTER) {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(open_worker(app, s, dir).await?);
    }
    let Some(w) = slot.as_mut() else { return Err("no session".to_string()) };
    w.calls += 1;
    let session = &w.session;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let turn = session.prompt(prompt, tx);
    let collect = async {
        let mut text = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text: t } => text.push_str(&t),
                AgentEvent::Permission { request, .. } => {
                    let _ = session.answer_permission(&request, None);
                }
                _ => {}
            }
        }
        text
    };
    let outcome = tokio::time::timeout(CALL_TIMEOUT, async { tokio::join!(turn, collect) }).await;
    match outcome {
        Ok((Ok(()), text)) => Ok(text),
        Ok((Err(err), _)) => {
            *slot = None;
            Err(err.to_string())
        }
        Err(_) => {
            session.cancel();
            *slot = None;
            Err("the agent took too long".to_string())
        }
    }
}

fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn emit_source(app: &AppHandle, db: &KnowledgeDb, id: &str) {
    if let Ok(Some(s)) = db.source(id) {
        let _ = app.emit(EVENT, &s);
    }
}

/// Describe every chunk across the pool's sessions, in parallel. Returns
/// one extraction per chunk (`None` where describing failed) and the first
/// error seen.
async fn describe(app: &AppHandle, pool: &mut Pool, dir: &Path, db: &KnowledgeDb, id: &str, chunks: &[orchestra_knowledge::Chunk]) -> (Vec<Option<Extraction>>, Option<String>) {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results = parking_lot::Mutex::new(vec![None; chunks.len()]);
    let first_error: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);
    let settings = pool.settings.clone();
    let lanes = pool.workers.iter_mut().map(|slot| {
        let (next, done, results, first_error, settings) = (&next, &done, &results, &first_error, &settings);
        async move {
            loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(chunk) = chunks.get(i) else { break };
                match ask(app, settings, dir, slot, extract::extraction_prompt(&chunk.content, &nonce())).await {
                    Ok(reply) => match extract::parse_extraction(&reply) {
                        Some(x) => results.lock()[i] = Some(x),
                        None => {
                            first_error.lock().get_or_insert_with(|| "the agent's reply was not the JSON asked for".to_string());
                        }
                    },
                    Err(err) => {
                        first_error.lock().get_or_insert(err.clone());
                        // No session could start: the other chunks would fail the same way.
                        if err.starts_with("could not start") || err.starts_with("no agent") {
                            next.store(chunks.len(), Ordering::SeqCst);
                        }
                    }
                }
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = db.set_progress(id, n.min(chunks.len()), chunks.len());
                emit_source(app, db, id);
            }
        }
    });
    futures::future::join_all(lanes).await;
    (results.into_inner(), first_error.into_inner())
}

/// Bring one source up to date with its file.
async fn sync(app: &AppHandle, id: &str, pool: &mut Option<Pool>) {
    let state = app.state::<AppState>();
    let lib = &state.library;
    let db = lib.db.clone();
    let Ok(Some(src)) = db.source(id) else { return };
    let path = PathBuf::from(&src.uri);
    if !path.is_file() {
        let _ = db.set_status(id, status::MISSING, "the file is gone");
        emit_source(app, &db, id);
        return;
    }
    let read_path = path.clone();
    let file = match tokio::task::spawn_blocking(move || read::read_file(&read_path)).await {
        Ok(Ok(f)) => f,
        Ok(Err(err)) => {
            let _ = db.set_status(id, status::ERROR, &err.to_string());
            remember_file(&db, id, &path);
            emit_source(app, &db, id);
            return;
        }
        Err(err) => {
            let _ = db.set_status(id, status::ERROR, &err.to_string());
            emit_source(app, &db, id);
            return;
        }
    };
    let file_state = FileState { hash: file.hash.clone(), mtime_ms: file.mtime_ms, size: file.size as i64 };
    if file.hash == src.content_hash && src.items > 0 {
        let _ = db.touch(id, &file_state);
        emit_source(app, &db, id);
        return;
    }
    if let Ok(Some(owner)) = db.hash_owner(&file.hash, id) {
        let _ = db.set_status(id, status::DUPLICATE, &format!("the same text is already in the library as {owner}"));
        remember_file(&db, id, &path);
        emit_source(app, &db, id);
        return;
    }

    let shape = read::shape_of(&path).unwrap_or(Shape::Text);
    let chunks = chunk_document(shape, &file.text);
    let _ = db.set_status(id, status::INDEXING, "");
    let _ = db.set_progress(id, 0, chunks.len());
    emit_source(app, &db, id);

    let s = settings(&state);
    let mut extractions: Vec<Option<Extraction>> = vec![None; chunks.len()];
    let mut note = String::new();
    if s.extract && !chunks.is_empty() {
        if pool.as_ref().is_some_and(|p| p.settings != s) {
            *pool = None;
        }
        let p = pool.get_or_insert_with(|| Pool { settings: s.clone(), workers: (0..s.pool).map(|_| None).collect() });
        let (found, error) = describe(app, p, &lib.work_dir, &db, id, &chunks).await;
        extractions = found;
        let failed = extractions.iter().filter(|x| x.is_none()).count();
        if failed > 0 {
            note = format!(
                "{failed} of {} chunks were not described{}",
                chunks.len(),
                error.map(|e| format!(": {e}")).unwrap_or_default()
            );
        }
    }
    let tag = match shape {
        Shape::Markdown => "content_type:markdown",
        Shape::Code => "content_type:code",
        Shape::Text => "content_type:text",
    };
    let extracted = extractions.iter().any(Option::is_some);
    let summaries: Vec<String> = extractions.iter().flatten().map(|x| x.summary.clone()).filter(|s| !s.is_empty()).collect();
    let items: Vec<NewItem> = chunks
        .into_iter()
        .zip(extractions)
        .map(|(chunk, extraction)| NewItem { chunk, extraction, tags: vec![tag.to_string()] })
        .collect();
    let store_db = db.clone();
    let owned = id.to_string();
    let stored = tokio::task::spawn_blocking(move || store_db.replace_items(&owned, &items, extracted, &file_state)).await;
    match stored {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            // Removed while it was being described, most likely.
            tracing::warn!(%err, source = id, "could not store knowledge items");
            let _ = db.set_status(id, status::ERROR, &err.to_string());
            emit_source(app, &db, id);
            return;
        }
        Err(err) => {
            tracing::warn!(%err, "knowledge store task failed");
            return;
        }
    }
    if !note.is_empty() {
        let _ = db.set_status(id, status::SYNCED, &note);
    }
    lib.embed_wake.notify_one();
    if !summaries.is_empty() {
        if let Some(p) = pool.as_mut() {
            let settings = p.settings.clone();
            if let Some(slot) = p.workers.first_mut() {
                match ask(app, &settings, &lib.work_dir, slot, extract::summary_prompt(&summaries.join("\n"))).await {
                    Ok(reply) => {
                        if let Some(sum) = extract::parse_summary(&reply) {
                            let _ = db.set_summary(id, &sum.topic, &sum.themes);
                        }
                    }
                    Err(err) => tracing::info!(%err, source = id, "no document summary"),
                }
            }
        }
    }
    emit_source(app, &db, id);
}

/// Keep a file's time and size so the watcher does not retry it until it
/// changes.
fn remember_file(db: &KnowledgeDb, id: &str, path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = db.remember_file(id, read::mtime_ms(&meta), meta.len() as i64);
    }
}

/// Queue sources whose file changed, and mark vanished files missing.
fn check_files(app: &AppHandle) {
    let state = app.state::<AppState>();
    let lib = &state.library;
    let Ok(sources) = lib.db.sources() else { return };
    for s in sources {
        match std::fs::metadata(&s.uri) {
            Err(_) => {
                if s.status != status::MISSING {
                    let _ = lib.db.set_status(&s.id, status::MISSING, "the file is gone");
                    emit_source(app, &lib.db, &s.id);
                }
            }
            Ok(meta) => {
                let changed = read::mtime_ms(&meta) != s.mtime_ms || meta.len() as i64 != s.size;
                if changed || s.status == status::PENDING || s.status == status::MISSING {
                    lib.enqueue(&s.id);
                }
            }
        }
    }
}

/// Embed every item not yet in the configured space, a batch at a time,
/// within the rate limit. Stops at the first failure and says why.
async fn embed_pending(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(ep) = embed::endpoint(&state) else { return };
    let sig = ep.signature();
    let db = state.library.db.clone();
    let spacing = Duration::from_millis(60_000 / embed::PER_MINUTE);
    loop {
        let batch = match db.to_embed(&sig, embed::BATCH) {
            Ok(b) if !b.is_empty() => b,
            _ => break,
        };
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        match embed::embed(&ep, &texts).await {
            Ok(vectors) => {
                let pairs: Vec<(i64, Vec<f32>)> = batch.iter().map(|(id, _)| *id).zip(vectors).collect();
                if let Err(err) = db.set_embeddings(&sig, &pairs) {
                    *state.library.embed_error.lock() = err.to_string();
                    break;
                }
                state.library.embed_error.lock().clear();
                let _ = app.emit(EMBED_EVENT, ());
            }
            Err(err) => {
                tracing::warn!(%err, "embedding failed");
                *state.library.embed_error.lock() = err;
                let _ = app.emit(EMBED_EVENT, ());
                break;
            }
        }
        // The settings may have changed meanwhile: go again in the new space.
        if embed::endpoint(&state).map(|e| e.signature()) != Some(sig.clone()) {
            break;
        }
        tokio::time::sleep(spacing).await;
    }
}

/// The vector of a search query, when embeddings are on and the library
/// has vectors in their space; `None` (search goes without) otherwise.
pub async fn query_vector(app: &AppHandle, query: &str) -> Option<(Vec<f32>, String)> {
    let state = app.state::<AppState>();
    let ep = embed::endpoint(&state)?;
    let sig = ep.signature();
    if state.library.db.embedded(&sig).ok()?.0 == 0 {
        return None;
    }
    match tokio::time::timeout(embed::QUERY_TIMEOUT, embed::embed(&ep, &[query.to_string()])).await {
        Ok(Ok(mut v)) => v.pop().map(|v| (v, sig)),
        Ok(Err(err)) => {
            tracing::info!(%err, "searching without the query's vector");
            None
        }
        Err(_) => None,
    }
}

/// Start the sync loop, the watcher and the embedding loop.
pub fn start(app: AppHandle) {
    let rx = app.state::<AppState>().library.rx.lock().take();
    let Some(mut rx) = rx else { return };
    let worker = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut pool: Option<Pool> = None;
        loop {
            match tokio::time::timeout(POOL_IDLE, rx.recv()).await {
                Ok(Some(id)) => {
                    worker.state::<AppState>().library.queued.lock().remove(&id);
                    sync(&worker, &id, &mut pool).await;
                }
                Ok(None) => break,
                // Idle: let the describing agents go.
                Err(_) => pool = None,
            }
        }
    });
    let embedder = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            embed_pending(&embedder).await;
            let wake = &embedder.state::<AppState>().library.embed_wake;
            // Woken by a sync or a settings change; otherwise retry now and then.
            let _ = tokio::time::timeout(Duration::from_secs(10 * 60), wake.notified()).await;
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            let a = app.clone();
            let _ = tokio::task::spawn_blocking(move || check_files(&a)).await;
            tokio::time::sleep(WATCH_EVERY).await;
        }
    });
}

/// Paths as the library keeps them: absolute, without Windows' `\\?\`.
fn normalize(path: &Path) -> Result<PathBuf, String> {
    let full = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let text = full.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => full,
    })
}

/// A path the UI names: absolute, or relative to a track's folder.
fn locate(state: &AppState, path: &str, track: Option<&str>) -> Result<PathBuf, String> {
    let full = match track {
        Some(t) => crate::workspace::resolve(&crate::track_root(state, t)?, path)?,
        None => PathBuf::from(path),
    };
    normalize(&full)
}

/// A document just added: its source and the artifact that lists it.
#[derive(Serialize)]
pub struct Added {
    source: Source,
    artifact: ArtifactInfo,
}

/// Add a file to the library and start describing it.
#[tauri::command]
pub async fn knowledge_add(app: AppHandle, path: String, track: Option<String>) -> Result<Added, String> {
    let state = app.state::<AppState>();
    let full = locate(&state, &path, track.as_deref())?;
    if !full.is_file() {
        return Err(format!("not a file: {}", full.display()));
    }
    if let Some(why) = read::refusal(&full) {
        return Err(why);
    }
    let uri = full.to_string_lossy().to_string();
    if state.library.db.source_by_uri(&uri).map_err(|e| e.to_string())?.is_some() {
        return Err("already in the knowledge library".to_string());
    }
    let title = full.file_name().and_then(|n| n.to_str()).unwrap_or("document").to_string();
    let artifact = state.store.create_artifact(KIND, &title, "", "{}").map_err(|e| e.to_string())?;
    let source = match state.library.db.add_source(&artifact.id, "local_file", &uri) {
        Ok(s) => s,
        Err(err) => {
            let _ = state.store.delete_artifact(&artifact.id);
            return Err(err.to_string());
        }
    };
    state.library.enqueue(&artifact.id);
    let _ = app.emit(EVENT, &source);
    Ok(Added { source, artifact })
}

/// Forget a document (its artifact is being deleted).
pub fn remove(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    if let Err(err) = state.library.db.delete_source(id) {
        tracing::warn!(%err, source = id, "could not remove knowledge source");
    }
    let _ = app.emit(REMOVED_EVENT, id);
}

#[tauri::command(async)]
pub fn knowledge_sources(state: State<'_, AppState>) -> Result<Vec<Source>, String> {
    state.library.db.sources().map_err(|e| e.to_string())
}

/// The source for a file, if it is in the library.
#[tauri::command(async)]
pub fn knowledge_source_for(state: State<'_, AppState>, path: String, track: Option<String>) -> Result<Option<Source>, String> {
    let Ok(full) = locate(&state, &path, track.as_deref()) else { return Ok(None) };
    state.library.db.source_by_uri(&full.to_string_lossy()).map_err(|e| e.to_string())
}

/// Re-read and re-describe a document now, even if its text is unchanged.
#[tauri::command(async)]
pub fn knowledge_sync(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.library.db.forget_hash(&id).map_err(|e| e.to_string())?;
    state.library.db.set_status(&id, status::PENDING, "").map_err(|e| e.to_string())?;
    state.library.enqueue(&id);
    emit_source(&app, &state.library.db, &id);
    Ok(())
}

/// An item as listed: with its score when it came from a search.
#[derive(Serialize)]
pub struct Listed {
    #[serde(flatten)]
    item: Item,
    score: Option<f64>,
    match_type: Option<String>,
}

/// Items of one source or of all, or the matches of a search.
#[tauri::command]
pub async fn knowledge_items(app: AppHandle, source: Option<String>, query: Option<String>) -> Result<Vec<Listed>, String> {
    let db = app.state::<AppState>().library.db.clone();
    match query.filter(|q| !q.trim().is_empty()) {
        Some(q) => Ok(db
            .search(&q, 30, source.as_deref(), query_vector(&app, &q).await.as_ref().map(|(v, s)| (v.as_slice(), s.as_str())))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|h| Listed { item: h.item, score: Some(h.score), match_type: Some(h.match_type) })
            .collect()),
        None => Ok(db
            .items(source.as_deref())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| Listed { item, score: None, match_type: None })
            .collect()),
    }
}

#[tauri::command(async)]
pub fn knowledge_graph(state: State<'_, AppState>) -> Result<Graph, String> {
    state.library.db.graph(300).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn knowledge_entity_items(state: State<'_, AppState>, id: i64) -> Result<Vec<Item>, String> {
    state.library.db.entity_items(id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn knowledge_stats(state: State<'_, AppState>) -> Result<Stats, String> {
    state.library.db.stats().map_err(|e| e.to_string())
}

/// Where embeddings stand: whether they are on, the model, how many items
/// have a vector in its space, and the last failure.
#[derive(Serialize)]
pub struct EmbeddingStatus {
    enabled: bool,
    model: String,
    embedded: i64,
    total: i64,
    error: String,
}

#[tauri::command(async)]
pub fn knowledge_embedding_status(state: State<'_, AppState>) -> Result<EmbeddingStatus, String> {
    let ep = embed::endpoint(&state);
    let sig = ep.as_ref().map(|e| e.signature()).unwrap_or_default();
    let (embedded, total) = state.library.db.embedded(&sig).map_err(|e| e.to_string())?;
    Ok(EmbeddingStatus {
        enabled: ep.is_some(),
        model: ep.map(|e| e.model).unwrap_or_default(),
        embedded,
        total,
        error: state.library.embed_error.lock().clone(),
    })
}

/// Try an endpoint before saving it: the length of one vector.
#[tauri::command]
pub async fn knowledge_embed_test(url: String, model: String, key: String, dims: Option<u32>) -> Result<usize, String> {
    let ep = embed::Endpoint { url: url.trim().to_string(), model: model.trim().to_string(), key: key.trim().to_string(), dims: dims.filter(|d| *d > 0) };
    if ep.url.is_empty() || ep.model.is_empty() {
        return Err("the endpoint URL and the model are needed".to_string());
    }
    let v = embed::embed(&ep, &["Divixi knowledge library 지식 라이브러리".to_string()]).await?;
    Ok(v.first().map(Vec::len).unwrap_or(0))
}

/// Start embedding now (the settings just changed).
#[tauri::command]
pub fn knowledge_embed_now(state: State<'_, AppState>) {
    state.library.embed_error.lock().clear();
    state.library.embed_wake.notify_one();
}

/// The options documents are described with by default on an agent: its
/// cheapest model and least effort.
#[tauri::command(async)]
pub fn knowledge_default_config(state: State<'_, AppState>, agent: String) -> BTreeMap<String, String> {
    cheapest_config(&state.config_options_for(&agent))
}

/// File extensions the library takes.
#[tauri::command]
pub fn knowledge_formats() -> Vec<&'static str> {
    read::supported_extensions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_acp::ConfigChoice;

    fn choices(ids: &[(&str, &str)]) -> Vec<ConfigChoice> {
        ids.iter().map(|(id, name)| ConfigChoice { id: id.to_string(), name: name.to_string(), description: None, group: None }).collect()
    }

    #[test]
    fn picks_the_cheapest_model_by_name() {
        let claude = choices(&[("default", "Default (recommended)"), ("sonnet", "Sonnet 5"), ("haiku", "Haiku 4.5"), ("opus", "Opus 5")]);
        assert_eq!(cheapest_model(&claude).as_deref(), Some("haiku"));
        let copilot = choices(&[
            ("auto", "Auto"),
            ("gpt-5.4", "GPT-5.4"),
            ("gpt-5.4-mini", "GPT-5.4 mini"),
            ("gpt-5-mini", "GPT-5 mini"),
            ("claude-haiku-4.5", "Claude Haiku 4.5"),
            ("gemini-3.5-flash", "Gemini 3.5 Flash"),
        ]);
        assert_eq!(cheapest_model(&copilot).as_deref(), Some("gpt-5-mini"));
        let antigravity = choices(&[
            ("gemini-3.8-flash-high", "Gemini 3.8 Flash (High)"),
            ("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)"),
            ("gemini-3.6-flash-low", "Gemini 3.6 Flash (Low)"),
            ("gemini-pro-agent", "Gemini 3.1 Pro (High)"),
        ]);
        assert_eq!(cheapest_model(&antigravity).as_deref(), Some("gemini-3.6-flash-low"));
        let codex = choices(&[("gpt-5.6-terra", "5.6 Terra"), ("gpt-5.6-luna", "5.6 Luna"), ("gpt-5.5", "5.5")]);
        assert_eq!(cheapest_model(&codex), None, "no name says which is cheaper");
    }

    #[test]
    fn cheapest_config_lowers_effort_too() {
        let opt = |id: &str, category: &str, ids: &[(&str, &str)]| ConfigOptionInfo {
            id: id.into(),
            name: id.into(),
            description: None,
            category: category.into(),
            current: String::new(),
            choices: choices(ids),
        };
        let c = cheapest_config(&[
            opt("model", "model", &[("sonnet", "Sonnet"), ("haiku", "Haiku")]),
            opt("effort", "thought_level", &[("default", "Default"), ("low", "Low"), ("high", "High")]),
        ]);
        assert_eq!(c.get("model").map(String::as_str), Some("haiku"));
        assert_eq!(c.get("effort").map(String::as_str), Some("low"));
    }
}
