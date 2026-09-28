//! Routines: saved work, run on its own whenever it is wanted.
//!
//! The work a human comes back to — check the dependencies, sweep the stale
//! branches, read the error log — is dictated once and then dictated again,
//! in slightly different words, every time after. A routine is that work
//! written down with everything it needs to run alone: what to do, the
//! folder to do it in, and the agent to do it on. Pressing its button is the
//! whole feature.
//!
//! **Not a schedule.** Nothing here fires on a clock. A routine runs when a
//! human presses it or when the conductor calls `run_routine`, and at no
//! other moment. Divixi is a desktop app: a timer that only ticks while the
//! window is open would be a promise it could not keep, and a routine nobody
//! noticed had stopped running is worse than one that never claimed to.
//!
//! **Not a worker.** This is the part that matters, and the part an earlier
//! version of this module got wrong by reusing `start_worker_turn`. A
//! routine opens an agent session of its own, in its own folder, on its own
//! agent. It takes no worker name, holds no place in any track's worker
//! list, inherits no conversation, and cannot cut one short. Its runs are
//! kept under the track key `routine:<id>` — the same trick `artifact` uses
//! — so every query that lists a track's runs or workers passes them by.
//! Nothing downstream of the worker path (a worker folder, a git checkout,
//! an entry in the track's session list, a card in a track's panels) is
//! reached at all.
//!
//! What it does share is the layer below `worker`: the ACP session, the
//! event pump, the run record, the report block. Those are how divixi talks
//! to an agent, not what a worker is.
//!
//! **Every run is a new session.** A routine is the same work done twice, so
//! the second time must not read the first time's conversation. There is no
//! resume and nothing is kept between runs: the session is opened for the
//! turn and dropped when it ends.
//!
//! **The report goes to a track, when one is named.** A routine belongs to
//! no track, but the human hears about everything through a conductor, so a
//! routine may name one to tell. It arrives as `[routine-report]`, never as
//! a worker's — the conductor is told plainly that no worker was involved
//! and that it has nothing to follow up with.

use std::collections::HashMap;
use std::sync::Arc;

use orchestra_acp::AgentSession;
use orchestra_core::report::{self, Report};
use orchestra_core::AgentEvent;
use orchestra_store::{routine_run_key, Routine, ROUTINE_SESSION};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::conductor::{self, session_options};
use crate::{pump, AppHandle, AppState};

/// The session of every routine with a turn in flight, by routine id.
///
/// A routine keeps nothing between runs — see the module note — so an entry
/// here IS "this routine is running". The session is held only so the human
/// can answer what its agent asks and stop it part-way; it is dropped the
/// moment the turn ends.
#[derive(Default)]
pub struct Routines {
    live: Mutex<HashMap<String, Arc<AgentSession>>>,
}

impl Routines {
    /// The routines running right now.
    pub async fn running(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }
}

/// Run a routine now: open its agent in its folder, give it its
/// instruction, and record the turn under the routine's own key.
///
/// Returns as soon as the agent has the work, with the run id. The report
/// follows later — on the routine's own list always, and at a track's
/// conductor when one is named.
pub async fn run(app: AppHandle, id: &str) -> Result<Value, String> {
    let r = {
        let state = app.state::<AppState>();
        read(&state, id)?
    };

    // Said in the routine's own words. Nothing here can refuse for a
    // worker's reasons, because there is no worker to be busy.
    if app.state::<AppState>().routines.live.lock().await.contains_key(&r.id) {
        return Err(format!("'{}' is already running. Wait for it to finish.", r.name));
    }

    let outcome = start(app.clone(), &r).await;
    let _ = notify(&app);
    outcome
}

/// Open the session and drive the turn. Split from [`run`] so the running
/// mark is released on every failing path without repeating it.
async fn start(app: AppHandle, r: &Routine) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let spec = state.spec_for(&r.agent)?;
    // No MCP server: a routine has no divixi tools to offer. It does work in
    // a folder and says what it did.
    let opts = session_options(&state, &r.agent, &r.cwd, &r.config, None);
    // Deliberately no `opts.resume`: see the module note. Each run is new.
    tracing::info!(routine = %r.id, agent = %r.agent, cwd = %r.cwd, "opening routine session");
    let session = Arc::new(AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?);

    let key = routine_run_key(&r.id);
    let run = state
        .store
        .begin_run(&key, ROUTINE_SESSION, &r.agent, &r.instruction, &r.cwd)
        .map_err(|e| e.to_string())?;

    // Held for the turn: what the human answers and stops.
    state.routines.live.lock().await.insert(r.id.clone(), session.clone());

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tauri::async_runtime::spawn(pump(app.clone(), key, ROUTINE_SESSION.to_string(), run.clone(), rx));
    let _ = tx.send(AgentEvent::Started { session_id: session.session_id().to_string(), cwd: r.cwd.clone() });

    // The report rules are the same ones a worker is given: they describe
    // the block, not the worker.
    let sent = format!("{}\n\n---\n{}", r.instruction, report::instructions());

    let (app_t, id_t, run_t, name_t, track_t) =
        (app.clone(), r.id.clone(), run.clone(), r.name.clone(), r.track.clone());
    tauri::async_runtime::spawn(async move {
        let result = session.prompt_with(sent, Vec::new(), tx.clone()).await;
        if let Err(err) = &result {
            let _ = tx.send(AgentEvent::Failed { error: err.to_string() });
        }
        // Let go here: the session lives for the turn and no longer.
        drop(session);
        let st = app_t.state::<AppState>();
        st.routines.live.lock().await.remove(&id_t);
        let _ = app_t.emit("routines", ());
        if !track_t.is_empty() {
            let report = finish(&app_t, &run_t);
            conductor::report_routine_to_conductor(app_t.clone(), track_t, name_t, id_t, run_t, report).await;
        }
    });

    Ok(json!({
        "routine": r.id,
        "run": run,
        "agent": r.agent,
        "cwd": r.cwd,
        "status": "running",
    }))
}

/// The report a finished run left, checked. A reply without a usable block
/// is not asked again for one — there is no conversation to ask in, the
/// session is gone — so it falls back to what it said.
fn finish(app: &AppHandle, run: &str) -> Report {
    let state = app.state::<AppState>();
    let summary = state.store.run(run).ok().flatten();
    let reply = summary.as_ref().map(|s| s.output.clone()).unwrap_or_default();
    match report::parse(&reply) {
        Ok(r) => r,
        // Whether the turn ENDED is not whether it said anything useful, and
        // the made-up report says which: a run that failed is `failed`, one
        // that finished without a block is `partial`.
        Err(why) => {
            let finished = summary.is_some_and(|s| s.status == orchestra_core::RunStatus::Done);
            Report::unstructured(finished, &reply, &why)
        }
    }
}

/// One routine, or why it is not there.
fn read(state: &AppState, id: &str) -> Result<Routine, String> {
    state
        .store
        .routine(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no routine {id}"))
}

/// Answer a permission question a routine's agent is waiting on. Its own
/// agent asks, so the human answers it on a card — there is no conductor of
/// a routine to decide for them.
pub async fn answer_permission(app: &AppHandle, id: &str, request: &str, option: Option<&str>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let live = state.routines.live.lock().await;
    let session = live.get(id).ok_or("the routine has finished; the question went with it")?;
    session.answer_permission(request, option).map_err(|e| e.to_string())
}

/// Stop a routine's turn in flight.
pub async fn cancel(app: &AppHandle, id: &str) {
    // Cloned out of the map rather than cancelled under its guard: the
    // guard borrows the state, which this function owns and drops first.
    let session = {
        let state = app.state::<AppState>();
        let live = state.routines.live.lock().await;
        live.get(id).cloned()
    };
    if let Some(session) = session {
        session.cancel();
    }
}

/// Tell the window a routine's list has moved.
pub fn notify(app: &AppHandle) -> Result<(), String> {
    app.emit("routines", ()).map_err(|e| e.to_string())
}
