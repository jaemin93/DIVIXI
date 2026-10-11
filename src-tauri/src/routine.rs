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
    /// The plan it was given (its saved version), when it was given one.
    plan: Option<Value>,
    /// What its agent said so far of each step, through `plan_step`.
    progress: Progress,
}

/// Each step's latest word from a run's agent: `started`, `done`, `failed`
/// or `skipped`, and a short note.
type Progress = Arc<parking_lot::Mutex<std::collections::BTreeMap<String, (String, String)>>>;

/// What the window hears as a run's agent says where it is in its plan.
#[derive(Clone, serde::Serialize)]
pub struct StepProgress {
    pub routine: String,
    pub run: String,
    pub step: String,
    pub status: String,
    pub note: String,
}

/// The run of a routine in flight, as the plan on screen shows it: the run,
/// the plan it was given, and where its agent said it is. None when the
/// routine is not running.
pub async fn live(app: &AppHandle, id: &str) -> Option<Value> {
    let state = app.state::<AppState>();
    let live = state.routines.live.lock().await;
    let l = live.get(id)?;
    let progress: serde_json::Map<String, Value> =
        l.progress.lock().iter().map(|(k, (status, note))| (k.clone(), json!({ "status": status, "note": note }))).collect();
    Some(json!({ "run": l.run, "plan": l.plan, "progress": progress }))
}

/// The one tool a run given a plan has: saying a step has started, and how
/// it ended. That is what the plan on screen shows while the run goes; the
/// report at the end still has the last word.
fn step_tool(app: AppHandle, routine: String, run: String, ids: Vec<String>, progress: Progress) -> orchestra_mcp::Tool {
    orchestra_mcp::Tool::new(
        crate::plan::PLAN_STEP,
        "Say where you are in the plan: call it with status \"started\" as you begin a step, and \"done\", \"failed\" or \"skipped\" when it ends. The human watches the plan move with these.",
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The step's id, as in the plan (s1, s2...)." },
                "status": { "type": "string", "enum": ["started", "done", "failed", "skipped"] },
                "note": { "type": "string", "description": "One short line, optional." }
            },
            "required": ["id", "status"],
            "additionalProperties": false
        }),
        move |args| {
            let (app, routine, run, ids, progress) = (app.clone(), routine.clone(), run.clone(), ids.clone(), progress.clone());
            async move {
                let step = args.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                let status = args.get("status").and_then(Value::as_str).unwrap_or_default().to_string();
                if !ids.contains(&step) {
                    return Err(format!("no step {step} in this plan; the steps are {}", ids.join(", ")));
                }
                if !matches!(status.as_str(), "started" | "done" | "failed" | "skipped") {
                    return Err("status is started, done, failed or skipped".to_string());
                }
                let note: String = args.get("note").and_then(Value::as_str).unwrap_or_default().chars().take(300).collect();
                progress.lock().insert(step.clone(), (status.clone(), note.clone()));
                let _ = app.emit("routine_progress", StepProgress { routine, run, step, status, note });
                Ok(json!({ "ok": true }))
            }
        },
    )
}

/// Step results from what the agent said through `plan_step`, for a run
/// whose report left none: the steps that ended, in plan order.
fn results_from(progress: &Progress, ids: &[String]) -> Option<Value> {
    let p = progress.lock();
    let out: Vec<Value> = ids
        .iter()
        .filter_map(|id| p.get(id).filter(|(s, _)| s != "started").map(|(s, n)| json!({ "id": id, "status": s, "note": n })))
        .collect();
    (!out.is_empty()).then(|| Value::Array(out))
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
    /// Routines claimed by a run that is still opening its agent: between
    /// the check and the `live` entry there are awaits, and a second Run
    /// (the phone's, the conductor's, a double press) must not slip in.
    starting: parking_lot::Mutex<HashSet<String>>,
    /// Runs a person stopped by hand, so the record says `stopped` and not
    /// `done`. Read once, when the run closes.
    stopped: parking_lot::Mutex<HashSet<String>>,
}

impl Routines {
    /// The routines running right now, those still opening their agent too.
    pub async fn running(&self) -> Vec<String> {
        let live = self.live.lock().await;
        let mut out: Vec<String> = live.keys().cloned().collect();
        out.extend(self.starting.lock().iter().filter(|id| !live.contains_key(*id)).cloned());
        out
    }
}

/// Claim a routine for a run about to start: false when it is running, or
/// another run is already starting it.
fn claim(starting: &mut HashSet<String>, running: bool, id: &str) -> bool {
    !running && starting.insert(id.to_string())
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
    // worker's reasons, because there is no worker to be busy. The check and
    // the claim are one step, under the lock the running list is read under.
    {
        let state = app.state::<AppState>();
        let live = state.routines.live.lock().await;
        if !claim(&mut state.routines.starting.lock(), live.contains_key(&r.id), &r.id) {
            return Err(format!("'{}' is already running. Wait for it to finish.", r.name));
        }
    }

    let outcome = start(app.clone(), &r).await;
    // Started (it is in `live` now) or not: the claim is done either way.
    app.state::<AppState>().routines.starting.lock().remove(&r.id);
    let _ = notify(&app);
    outcome
}

/// Open the session and drive the turn.
async fn start(app: AppHandle, r: &Routine) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let spec = state.spec_for(&r.agent)?;

    // Its plan, when it has a saved version: the last one, given in the same
    // prompt and kept with the run.
    let guided = guidance(&state, r);
    let ids: Vec<String> = guided.as_ref().map(|g| g.ids.clone()).unwrap_or_default();
    let progress: Progress = Default::default();
    // The run's id comes before the session, so the plan's one tool can name
    // it; a session that does not open leaves the run failed, as any other.
    let run = state
        .store
        .begin_routine_run(&r.id, &r.agent, &r.cwd, &r.instruction)
        .map_err(|e| e.to_string())?;

    // A run without a plan has no divixi tools: it does work in a folder and
    // says what it did. A run given a plan has one, `plan_step`, to say
    // where it is in it.
    let mcp = match &guided {
        Some(_) => {
            let tool = step_tool(app.clone(), r.id.clone(), run.clone(), ids.clone(), progress.clone());
            match orchestra_mcp::McpServer::start("divixi", vec![tool]).await {
                Ok(server) => Some(server),
                Err(err) => {
                    tracing::warn!(%err, routine = %r.id, "running without step progress: its tool did not start");
                    None
                }
            }
        }
        None => None,
    };
    let opts = session_options(&state, &r.agent, &r.cwd, &r.config, mcp.as_ref());
    // Deliberately no `opts.resume`: see the module note. Each run is new.
    tracing::info!(routine = %r.id, agent = %r.agent, cwd = %r.cwd, "opening routine session");
    let session = match AgentSession::open(&spec, opts).await {
        Ok(s) => Arc::new(s),
        Err(err) => {
            let why = err.to_string();
            let _ = state.store.end_routine_run(&run, RoutineStatus::Failed, "", &why, 0, None);
            return Err(why);
        }
    };

    if let Some(g) = &guided {
        if let Err(err) = state.store.set_routine_run_plan(&run, &g.snapshot) {
            tracing::warn!(%err, %run, "could not keep the plan a routine run was given");
        }
    }

    // Held for the turn: what the human answers and stops, and where the plan is.
    state.routines.live.lock().await.insert(
        r.id.clone(),
        Live { run: run.clone(), session: session.clone(), plan: guided.as_ref().map(|g| g.snapshot.clone()), progress: progress.clone() },
    );
    if guided.is_some() {
        let _ = app.emit("routine_progress", StepProgress { routine: r.id.clone(), run: run.clone(), step: String::new(), status: "begun".into(), note: String::new() });
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    // The routine's own event reader. Not `pump`: that writes a run row and
    // an event log, which is the table a routine must stay out of.
    let collector = tauri::async_runtime::spawn(collect(app.clone(), run.clone(), rx));

    // The report rules are the same ones a worker is given: they describe
    // the block, not the worker.
    let sent = prompt(&r.instruction, guided.as_ref().map(|g| g.section.as_str()));

    let (app_t, id_t, run_t, name_t) = (app.clone(), r.id.clone(), run.clone(), r.name.clone());
    tauri::async_runtime::spawn(async move {
        let failed = session.prompt_with(sent, Vec::new(), tx.clone()).await.err().map(|e| e.to_string());
        drop(tx); // ends the collector
        // Let go here: the session lives for the turn and no longer, and
        // the plan's tool with it.
        drop(session);
        drop(mcp);

        let (text, tools) = collector.await.unwrap_or_default();
        let st = app_t.state::<AppState>();
        {
            // This run's entry only: never another run's of the same routine.
            let mut live = st.routines.live.lock().await;
            if live.get(&id_t).is_some_and(|l| l.run == run_t) {
                live.remove(&id_t);
            }
        }
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
        // What the agent said of each step. A run that finished by itself:
        // its report's list, else what it said step by step. One that failed
        // or was stopped: what it said step by step, which is where it broke.
        if !ids.is_empty() {
            let steps = if status == RoutineStatus::Done {
                crate::plan::step_results(&text, &ids).or_else(|| results_from(&progress, &ids))
            } else {
                results_from(&progress, &ids)
            };
            if let Some(steps) = steps {
                if let Err(err) = st.store.set_routine_run_steps(&run_t, &steps) {
                    tracing::warn!(%err, run = %run_t, "could not keep a routine run's step results");
                }
            }
        }
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

/// What a routine's session is sent: its instruction, its plan when it is
/// given one, and the report rules. Without a plan it is exactly what it
/// always was.
fn prompt(instruction: &str, plan: Option<&str>) -> String {
    match plan {
        None => format!("{}\n\n---\n{}", instruction, report::instructions()),
        Some(plan) => format!("{}\n\n---\n{}\n\n---\n{}", instruction, plan, report::instructions()),
    }
}

/// Why the conductor may not run this routine now (see
/// `plan::run_refusal`), in the conductor's own language. A plan that
/// cannot be read is a refusal too: the run would go without the plan the
/// human sees.
pub fn refusal(app: &AppHandle, id: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let r = state.store.routine(id).ok().flatten()?;
    if r.plan.is_empty() {
        return None;
    }
    let pref = state.store.get_meta(&format!("{}language", crate::SETTING_PREFIX)).ok().flatten();
    let ko = korean(pref.as_deref());
    match crate::plan::doc(&state, &r.plan) {
        Ok(doc) => crate::plan::run_refusal(&doc, ko),
        Err(err) if ko => Some(format!("계획을 읽지 못해 실행하지 않았습니다: {err}")),
        Err(err) => Some(format!("The plan could not be read, so it did not run: {err}")),
    }
}

/// Whether the conductor speaks Korean, as it decides it (conductor.rs):
/// everything but English chosen outright, "system" included.
fn korean(pref: Option<&str>) -> bool {
    pref != Some("en")
}

/// The plan a run is given, if its routine has one to give. A plan that
/// cannot be read is said in the log and the run goes on without it: the
/// instruction alone is still the routine.
fn guidance(state: &AppState, r: &Routine) -> Option<crate::plan::Guidance> {
    if r.plan.is_empty() {
        return None;
    }
    match crate::plan::doc(state, &r.plan) {
        Ok(doc) => crate::plan::guidance(&doc),
        Err(err) => {
            tracing::warn!(%err, routine = %r.id, plan = %r.plan, "running without the plan: it could not be read");
            None
        }
    }
}

/// Read the turn's events for the two things a routine's history keeps: what
/// the agent said, and how many tools it used. Everything else an agent
/// streams — its thoughts, each tool's title — belongs to a transcript, and
/// a routine has none to belong to.
async fn collect(app: AppHandle, run: String, mut rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>) -> (String, u32) {
    // Put together as a track's run is, so a turn reads the same on the
    // Routines page as in a timeline.
    let mut reply = orchestra_core::Reply::default();
    let mut tools = 0u32;
    while let Some(event) = rx.recv().await {
        match &event {
            AgentEvent::Message { text: chunk } => reply.say(chunk),
            AgentEvent::ToolCall { .. } => {
                reply.tool();
                tools += 1
            }
            // An agent asking before it acts: a routine has no conductor, so
            // it goes to the human on a card.
            AgentEvent::Permission { .. } => {
                crate::conductor::route_permission(&app, &ask_key(&run), ASK_SESSION, &run, &event);
            }
            _ => {}
        }
    }
    (reply.into_string(), tools)
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_refusal_speaks_the_conductors_language() {
        assert!(korean(Some("ko")));
        assert!(!korean(Some("en")));
        assert!(korean(Some("system")), "as the conductor reads it: anything but English is Korean");
        assert!(korean(None));
    }

    #[test]
    fn a_routine_is_claimed_once_while_it_starts() {
        let mut starting = HashSet::new();
        assert!(claim(&mut starting, false, "ro1"));
        assert!(!claim(&mut starting, false, "ro1"), "a second Run while the first opens its agent");
        assert!(claim(&mut starting, false, "ro2"), "another routine is its own");
        starting.remove("ro1");
        assert!(!claim(&mut starting, true, "ro1"), "running already");
        assert!(claim(&mut starting, false, "ro1"), "free again once it ended");
    }

    use super::*;

    /// A routine without a plan is sent what it always was, to the byte.
    #[test]
    fn without_a_plan_the_prompt_is_as_before() {
        let before = format!("{}\n\n---\n{}", "list the stale dependencies", report::instructions());
        assert_eq!(prompt("list the stale dependencies", None), before);
    }

    #[test]
    fn with_a_plan_it_sits_between_the_instruction_and_the_report_rules() {
        let sent = prompt("check deps", Some("[plan] Wave 1"));
        let (i, p, r) = (sent.find("check deps").unwrap(), sent.find("[plan]").unwrap(), sent.find(report::FENCE).unwrap());
        assert!(i < p && p < r, "{sent}");
    }

    #[test]
    fn step_results_fall_back_to_what_the_agent_said_step_by_step() {
        let progress: Progress = Default::default();
        let ids: Vec<String> = ["s1", "s2", "s3"].iter().map(|s| s.to_string()).collect();
        assert!(results_from(&progress, &ids).is_none(), "nothing said, nothing kept");
        progress.lock().insert("s2".into(), ("failed".into(), "no network".into()));
        progress.lock().insert("s1".into(), ("done".into(), String::new()));
        progress.lock().insert("s3".into(), ("started".into(), String::new()));
        assert_eq!(
            results_from(&progress, &ids).unwrap(),
            json!([{ "id": "s1", "status": "done", "note": "" }, { "id": "s2", "status": "failed", "note": "no network" }]),
            "in plan order, and a step only started has no result"
        );
    }
}
