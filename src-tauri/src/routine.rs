//! Routines: a track's saved instructions, run again whenever they are wanted.
//!
//! The work a human comes back to — check the dependencies, sweep the
//! stale branches, read the error log — is dictated to the conductor once
//! and then dictated again, in slightly different words, every time after.
//! A routine is that instruction written down: a name, what to do, and the
//! worker it runs as. Pressing its button is the whole feature.
//!
//! **Not a schedule.** Nothing here fires on a clock. A routine runs when a
//! human presses it or when the conductor calls `run_routine`, and at no
//! other moment. Divixi is a desktop app: a timer that only ticks while the
//! window is open would be a promise it could not keep, and a routine
//! nobody noticed had stopped running is worse than one that never claimed
//! to.
//!
//! **Nothing here executes anything.** A routine's run is an ordinary
//! worker turn: [`conductor::start_worker_turn`] opens the worker in the
//! track's folder, the report crosses the same membrane, and the track's
//! conductor is handed it exactly as if it had called `spawn_worker`
//! itself. The store's `routine_runs` is a note beside the run saying which
//! routine it came from — the run itself belongs to the track, and stays
//! there when the routine is deleted. That is why the timeline, the worker
//! view, the report card and `read_report` all work on a routine's runs
//! without knowing routines exist.
//!
//! **Every run starts fresh.** A routine is the same work done twice, so
//! the second time must not read the first time's conversation: an agent
//! told "check the dependencies" for the fourth time would otherwise answer
//! from three weeks of context about the first three. [`run`] closes an
//! idle session before opening the next, and passes `fresh`.

use orchestra_store::Routine;
use serde_json::Value;

use crate::conductor::{self, worker_key};
use crate::{AppHandle, AppState};
use tauri::Manager;

/// Start a routine's worker and note the run against it.
///
/// Returns what `spawn_worker` returns — the run id and the worker's
/// standing — so a caller can say what it started. The report arrives later,
/// at the track's conductor, by the ordinary path.
pub async fn run(app: AppHandle, id: &str) -> Result<Value, String> {
    let (routine, track, worker, agent, instruction) = {
        let state = app.state::<AppState>();
        let r = read(&state, id)?;
        let agent = (!r.agent.is_empty()).then(|| r.agent.clone());
        (r.id.clone(), r.track.clone(), r.worker.clone(), agent, r.instruction.clone())
    };

    // Refuse in the routine's own words. `start_worker_turn` refuses a busy
    // worker too, but it answers the conductor: it names `[worker-report]`
    // and tells the reader to wait for one, which is an instruction to an
    // agent, not to the human looking at a routine's button.
    let key = worker_key(&track, &worker);
    {
        let state = app.state::<AppState>();
        let mut workers = state.sessions.workers.lock().await;
        match workers.get(&key) {
            Some(live) if live.running.is_some() => {
                return Err(format!("'{}' is already running. Wait for it to report.", read(&state, id)?.name));
            }
            // An idle session from the last run is closed before this one
            // opens, for the two reasons in the module note: `spawn_worker`
            // refuses a name that is already open, and a routine's runs must
            // not accumulate one another's conversation. The record and the
            // worker's folder stay — only the live session goes.
            Some(_) => {
                workers.remove(&key);
            }
            None => {}
        }
    }

    let started = conductor::start_worker_turn(
        app.clone(),
        track,
        worker,
        instruction,
        agent,
        true,  // open: a routine always opens its worker anew
        true,  // fresh: without the last run's conversation
    )
    .await?;

    // The run exists from the moment the turn was claimed, so the link is
    // written now rather than when the worker reports: a run the app loses
    // track of halfway is still visibly this routine's.
    if let Some(run) = started.get("run").and_then(Value::as_str) {
        let state = app.state::<AppState>();
        if let Err(err) = state.store.link_routine_run(&routine, run) {
            // The worker is already working. Losing the note costs the run
            // its place in this routine's history, which is not worth
            // failing a call that has already succeeded.
            tracing::warn!(%err, routine = %routine, %run, "could not note the run against its routine");
        }
        let _ = notify(&app);
    }
    Ok(started)
}

/// One routine, or why it is not there.
fn read(state: &AppState, id: &str) -> Result<Routine, String> {
    state
        .store
        .routine(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no routine {id}"))
}

/// Tell the window a routine's list has moved.
pub fn notify(app: &AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    app.emit("routines", ()).map_err(|e| e.to_string())
}
