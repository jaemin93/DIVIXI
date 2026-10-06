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
use orchestra_knowledge::store::{status, GENERAL};
use orchestra_knowledge::{chunk_document, extract, read, Extraction, FileState, Graph, Item, KnowledgeDb, Library as KLibrary, NewItem, Shape, Source, Stats, ToEmbed};
use orchestra_store::ArtifactInfo;
use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::AppHandle;
use tokio::sync::mpsc;

use crate::{embed, AppState, SETTING_PREFIX};

/// The artifact kind a library document is listed as.
pub const KIND: &str = "knowledge";
/// The knowledge agent's conversation (kind "graph", kept for the artifacts
/// already stored): one for every library (it is told the library on screen
/// with each message), an artifact so it runs like a design's (its own
/// agent, session and turns), apart from the documents (`KIND`), which are
/// artifacts too.
pub const GRAPH_KIND: &str = "graph";
/// Window event carrying a source whenever it changes.
const EVENT: &str = "knowledge";
/// Items listed at most when nothing narrows the list.
const LIST_LIMIT: usize = 1000;
/// Window event when embeddings moved on (or failed).
const EMBED_EVENT: &str = "knowledge-embedding";
/// Window event when a source is gone.
const REMOVED_EVENT: &str = "knowledge-removed";
/// Window event when the libraries changed (made, renamed, deleted, a document moved).
const LIBRARIES_EVENT: &str = "knowledge-libraries";

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
    /// Sources to describe again on their next sync even if unchanged.
    forced: parking_lot::Mutex<HashSet<String>>,
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
            forced: Default::default(),
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
    // It reads untrusted text: never in a mode that acts unasked.
    opts.restricted = true;
    opts.mode = None;
    let session = AgentSession::open(&spec, opts).await.map_err(|e| format!("could not start {}: {e}", s.agent))?;
    Ok(Worker { session, calls: 0 })
}

/// One prompt to a describing session, its reply text. Describing needs no
/// tools: permission questions are refused, and a tool call cancels the
/// turn. A failure closes the session so the next call starts a fresh one.
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
        let mut used_tool = false;
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text: t } => text.push_str(&t),
                AgentEvent::Permission { request, .. } => {
                    let _ = session.answer_permission(&request, None);
                }
                AgentEvent::ToolCall { title, .. } if !used_tool => {
                    used_tool = true;
                    tracing::warn!(%title, "the describing agent reached for a tool; cancelling the turn");
                    session.cancel();
                }
                _ => {}
            }
        }
        (text, used_tool)
    };
    let outcome = tokio::time::timeout(CALL_TIMEOUT, async { tokio::join!(turn, collect) }).await;
    match outcome {
        Ok((Ok(()), (_, true))) => {
            *slot = None;
            Err("the agent tried to use a tool while describing; its reply was dropped".to_string())
        }
        Ok((Ok(()), (text, false))) => Ok(text),
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
    // Taken here, so a request made while this sync runs forces the next one.
    let forced = lib.forced.lock().remove(id);
    if !forced && file.hash == src.content_hash && src.items > 0 {
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
                // A duplicate is rechecked too: its original may be gone.
                if changed || matches!(s.status.as_str(), status::PENDING | status::MISSING | status::DUPLICATE) {
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
    let mut after = 0;
    let fail = |err: String| {
        tracing::warn!(%err, "embedding failed");
        *state.library.embed_error.lock() = err;
        let _ = app.emit(EMBED_EVENT, ());
    };
    // Nothing left to embed: whatever went wrong last time is over, so the
    // failure stops being shown.
    //
    // This has to be said here. The only other place that clears it is a
    // batch landing, and a loop with nothing to do stores nothing -- so a
    // rate limit hit on the last few items left "Embedding failed: 429" on
    // screen for good once those items went through on a later pass. The
    // human could not tell a stopped run from a finished one.
    let caught_up = || {
        let had = std::mem::take(&mut *state.library.embed_error.lock());
        if !had.is_empty() {
            tracing::info!("everything is embedded; the last failure no longer applies");
            let _ = app.emit(EMBED_EVENT, ());
        }
    };
    loop {
        let (d, s) = (db.clone(), sig.clone());
        let batch = match tokio::task::spawn_blocking(move || d.to_embed(&s, after, embed::BATCH)).await {
            Ok(Ok(b)) if !b.is_empty() => b,
            Ok(Ok(_)) => return caught_up(),
            Ok(Err(err)) => return fail(err.to_string()),
            Err(err) => return fail(err.to_string()),
        };
        after = batch.last().map(|t| t.id).unwrap_or(after);
        let texts: Vec<String> = batch.iter().map(|t| t.text.clone()).collect();
        let mut done: Vec<(ToEmbed, Vec<f32>)> = Vec::new();
        let mut refused: Vec<ToEmbed> = Vec::new();
        match embed::embed(&ep, &texts).await {
            Ok(vectors) => done.extend(batch.into_iter().zip(vectors)),
            // The endpoint refuses something in the batch: find out which, one at a time.
            Err(err) if err.permanent && batch.len() > 1 => {
                let size = batch.len();
                for item in batch {
                    match embed::embed(&ep, std::slice::from_ref(&item.text)).await {
                        Ok(mut v) => done.push((item, v.pop().unwrap_or_default())),
                        Err(e) if e.permanent => {
                            tracing::info!(item = item.id, err = %e, "the endpoint refuses this item; skipping it");
                            refused.push(item);
                        }
                        Err(e) => return fail(e.message),
                    }
                    tokio::time::sleep(embed::spacing(embed::per_minute(&state))).await;
                }
                // Every item refused alone: the endpoint, not the items, is the problem.
                if done.is_empty() && refused.len() == size {
                    return fail(err.message);
                }
            }
            Err(err) if err.permanent => refused.extend(batch),
            Err(err) => return fail(err.message),
        }
        let (d, s) = (db.clone(), sig.clone());
        let stored = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let pairs: Vec<(&ToEmbed, Vec<f32>)> = done.iter().map(|(t, v)| (t, v.clone())).collect();
            d.set_embeddings(&s, &pairs)?;
            for item in &refused {
                d.embed_failed(&s, item)?;
            }
            Ok(())
        })
        .await;
        match stored {
            Ok(Ok(())) => {
                state.library.embed_error.lock().clear();
                let _ = app.emit(EMBED_EVENT, ());
            }
            Ok(Err(err)) => return fail(err.to_string()),
            Err(err) => return fail(err.to_string()),
        }
        // The settings may have changed meanwhile: go again in the new space.
        if embed::endpoint(&state).map(|e| e.signature()) != Some(sig.clone()) {
            break;
        }
        // Read each time, so a changed limit applies at once.
        tokio::time::sleep(embed::spacing(embed::per_minute(&state))).await;
    }
}

/// The vector of a search query, when embeddings are on and the library
/// has vectors in their space; `None` (search goes without) otherwise.
pub async fn query_vector(app: &AppHandle, query: &str) -> Option<(Vec<f32>, String)> {
    let state = app.state::<AppState>();
    let ep = embed::endpoint(&state)?;
    let sig = ep.signature();
    let (db, s) = (state.library.db.clone(), sig.clone());
    if !tokio::task::spawn_blocking(move || db.has_vectors(&s)).await.ok()?.ok()? {
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

/// Add a file to a library (General when none is named) and start describing it.
#[tauri::command]
pub async fn knowledge_add(app: AppHandle, path: String, track: Option<String>, library: Option<i64>) -> Result<Added, String> {
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
    let library = library.unwrap_or(GENERAL);
    if state.library.db.library(library).map_err(|e| e.to_string())?.is_none() {
        return Err(format!("no library {library}"));
    }
    let title = full.file_name().and_then(|n| n.to_str()).unwrap_or("document").to_string();
    let artifact = state.store.create_artifact(KIND, &title, "", "{}").map_err(|e| e.to_string())?;
    let source = match state.library.db.add_source_in(&artifact.id, "local_file", &uri, library) {
        Ok(s) => s,
        Err(err) => {
            let _ = state.store.delete_artifact(&artifact.id);
            return Err(err.to_string());
        }
    };
    state.library.enqueue(&artifact.id);
    let _ = app.emit(EVENT, &source);
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(Added { source, artifact })
}

/// Forget a document (its artifact is being deleted).
pub fn remove(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    if let Err(err) = state.library.db.delete_source(id) {
        tracing::warn!(%err, source = id, "could not remove knowledge source");
    }
    // A duplicate of it may now be the only copy.
    for s in state.library.db.sources().unwrap_or_default() {
        if s.status == status::DUPLICATE {
            state.library.enqueue(&s.id);
        }
    }
    let _ = app.emit(REMOVED_EVENT, id);
    let _ = app.emit(LIBRARIES_EVENT, ());
}

// ----- libraries -----

/// Every library, General first, each with its number of documents.
#[tauri::command(async)]
pub fn knowledge_libraries(state: State<'_, AppState>) -> Result<Vec<KLibrary>, String> {
    state.library.db.libraries().map_err(|e| e.to_string())
}

/// A new, empty library.
#[tauri::command(async)]
pub fn knowledge_library_create(app: AppHandle, name: String) -> Result<KLibrary, String> {
    let made = app.state::<AppState>().library.db.create_library(&name).map_err(|e| e.to_string())?;
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(made)
}

#[tauri::command(async)]
pub fn knowledge_library_rename(app: AppHandle, id: i64, name: String) -> Result<KLibrary, String> {
    let renamed = app.state::<AppState>().library.db.rename_library(id, &name).map_err(|e| e.to_string())?;
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(renamed)
}

/// A library's colour and/or tags, checked as a track's and a design's are.
#[tauri::command(async)]
pub fn knowledge_library_update(app: AppHandle, id: i64, color: Option<String>, tags: Option<Vec<String>>) -> Result<KLibrary, String> {
    let mut look = orchestra_store::TrackPatch { color, tags, ..Default::default() };
    crate::check_patch(&mut look)?;
    let updated = app
        .state::<AppState>()
        .library
        .db
        .set_library_look(id, look.color.as_deref(), look.tags.as_deref())
        .map_err(|e| e.to_string())?;
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(updated)
}

/// Delete an empty library; General, and a library with documents, are refused.
#[tauri::command(async)]
pub fn knowledge_library_delete(app: AppHandle, id: i64) -> Result<(), String> {
    app.state::<AppState>().library.db.delete_library(id).map_err(|e| e.to_string())?;
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(())
}

/// Move a document to another library. Duplicates are told within a
/// library, so whatever was a duplicate is looked at again: the moved
/// document may be the only copy where it went, and one it left behind may
/// now be the only copy there.
#[tauri::command(async)]
pub fn knowledge_move(app: AppHandle, id: String, library: i64) -> Result<Source, String> {
    let state = app.state::<AppState>();
    let moved = state.library.db.move_source(&id, library).map_err(|e| e.to_string())?;
    for s in state.library.db.sources().unwrap_or_default() {
        if s.status == status::DUPLICATE {
            state.library.enqueue(&s.id);
        }
    }
    let _ = app.emit(EVENT, &moved);
    let _ = app.emit(LIBRARIES_EVENT, ());
    Ok(moved)
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
    if state.library.db.source(&id).map_err(|e| e.to_string())?.is_none() {
        return Err(format!("no knowledge source {id}"));
    }
    state.library.forced.lock().insert(id.clone());
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

/// Items of one source or of all, or the matches of a search; of one
/// library, or of every one when `library` is not given.
#[tauri::command]
pub async fn knowledge_items(app: AppHandle, source: Option<String>, query: Option<String>, library: Option<i64>) -> Result<Vec<Listed>, String> {
    let db = app.state::<AppState>().library.db.clone();
    let query = query.filter(|q| !q.trim().is_empty());
    let vector = match &query {
        Some(q) => query_vector(&app, q).await,
        None => None,
    };
    tokio::task::spawn_blocking(move || match query {
        Some(q) => Ok(db
            .search_in(&q, 30, source.as_deref(), library, vector.as_ref().map(|(v, s)| (v.as_slice(), s.as_str())))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|h| Listed { item: h.item, score: Some(h.score), match_type: Some(h.match_type) })
            .collect()),
        None => Ok(db
            .items_in(source.as_deref(), library, LIST_LIMIT)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| Listed { item, score: None, match_type: None })
            .collect()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The entity graph of one library, or of every one.
#[tauri::command(async)]
pub fn knowledge_graph(state: State<'_, AppState>, library: Option<i64>) -> Result<Graph, String> {
    state.library.db.graph_in(library, 300).map_err(|e| e.to_string())
}

/// The entities a search reaches (named, and within two relations), for the
/// graph view to light up beside the query it shows.
#[tauri::command(async)]
pub fn knowledge_query_entities(state: State<'_, AppState>, query: String) -> Result<orchestra_knowledge::store::QueryEntities, String> {
    state.library.db.query_entities(query.trim()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn knowledge_entity_items(state: State<'_, AppState>, id: i64, library: Option<i64>) -> Result<Vec<Item>, String> {
    state.library.db.entity_items_in(id, library).map_err(|e| e.to_string())
}

/// Sources sharing at least half their passages with another: likely
/// copies or versions of one document, for the human to tidy up.
#[tauri::command(async)]
pub fn knowledge_overlaps(state: State<'_, AppState>) -> Result<Vec<orchestra_knowledge::store::Overlap>, String> {
    state.library.db.overlaps(50).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn knowledge_stats(state: State<'_, AppState>, library: Option<i64>) -> Result<Stats, String> {
    state.library.db.stats_in(library).map_err(|e| e.to_string())
}

/// A passage found for the human to attach to a message (`@kb`).
#[derive(Serialize)]
pub struct ContextHit {
    id: i64,
    title: String,
    /// The document's file name.
    source: String,
    section: Option<String>,
    line_start: i64,
    line_end: i64,
    summary: String,
    content: String,
    /// About how many tokens it adds to the message.
    tokens: usize,
    match_type: String,
    /// The library its document is in.
    library: String,
}

/// Passages for `@kb <query>`: the library searched as the conductor
/// searches it (every library), the best few with their size.
#[tauri::command]
pub async fn knowledge_context(app: AppHandle, query: String) -> Result<Vec<ContextHit>, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let vector = query_vector(&app, &query).await;
    let db = app.state::<AppState>().library.db.clone();
    let hits = tokio::task::spawn_blocking(move || db.search(&query, 8, None, vector.as_ref().map(|(v, s)| (v.as_slice(), s.as_str()))))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| {
            let source = std::path::Path::new(&h.source_uri).file_name().and_then(|n| n.to_str()).unwrap_or(&h.source_uri).to_string();
            ContextHit {
                id: h.item.id,
                tokens: orchestra_knowledge::chunk::tokens(&h.item.content),
                title: h.item.title,
                source,
                section: h.item.section,
                line_start: h.item.line_start,
                line_end: h.item.line_end,
                summary: h.item.summary,
                content: h.item.content,
                match_type: h.match_type,
                library: h.library,
            }
        })
        .collect())
}

/// Where embeddings stand: whether they are on, the model, how many items
/// have a vector in its space, and the last failure.
#[derive(Serialize)]
pub struct EmbeddingStatus {
    enabled: bool,
    model: String,
    embedded: i64,
    /// Items the endpoint refused; they are not retried in this space.
    failed: i64,
    total: i64,
    error: String,
}

#[tauri::command(async)]
pub fn knowledge_embedding_status(state: State<'_, AppState>) -> Result<EmbeddingStatus, String> {
    let ep = embed::endpoint(&state);
    let sig = ep.as_ref().map(|e| e.signature()).unwrap_or_default();
    let (embedded, failed, total) = state.library.db.embedded(&sig).map_err(|e| e.to_string())?;
    Ok(EmbeddingStatus {
        enabled: ep.is_some(),
        model: ep.map(|e| e.model).unwrap_or_default(),
        embedded,
        failed,
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
    let v = embed::embed(&ep, &["Divixi knowledge library 지식 라이브러리".to_string()]).await.map_err(|e| e.message)?;
    Ok(v.first().map(Vec::len).unwrap_or(0))
}

/// Start embedding now (the settings just changed).
#[tauri::command]
pub fn knowledge_embed_now(state: State<'_, AppState>) {
    // Items refused before are tried again: the settings may be what changed.
    if let Err(err) = state.library.db.retry_refused() {
        tracing::warn!(%err, "could not reset refused embeddings");
    }
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

// ----- the knowledge graph's conversation -----

/// The library's two read-only tools, `knowledge_search` and
/// `knowledge_list_sources`: the conductor's, and the graph conversation's.
pub fn library_tools(app: AppHandle) -> Vec<orchestra_mcp::Tool> {
    use orchestra_mcp::Tool;
    use serde_json::{json, Value};
    let search = app.clone();
    let list = app;
    vec![
        Tool::new(
            "knowledge_search",
            "Search the human's knowledge library: documents they chose to add, split into sections, each with a title, a summary and the entities it names. Call it when the human asks what we know about something, refers to their docs or notes or to a stored document by name, or when a task you are about to delegate touches a topic the library covers (knowledge_list_sources shows the topics). Do NOT call it for general coding questions, file operations, debugging, or anything the working folder or the conversation already answers. Matching is by keyword, by entity and, when embeddings are set up, by meaning: use the distinctive words a document would contain, and try other wording once if nothing comes back. Workers cannot search the library; pass them what they need.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to find in the documents." },
                    "limit": { "type": "integer", "description": "Max results (default 3, max 5). One extra may be added when the best keyword match would otherwise be dropped.", "default": 3 },
                    "source_id": { "type": "string", "description": "Optional source id (from knowledge_list_sources) to search one document." },
                    "library": { "type": "string", "description": "Optional library name (from knowledge_list_sources) to search one library. Every library is searched when it is left out, which is usually what you want." }
                },
                "required": ["query"]
            }),
            move |args| {
                let app = search.clone();
                async move {
                    let query = crate::conductor::str_arg(&args, "query")?;
                    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(3).clamp(1, 5) as usize;
                    let source = args.get("source_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                    let library_name = args.get("library").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                    let vector = crate::knowledge::query_vector(&app, &query).await;
                    let db = app.state::<AppState>().library.db.clone();
                    if let Some(id) = &source {
                        if db.source(id).map_err(|e| e.to_string())?.is_none() {
                            return Err(format!("No knowledge source with id {id}. Call knowledge_list_sources to see the valid ids."));
                        }
                    }
                    let library = match &library_name {
                        Some(name) => match find_library(&db, name).map_err(|e| e.to_string())? {
                            Some(id) => Some(id),
                            None => return Err(format!("No knowledge library named {name}. Call knowledge_list_sources to see the libraries, or leave library out to search them all.")),
                        },
                        None => None,
                    };
                    let (hits, libraries) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                        let hits = db.search_in(&query, limit, source.as_deref(), library, vector.as_ref().map(|(v, s)| (v.as_slice(), s.as_str())))?;
                        Ok((hits, db.libraries()?.len()))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                    Ok(Value::String(orchestra_knowledge::format_hits(&hits, libraries)))
                }
            },
        ),
        Tool::new(
            "knowledge_list_sources",
            "What is in the human's knowledge library: counts, then one line per document with its id, item count, sync status and topic, grouped by library when the human keeps several. Read-only. Use it to see which topics the library covers and to find a source_id or a library for knowledge_search.",
            json!({ "type": "object", "properties": {} }),
            move |_args| {
                let app = list.clone();
                async move {
                    let db = app.state::<AppState>().library.db.clone();
                    let (sources, stats, libraries) = tokio::task::spawn_blocking(move || Ok::<_, anyhow::Error>((db.sources()?, db.stats()?, db.libraries()?)))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    Ok(Value::String(orchestra_knowledge::format_sources(&sources, &stats, &libraries)))
                }
            },
        ),
    ]
}

/// What the person may call the General library while it keeps its stored
/// name: the UI shows it in their language (`kb.lib.general`).
const GENERAL_SHOWN_AS: [&str; 1] = ["일반"];

/// A library by the name an agent gives: as stored (case aside), or General
/// by the name the person sees it under.
fn find_library(db: &KnowledgeDb, name: &str) -> anyhow::Result<Option<i64>> {
    if let Some(l) = db.library_named(name)? {
        return Ok(Some(l.id));
    }
    if GENERAL_SHOWN_AS.contains(&name.trim()) {
        if let Some(g) = db.library(GENERAL)? {
            if g.name == orchestra_knowledge::store::GENERAL_NAME {
                return Ok(Some(GENERAL));
            }
        }
    }
    Ok(None)
}

/// Most entities and passages one message carries; past these the picks are cut.
pub const MAX_PICKED_ENTITIES: usize = 80;
pub const MAX_PICKED_PASSAGES: usize = 12;
/// Characters of a picked passage that go with the message.
const PASSAGE_CHARS: usize = 1500;

/// What the graph view picked, as the composer sends it: `e:<id>` for an
/// entity, `i:<id>` for a passage. Anything else (the library, `l:<id>`, see
/// [`picked_library`]) is left out here; each list keeps its order, without
/// repeats, cut at its limit.
pub fn parse_picks(selected: &[String]) -> (Vec<i64>, Vec<i64>) {
    let (mut entities, mut items) = (Vec::new(), Vec::new());
    for s in selected {
        let (list, limit) = match s.split_once(':') {
            Some(("e", _)) => (&mut entities, MAX_PICKED_ENTITIES),
            Some(("i", _)) => (&mut items, MAX_PICKED_PASSAGES),
            _ => continue,
        };
        let Some(id) = s.split_once(':').and_then(|(_, n)| n.trim().parse::<i64>().ok()) else { continue };
        if list.len() < limit && !list.contains(&id) {
            list.push(id);
        }
    }
    (entities, items)
}

/// The library the knowledge page shows, as the composer sends it (`l:<id>`);
/// none when the page shows every library.
pub fn picked_library(selected: &[String]) -> Option<i64> {
    selected.iter().find_map(|s| s.strip_prefix("l:").and_then(|n| n.trim().parse::<i64>().ok()))
}

/// The line that tells the graph conversation's agent which library is on screen.
pub fn library_line(name: &str) -> String {
    format!("[library on screen: {name} — knowledge_search with library \"{name}\" searches it alone]")
}

/// The text that goes with a graph conversation's message: the library on
/// screen, when the page shows one, then what was picked.
pub fn graph_context(db: &KnowledgeDb, selected: &[String]) -> Result<String, String> {
    let (entities, items) = parse_picks(selected);
    let picked = db.selection_context(&entities, &items, PASSAGE_CHARS).map_err(|e| e.to_string())?;
    let library = match picked_library(selected) {
        Some(id) => db.library(id).map_err(|e| e.to_string())?,
        None => None,
    };
    Ok(match library {
        Some(l) if picked.is_empty() => library_line(&l.name),
        Some(l) => format!("{}\n{picked}", library_line(&l.name)),
        None => picked,
    })
}

/// What the graph conversation's agent is told once, at the start of its session.
pub fn graph_preamble(lang: &str) -> String {
    if lang == "ko" {
        return "당신은 DIVIXI 지식 라이브러리의 지식 에이전트입니다. 사람은 라이브러리 문서에서 뽑은 엔티티 그래프를 보면서, 그중 일부(엔티티, 그 사이 관계, 엔티티를 언급하는 구절)를 골라 질문합니다. 고른 것은 메시지 끝의 [selection from the knowledge graph] 블록으로 옵니다. 사람이 라이브러리 하나를 보고 있으면 [library on screen: …] 줄이 함께 옵니다.\n\n- 고른 것에 근거해 답하고, 근거가 된 문서(파일 이름)를 밝힙니다.\n- 고른 것만으로 부족하면 knowledge_search로 라이브러리를 더 찾습니다(무엇이 있는지는 knowledge_list_sources). 기본은 모든 라이브러리이고, 보고 있는 라이브러리만 찾으려면 library에 그 이름을 줍니다. 그래도 없으면 없다고 말합니다. 추측은 추측이라고 밝힙니다.\n- 라이브러리는 읽기만 합니다. 파일을 고치거나 명령을 실행하지 않습니다.\n- 짧고 분명하게, 사람이 쓴 언어로 답합니다.".to_string();
    }
    "You are the knowledge agent of a DIVIXI knowledge library. The human is looking at the entity graph drawn from the library's documents, picks part of it (entities, the relations among them, passages that mention them) and asks about it. What was picked comes at the end of the message, in a [selection from the knowledge graph] block; when the human is looking at one library, a [library on screen: …] line comes with it.\n\n- Answer from what was picked, and name the documents (file names) the answer rests on.\n- When the pick is not enough, search the library with knowledge_search (knowledge_list_sources shows what is there). It searches every library; give library the name on screen to search that one alone. If it is not there either, say so; say when something is a guess.\n- The library is read only to you: do not edit files or run commands.\n- Be brief and plain, in the language the human writes in.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_picks_are_entities_and_passages_in_order_without_repeats() {
        let picks: Vec<String> = ["e:3", "i:10", "e:1", "e:3", "x:5", "e:", "e:abc", "i:11", "nonsense"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_picks(&picks), (vec![3, 1], vec![10, 11]));
        let many: Vec<String> = (0..200).map(|i| format!("e:{i}")).chain((0..50).map(|i| format!("i:{i}"))).collect();
        let (e, i) = parse_picks(&many);
        assert_eq!((e.len(), i.len()), (MAX_PICKED_ENTITIES, MAX_PICKED_PASSAGES), "cut at the limits");
        assert_eq!(parse_picks(&[]), (vec![], vec![]));
    }

    #[test]
    fn an_agent_finds_general_by_the_name_the_person_sees() {
        let db = KnowledgeDb::in_memory().unwrap();
        let work = db.create_library("Work").unwrap().id;
        assert_eq!(find_library(&db, "work").unwrap(), Some(work));
        assert_eq!(find_library(&db, "General").unwrap(), Some(GENERAL));
        assert_eq!(find_library(&db, " 일반 ").unwrap(), Some(GENERAL));
        assert_eq!(find_library(&db, "Elsewhere").unwrap(), None);
        db.rename_library(GENERAL, "Inbox").unwrap();
        assert_eq!(find_library(&db, "일반").unwrap(), None, "renamed: only its own name");
        assert_eq!(find_library(&db, "inbox").unwrap(), Some(GENERAL));
    }

    #[test]
    fn the_library_on_screen_rides_with_the_picks() {
        let picks: Vec<String> = ["e:3", "l:2", "i:10", "l:5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(picked_library(&picks), Some(2), "the first one");
        assert_eq!(parse_picks(&picks), (vec![3], vec![10]), "not an entity or a passage");
        assert_eq!(picked_library(&["e:1".to_string(), "l:x".to_string()]), None, "every library");
        let line = library_line("Work");
        assert!(line.starts_with("[library on screen: Work") && line.contains("library \"Work\""), "{line}");
    }

    #[test]
    fn the_knowledge_agent_is_told_where_the_pick_is_and_what_it_may_do() {
        for lang in ["en", "ko"] {
            let p = graph_preamble(lang);
            assert!(p.contains("knowledge agent") || p.contains("지식 에이전트"), "{lang}: its name");
            assert!(p.contains("[selection from the knowledge graph]"), "{lang}");
            assert!(p.contains("knowledge_search") && p.contains("knowledge_list_sources"), "{lang}");
            assert!(p.contains("[library on screen:"), "{lang}");
        }
        assert_ne!(graph_preamble("en"), graph_preamble("ko"));
    }
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
