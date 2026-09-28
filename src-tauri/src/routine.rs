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
//! window is open would be a promise it could not keep.
//!
//! **Not a worker, and not in the runs table.** A routine opens an agent
//! session of its own, in its own folder, on its own agent, and writes what
//! happened into `routine_runs`. An earlier version kept those rows in
//! `runs` under a made-up track key, reasoning that nothing would look for
//! them there. Something did: a routine ran on the agent a track used, and
//! its work turned up in that track's history. "No query will find it" is
//! not a property that can be held by being careful — the only way to be
//! sure is for the rows not to be there. So this module uses no `pump`, no
//! `begin_run`, no `events` and no `runs_fts`, and the store test
//! `a_routines_run_is_invisible_to_every_track_query` keeps it that way.
//!
//! What it does share is the layer below `worker`: opening an ACP session,
//! and the report block. Those are how divixi talks to an agent, not what a
//! worker is.
//!
//! **Every run is a new session.** A routine is the same work done twice, so
//! the second time must not read the first time's conversation. There is no
//! resume and nothing is kept between runs: the session is opened for the
//! turn and let go when it ends.
//!
//! **A finished run rings the bell.** The human hears about it in the app,
//! at the top right, beside the decisions that want them. Nothing is sent to
//! a conductor: a routine belongs to no track, and a report arriving in a
//! conversation that had no part in it was more to explain than it was
//! worth.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orchestra_acp::AgentSession;
use orchestra_core::report::{self, Report};
use orchestra_core::AgentEvent;
use orchestra_store::{Routine, RoutineStatus};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::conductor::session_options;
use crate::{AppHandle, AppState};

/// A routine's turn in flight.
struct Live {
    /// The run it is writing into.
    run: String,
    session: Arc<AgentSession>,
}

/// Every routine with a turn in flight, by routine id.
///
/// A routine keeps nothing between runs — see the module note — so an entry
/// here IS "this routine is running". The session is held only so the human
/// can answer what its agent asks and stop it part-way; it is let go the
/// moment the turn ends.
#[derive(Default)]
pub struct Routines {
    live: Mutex<HashMap<String, Live>>,
    /// Runs a person stopped by hand, so the record says `stopped` and not
    /// `done`. Read once, when the run closes.
    stopped: parking_lot::Mutex<HashSet<String>>,
}

impl Routines {
    /// The routines running right now.
    pub async fn running(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }
}

/// What a finished run tells the window, so the bell can say so.
#[derive(Clone, serde::Serialize)]
pub struct Finished {
    pub routine: String,
    pub name: String,
    pub run: String,
    pub status: &'static str,
    /// One line of what it found, for the notice's body.
    pub summary: String,
}

/// Run a routine now: open its agent in its folder, give it its
/// instruction, and write what happens into the routine's own history.
///
/// Returns as soon as the agent has the work, with the run id.
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

/// Open the session and drive the turn.
async fn start(app: AppHandle, r: &Routine) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let spec = state.spec_for(&r.agent)?;
    // No MCP server: a routine has no divixi tools to offer. It does work in
    // a folder and says what it did.
    let opts = session_options(&state, &r.agent, &r.cwd, &r.config, None);
    // Deliberately no `opts.resume`: see the module note. Each run is new.
    tracing::info!(routine = %r.id, agent = %r.agent, cwd = %r.cwd, "opening routine session");
    let session = Arc::new(AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?);

    let run = state
        .store
        .begin_routine_run(&r.id, &r.agent, &r.cwd, &r.instruction)
        .map_err(|e| e.to_string())?;

    // Held for the turn: what the human answers and stops.
    state
        .routines
        .live
        .lock()
        .await
        .insert(r.id.clone(), Live { run: run.clone(), session: session.clone() });

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    // The routine's own event reader. Not `pump`: that writes a run row and
    // an event log, which is the table a routine must stay out of.
    let collector = tauri::async_runtime::spawn(collect(app.clone(), run.clone(), rx));

    // The report rules are the same ones a worker is given: they describe
    // the block, not the worker.
    let sent = format!("{}\n\n---\n{}", r.instruction, report::instructions());

    let (app_t, id_t, run_t, name_t) = (app.clone(), r.id.clone(), run.clone(), r.name.clone());
    tauri::async_runtime::spawn(async move {
        let failed = session.prompt_with(sent, Vec::new(), tx.clone()).await.err().map(|e| e.to_string());
        drop(tx); // ends the collector
        // Let go here: the session lives for the turn and no longer.
        drop(session);

        let (text, tools) = collector.await.unwrap_or_default();
        let st = app_t.state::<AppState>();
        st.routines.live.lock().await.remove(&id_t);
        let stopped = st.routines.stopped.lock().remove(&run_t);

        let status = match (&failed, stopped) {
            // A stop is a person's doing, so it is read before a failure:
            // the agent reporting a cancelled turn is the stop arriving, not
            // something going wrong.
            (_, true) => RoutineStatus::Stopped,
            (Some(_), false) => RoutineStatus::Failed,
            (None, false) => RoutineStatus::Done,
        };
        // A report is only taken from a run that finished by itself. One
        // that failed or was cut short has nothing to say about work it did
        // not do, and a made-up "partial" would read as a finding.
        let report = (status == RoutineStatus::Done).then(|| checked(&text));
        let as_json = report.as_ref().and_then(|r| serde_json::to_value(r).ok());
        if let Err(err) = st.store.end_routine_run(
            &run_t,
            status,
            &text,
            failed.as_deref().unwrap_or_default(),
            tools,
            as_json.as_ref(),
        ) {
            tracing::warn!(%err, run = %run_t, "could not close a routine run");
        }

        let _ = app_t.emit(
            "routine_done",
            Finished {
                routine: id_t,
                name: name_t,
                run: run_t,
                status: status.as_str(),
                summary: report.map(|r| r.summary).unwrap_or_else(|| failed.unwrap_or_default()),
            },
        );
        let _ = app_t.emit("routines", ());
    });

    Ok(json!({ "routine": r.id, "run": run, "agent": r.agent, "cwd": r.cwd, "status": "running" }))
}

/// Read the turn's events for the two things a routine's history keeps: what
/// the agent said, and how many tools it used. Everything else an agent
/// streams — its thoughts, each tool's title — belongs to a transcript, and
/// a routine has none to belong to.
async fn collect(app: AppHandle, run: String, mut rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>) -> (String, u32) {
    let mut text = String::new();
    let mut tools = 0u32;
    while let Some(event) = rx.recv().await {
        match &event {
            AgentEvent::Message { text: chunk } => text.push_str(chunk),
            AgentEvent::ToolCall { .. } => tools += 1,
            // An agent asking before it acts: a routine has no conductor, so
            // it goes to the human on a card.
            AgentEvent::Permission { .. } => {
                crate::conductor::route_permission(&app, &ask_key(&run), ASK_SESSION, &run, &event);
            }
            _ => {}
        }
    }
    (text, tools)
}

/// The "track" a routine's permission card is filed under. A card needs one
/// and a routine has none, so this names the RUN: answering reaches the
/// session that is waiting, and a card left over from a finished run cannot
/// be mistaken for a live one.
pub fn ask_key(run: &str) -> String {
    format!("routine:{run}")
}

/// The session name a routine's permission card records.
pub const ASK_SESSION: &str = "routine";

/// The report a finished run left, checked. A reply without a usable block
/// is not asked again for one — there is no conversation to ask in, the
/// session is gone — so it falls back to what it said.
fn checked(reply: &str) -> Report {
    report::parse(reply).unwrap_or_else(|why| Report::unstructured(true, reply, &why))
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
pub async fn answer_permission(app: &AppHandle, run: &str, request: &str, option: Option<&str>) -> Result<(), String> {
    let session = {
        let state = app.state::<AppState>();
        let live = state.routines.live.lock().await;
        live.values().find(|l| l.run == run).map(|l| l.session.clone())
    };
    let session = session.ok_or("the routine has finished; the question went with it")?;
    session.answer_permission(request, option).map_err(|e| e.to_string())
}

/// Stop a routine's turn in flight.
pub async fn cancel(app: &AppHandle, id: &str) {
    // Cloned out of the map rather than cancelled under its guard: the
    // guard borrows the state, which this function owns and drops first.
    let found = {
        let state = app.state::<AppState>();
        let live = state.routines.live.lock().await;
        live.get(id).map(|l| (l.run.clone(), l.session.clone()))
    };
    if let Some((run, session)) = found {
        // Marked before the cancel lands, so the closing arm cannot read the
        // set before this wrote it.
        app.state::<AppState>().routines.stopped.lock().insert(run);
        session.cancel();
    }
}

/// Tell the window a routine's list has moved.
pub fn notify(app: &AppHandle) -> Result<(), String> {
    app.emit("routines", ()).map_err(|e| e.to_string())
}
