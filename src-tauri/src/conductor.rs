//! The conductor and its lanes.
//!
//! A track has one conductor: an agent session at Track level. What it can
//! do to the app arrives as MCP tools (served in-process, see
//! `orchestra_mcp`): open a lane, ask a lane a follow-up, check on it, read
//! a report, record a decision. Whether a human message is a question to
//! answer or work to delegate is the conductor's call, made in its prompt,
//! not in Rust — the core is the mechanism, the agent is the policy.
//!
//! Each track's conductor gets its own MCP server, so every tool call is
//! already scoped to the track that made it; the conductor never names its
//! track and cannot reach another.
//!
//! Lanes are agent sessions too, one per lane name within a track, alive
//! across runs, so a lane is a Claude Code (or Codex, …) session the
//! conductor keeps talking to. Every turn, on the conductor or a lane, is one
//! run in the store. A lane's session id is remembered in the store as well,
//! so a closed lane (closed on purpose, or gone with an app restart) reopens
//! with its conversation when the conductor calls it by name again.
//!
//! Lane work is asynchronous from the conductor's point of view: `spawn_lane`
//! and `ask_lane` return as soon as the lane has the task, and when the lane
//! finishes, Orchestra hands its report to the conductor as a new turn. A
//! blocking tool would trip the agent's own MCP call timeout on any lane
//! that runs for minutes, which real work does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use orchestra_acp::{AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_core::LaneEvent;
use orchestra_mcp::{McpServer, Tool};
use orchestra_store::TrackInfo;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::{pump, AppState};

/// The lane name conductor turns are recorded under.
pub const CONDUCTOR_LANE: &str = "conductor";

/// Prefix of conductor prompts that Orchestra itself injects (lane reports).
/// Language-neutral; the timeline shows these as system lines, not as the
/// human speaking, and the preamble tells the conductor what it means.
pub const REPORT_PREFIX: &str = "[lane-report]";

/// A lane run waits at most this long for the agent to finish a turn.
const LANE_TURN_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// How long a lane report waits for the conductor to become free.
const REPORT_WAIT: Duration = Duration::from_secs(30 * 60);

/// One open agent session and what it runs on.
pub struct Live {
    pub agent: String,
    /// Shared so a turn can run without holding the sessions lock; the
    /// session ends when the last clone drops.
    pub session: Arc<AgentSession>,
    pub turns: u32,
    /// Run id of the turn in flight, if any.
    pub running: Option<String>,
}

/// A track's conductor: its session and the MCP server that is its hands.
pub struct Conductor {
    pub live: Live,
    /// Agent and options the session was opened with; when the track's
    /// differ, the next message reopens it.
    fingerprint: String,
    /// Dropped with the conductor; the server stops then.
    _mcp: McpServer,
}

/// What a session was opened with, compared to decide on reopening.
fn fingerprint(agent: &str, config: &BTreeMap<String, String>) -> String {
    format!("{agent}\n{}", serde_json::to_string(config).unwrap_or_default())
}

/// Conductor and lane sessions, across tracks. Held behind async mutexes
/// because opening a session and running a turn both await.
#[derive(Default)]
pub struct Sessions {
    /// By track id.
    pub conductors: Mutex<HashMap<String, Conductor>>,
    /// By `track/lane` (see [`lane_key`]).
    pub lanes: Mutex<HashMap<String, Live>>,
    /// Tracks whose conductor has a turn in flight.
    busy: parking_lot::Mutex<HashSet<String>>,
}

impl Sessions {
    /// Mark a track's conductor busy; `false` if it already was.
    fn begin_turn(&self, track: &str) -> bool {
        self.busy.lock().insert(track.to_string())
    }

    fn end_turn(&self, track: &str) {
        self.busy.lock().remove(track);
    }

    pub fn is_busy(&self, track: &str) -> bool {
        self.busy.lock().contains(track)
    }

    /// Forget every session of a track (its conductor and lanes). Turns in
    /// flight finish on their own; nothing new starts.
    pub async fn close_track(&self, track: &str) {
        self.conductors.lock().await.remove(track);
        let prefix = lane_key(track, "");
        self.lanes.lock().await.retain(|key, _| !key.starts_with(&prefix));
    }
}

/// Key of a lane in [`Sessions::lanes`] and in the store's memory.
fn lane_key(track: &str, lane: &str) -> String {
    format!("{track}/{lane}")
}

/// What the conductor is told once, at the start of its session, in the
/// interface language. Only the wording differs; the rules are the same.
fn preamble(lang: &str, track: &TrackInfo) -> String {
    let intent = track.intent.trim();
    if lang.starts_with("ko") {
        let about = if intent.is_empty() {
            String::new()
        } else {
            format!("\n이 트랙의 목적: {intent}\n")
        };
        return format!(
            r#"당신은 Orchestra의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 레인(lane)이라는 별도의 에이전트 세션에 맡깁니다.

트랙 이름: {name}{about}
규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 레인을 열지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_lane`으로 레인을 열어 맡깁니다. 레인 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 레인이 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- `spawn_lane`과 `ask_lane`은 레인이 일을 받는 즉시 돌아옵니다. 결과를 기다리지 말고, 사람에게 무엇을 맡겼는지 한 문장으로 알린 뒤 턴을 끝냅니다. 레인이 끝나면 `{REPORT_PREFIX}`로 시작하는 메시지가 당신에게 옵니다. 그때 무슨 일이 있었는지 한두 문단으로 사람에게 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다.
- 같은 레인에 이어서 시킬 일은 `ask_lane`으로 보냅니다. 레인은 이전 대화를 기억합니다.
- 레인 목록은 열린 것과 닫힌 것 모두 `lane_status`로 봅니다. 터미널이나 파일을 뒤져 레인을 찾지 않습니다.
- 닫힌 레인은 같은 이름으로 `spawn_lane`이나 `ask_lane`을 부르면 이전 대화를 기억한 채 다시 열립니다. 기억을 버리고 처음부터 시작하려면 `spawn_lane`에 fresh=true를 줍니다. `close_lane`은 세션만 닫고 기록은 남깁니다.
- 도구가 오류를 돌려주면 오류 문구에 적힌 대로 한 번만 다시 시도하고, 그래도 안 되면 사람에게 무엇이 막혔는지 말합니다. 같은 도구를 반복해서 부르지 않습니다.
- 사람이 결정해야 할 일(되돌리기 어려운 변경, 여러 갈래 중 선택)은 스스로 정하지 말고 선택지를 제시하고 묻습니다. 사람이 정하면 `record_decision`으로 남깁니다.
- 한국어로 말합니다. 짧게, 명확하게.

작업 디렉터리는 {cwd} 입니다. 레인도 같은 디렉터리에서 일합니다.
"#,
            name = track.name,
            cwd = track.cwd,
        );
    }
    let about = if intent.is_empty() {
        String::new()
    } else {
        format!("\nWhat this track is for: {intent}\n")
    };
    format!(
        r#"You are Orchestra's conductor. You are the only one who talks to the human; real work is delegated to lanes, which are separate agent sessions.

Track: {name}{about}
Rules:
- If the human's message is a question or small talk, answer it yourself. Do not open a lane.
- If real work is needed (reading, changing or investigating code), open a lane with `spawn_lane`. Name it short and lowercase (e.g. fix-parser) and make the task specific enough for the lane to finish alone.
- `spawn_lane` and `ask_lane` return as soon as the lane has the task. Do not wait for the result: tell the human in one sentence what you delegated and end your turn. When the lane finishes, a message starting with `{REPORT_PREFIX}` reaches you. Then explain to the human in a paragraph or two what happened. Do not paste the report; give the gist.
- Follow-ups for the same lane go through `ask_lane`; the lane remembers its earlier turns.
- `lane_status` lists every lane, open and closed. Never hunt for lanes through the terminal or files.
- A closed lane reopens with its earlier conversation when you call `spawn_lane` or `ask_lane` with its name. To drop that memory and start over, pass fresh=true to `spawn_lane`. `close_lane` only closes the session; the record stays.
- If a tool returns an error, retry once as the message suggests; if that fails, tell the human what is blocked. Never call the same tool repeatedly.
- Decisions that belong to the human (hard-to-undo changes, a choice between directions) are not yours to make: present the options and ask. Once the human decides, record it with `record_decision`.
- Speak English. Short and clear.

The working directory is {cwd}. Lanes work in the same directory.
"#,
        name = track.name,
        cwd = track.cwd,
    )
}

/// Build the MCP tools that give one track's conductor its hands. Each
/// captures the app handle and the track, and reaches the state through the
/// handle.
pub fn tools(app: AppHandle, track: String) -> Vec<Tool> {
    let spawn = (app.clone(), track.clone());
    let ask = (app.clone(), track.clone());
    let status = (app.clone(), track.clone());
    let report = app.clone();
    let close = (app.clone(), track.clone());
    let decide = (app, track);

    vec![
        Tool::new(
            "spawn_lane",
            &format!("Open a lane (a separate agent session) and give it a task. Returns at once with the run id; the lane works in the background and its report reaches you later as a message starting with {REPORT_PREFIX}. A closed lane with this name is reopened with its earlier conversation (the result says resumed=true); pass fresh=true to start it over without that memory. Use for real work; answer questions yourself instead."),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short lane name, lowercase, e.g. fix-parser. New, or the name of a closed lane to reopen." },
                    "task": { "type": "string", "description": "What the lane should do, specific enough to finish alone." },
                    "agent": { "type": "string", "description": "Agent id to run the lane on: claude_code, codex, copilot, antigravity. Defaults to the lane's earlier agent, else the conductor's." },
                    "fresh": { "type": "boolean", "description": "Start over without the closed lane's earlier conversation. Default false." }
                },
                "required": ["name", "task"]
            }),
            move |args| {
                let (app, track) = spawn.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let task = str_arg(&args, "task")?;
                    let agent = args.get("agent").and_then(Value::as_str).map(str::to_owned);
                    let fresh = args.get("fresh").and_then(Value::as_bool).unwrap_or(false);
                    start_lane_turn(app, track, name, task, agent, true, fresh).await
                }
            },
        ),
        Tool::new(
            "ask_lane",
            &format!("Send a follow-up message to a lane. The lane remembers its earlier turns; a closed lane is reopened with that memory. Returns at once; the lane's answer reaches you later as a {REPORT_PREFIX} message."),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Lane name given to spawn_lane." },
                    "message": { "type": "string" }
                },
                "required": ["name", "message"]
            }),
            move |args| {
                let (app, track) = ask.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let message = str_arg(&args, "message")?;
                    start_lane_turn(app, track, name, message, None, false, false).await
                }
            },
        ),
        Tool::new(
            "lane_status",
            "List every lane of this track, open or closed: agent, whether its session is open, whether a turn is in flight (with its run id), how many runs it has, and its last run with status. A closed lane marked resumable reopens with its memory when you call spawn_lane or ask_lane with its name.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let (app, track) = status.clone();
                async move {
                    let state = app.state::<AppState>();
                    Ok(Value::Array(lane_list(&state, &track).await))
                }
            },
        ),
        Tool::new(
            "read_report",
            "Read a run's report by run id (e.g. t004): status, prompt, output, tools, duration.",
            json!({
                "type": "object",
                "properties": { "run": { "type": "string" } },
                "required": ["run"]
            }),
            move |args| {
                let app = report.clone();
                async move {
                    let run = str_arg(&args, "run")?;
                    let state = app.state::<AppState>();
                    match state.store.run(&run).map_err(|e| e.to_string())? {
                        Some(summary) => serde_json::to_value(summary).map_err(|e| e.to_string()),
                        None => Err(format!("no run {run}")),
                    }
                }
            },
        ),
        Tool::new(
            "close_lane",
            "Close a lane's session. Its runs and its memory stay in the record; spawn_lane or ask_lane with the same name reopens it.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            move |args| {
                let (app, track) = close.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let state = app.state::<AppState>();
                    let removed = state.sessions.lanes.lock().await.remove(&lane_key(&track, &name));
                    match removed {
                        Some(live) => {
                            drop(live);
                            Ok(Value::String(format!("closed {name}")))
                        }
                        None => Err(format!("no open lane {name}")),
                    }
                }
            },
        ),
        Tool::new(
            "record_decision",
            "Record a decision the human made: the question, the options, the choice and the rationale. Use after the human decides, never to decide for them.",
            json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "items": { "type": "string" } },
                    "choice": { "type": "string" },
                    "rationale": { "type": "string" }
                },
                "required": ["question", "choice"]
            }),
            move |args| {
                let (app, track) = decide.clone();
                async move {
                    let state = app.state::<AppState>();
                    let mut list: Vec<Value> = state
                        .store
                        .get_meta("decisions")
                        .map_err(|e| e.to_string())?
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default();
                    let mut entry = args.clone();
                    entry["track"] = json!(track);
                    entry["at"] = json!(now_ms());
                    entry["n"] = json!(list.len() + 1);
                    list.push(entry.clone());
                    state
                        .store
                        .set_meta("decisions", &serde_json::to_string(&list).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({ "recorded": entry["n"] }))
                }
            },
        ),
    ]
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing argument: {key}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What the store remembers about a lane's last agent session, so the lane
/// can be reopened with its conversation.
#[derive(serde::Serialize, serde::Deserialize)]
struct LaneRecord {
    session_id: String,
    agent: String,
}

fn lane_record_key(track: &str, name: &str) -> String {
    format!("lane_session:{}", lane_key(track, name))
}

fn lane_record(state: &AppState, track: &str, name: &str) -> Option<LaneRecord> {
    state
        .store
        .get_meta(&lane_record_key(track, name))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn remember_lane(state: &AppState, track: &str, name: &str, record: &LaneRecord) {
    match serde_json::to_string(record) {
        Ok(json) => {
            if let Err(err) = state.store.set_meta(&lane_record_key(track, name), &json) {
                tracing::warn!(lane = %name, %err, "could not remember lane session id");
            }
        }
        Err(err) => tracing::warn!(lane = %name, %err, "could not encode lane record"),
    }
}

/// Every lane the store knows in a track, merged with what is open right
/// now. The conductor's own lane is not a lane to it.
async fn lane_list(state: &AppState, track: &str) -> Vec<Value> {
    let history = state.store.lanes(track).unwrap_or_else(|err| {
        tracing::warn!(%err, "could not list lanes from the store");
        Vec::new()
    });
    let lanes = state.sessions.lanes.lock().await;
    let mut list: Vec<Value> = history
        .iter()
        .filter(|info| info.name != CONDUCTOR_LANE)
        .map(|info| {
            let live = lanes.get(&lane_key(track, &info.name));
            json!({
                "name": info.name,
                "agent": live.map(|l| l.agent.clone()).unwrap_or_else(|| info.agent.clone()),
                "open": live.is_some(),
                "running": live.map(|l| l.running.is_some()).unwrap_or(false),
                "run": live.and_then(|l| l.running.clone()),
                "runs": info.runs,
                "last_run": info.last_run,
                "last_status": info.last_status.as_str(),
                "last_at": info.last_at,
                "resumable": live.is_none() && lane_record(state, track, &info.name).is_some(),
            })
        })
        .collect();
    // A lane opened so recently that its first run is not in the store yet.
    let prefix = lane_key(track, "");
    for (key, live) in lanes.iter() {
        let Some(name) = key.strip_prefix(&prefix) else { continue };
        if !history.iter().any(|h| h.name == name) {
            list.push(json!({
                "name": name,
                "agent": live.agent,
                "open": true,
                "running": live.running.is_some(),
                "run": live.running,
                "runs": 0,
                "resumable": false,
            }));
        }
    }
    list
}

/// Session options for an agent: working directory, the track's choices
/// for that agent (`option id → value id`), and the conductor's tools when
/// it is the conductor.
///
/// The agent's mode option is sent as `session/set_mode`; every other
/// option goes through `session/set_config_option`. No mode chosen means
/// the most autonomous one the agent offers.
fn session_options(
    state: &AppState,
    agent: &str,
    cwd: &str,
    chosen: &BTreeMap<String, String>,
    mcp: Option<&McpServer>,
) -> SessionOptions {
    let known = state.config_options_for(agent);
    let mut mode = None;
    let mut config = Vec::new();
    for (id, value) in chosen {
        if value.trim().is_empty() {
            continue;
        }
        let is_mode = known.iter().any(|o| o.id == *id && o.category == "mode");
        if is_mode {
            mode = Some(value.clone());
        } else {
            config.push((id.clone(), value.clone()));
        }
    }
    // The library's servers for this agent, plus Orchestra's own tools for
    // a conductor.
    let mut mcp_servers = crate::library::mcp_for_agent(&state.store, agent);
    if let Some(m) = mcp {
        mcp_servers.push(
            McpHttp {
                name: "orchestra".to_string(),
                url: m.url(),
                headers: vec![m.auth_header()],
            }
            .into(),
        );
    }
    SessionOptions {
        cwd: PathBuf::from(cwd),
        mode,
        config,
        mcp_servers,
        resume: None,
    }
}

fn track_info(state: &AppState, track: &str) -> Result<TrackInfo, String> {
    state
        .store
        .track(track)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no track {track}"))
}

/// Give a lane a turn and return at once. `open` is `spawn_lane` (a lane
/// that is already open is refused); `ask_lane` needs the lane to exist,
/// open or in the record. A lane that is not open but has a remembered
/// session is reopened with it, unless `fresh` says to forget. The turn
/// runs in the background; when it ends, its report is handed to the
/// conductor as a new turn.
async fn start_lane_turn(
    app: AppHandle,
    track: String,
    name: String,
    text: String,
    agent: Option<String>,
    open: bool,
    fresh: bool,
) -> Result<Value, String> {
    if name == CONDUCTOR_LANE {
        return Err("that name is reserved".to_string());
    }
    let state = app.state::<AppState>();
    let info = track_info(&state, &track)?;
    let key = lane_key(&track, &name);

    // Resolve or open the lane session; refuse a second turn on a busy lane.
    let (agent_id, turns, session, resumed, note) = {
        let mut lanes = state.sessions.lanes.lock().await;
        match (lanes.get_mut(&key), open) {
            (Some(live), true) => {
                return Err(format!(
                    "lane {name} is already open (agent {}, {} turns). Send follow-ups with ask_lane(name=\"{name}\", message=...), or spawn_lane with a new name.",
                    live.agent, live.turns
                ))
            }
            (Some(live), false) => {
                if let Some(run) = &live.running {
                    return Err(format!(
                        "lane {name} is still working on run {run}. Wait for its {REPORT_PREFIX} before sending more."
                    ));
                }
                live.turns += 1;
                (live.agent.clone(), live.turns, live.session.clone(), false, None)
            }
            (None, _) => {
                let record = if fresh { None } else { lane_record(&state, &track, &name) };
                let history = state.store.lanes(&track).unwrap_or_default();
                let past = history.iter().find(|l| l.name == name);
                if !open && record.is_none() && past.is_none() {
                    let prefix = lane_key(&track, "");
                    let open_names: Vec<&str> = lanes.keys().filter_map(|k| k.strip_prefix(&prefix)).collect();
                    let closed: Vec<&str> = history
                        .iter()
                        .filter(|l| l.name != CONDUCTOR_LANE && !lanes.contains_key(&lane_key(&track, &l.name)))
                        .map(|l| l.name.as_str())
                        .collect();
                    return Err(format!(
                        "no lane {name}. Open lanes: {open_names:?}. Closed lanes: {closed:?}. Use spawn_lane to create one."
                    ));
                }
                // Agent: the caller's choice, else the lane's earlier one, else
                // the track's worker agent.
                let agent_id = match (agent, &record, past) {
                    (Some(a), _, _) => a,
                    (None, Some(r), _) => r.agent.clone(),
                    (None, None, Some(p)) if !fresh => p.agent.clone(),
                    _ => info.lane_agent().to_string(),
                };
                // Memory only carries over on the agent that made it.
                let resume = record.filter(|r| r.agent == agent_id).map(|r| r.session_id);
                let wanted = resume.is_some();
                let spec: AgentSpec = state.spec_for(&agent_id)?;
                // The track's worker options are in its worker agent's terms;
                // a lane on some other agent gets that agent's defaults.
                let empty = BTreeMap::new();
                let chosen = if agent_id == info.lane_agent() { &info.worker_config } else { &empty };
                let mut opts = session_options(&state, &agent_id, &info.cwd, chosen, None);
                opts.resume = resume;
                tracing::info!(%track, lane = %name, agent = %agent_id, resume = ?opts.resume, "opening lane session");
                let session = Arc::new(AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?);
                let resumed = session.resumed();
                remember_lane(
                    &state,
                    &track,
                    &name,
                    &LaneRecord {
                        session_id: session.session_id().to_string(),
                        agent: agent_id.clone(),
                    },
                );
                let turns = if resumed { past.map(|p| p.runs).unwrap_or(0) + 1 } else { 1 };
                lanes.insert(
                    key.clone(),
                    Live {
                        agent: agent_id.clone(),
                        session: session.clone(),
                        turns,
                        running: None,
                    },
                );
                let note = if resumed {
                    Some("Reopened with its earlier conversation.")
                } else if wanted {
                    Some("Its earlier conversation could not be restored; the lane starts fresh.")
                } else if past.is_some() {
                    Some("Started fresh; earlier runs stay in the record but the lane does not remember them.")
                } else {
                    None
                };
                (agent_id, turns, session, resumed, note)
            }
        }
    };

    let run = state
        .store
        .begin_run(&track, &name, &agent_id, &text, &info.cwd)
        .map_err(|e| e.to_string())?;
    if let Some(live) = state.sessions.lanes.lock().await.get_mut(&key) {
        live.running = Some(run.clone());
    }

    // The turn itself, in the background.
    let app_for_turn = app.clone();
    let (track_t, name_t, run_t, text_t, cwd_t) = (track.clone(), name.clone(), run.clone(), text, info.cwd.clone());
    tauri::async_runtime::spawn(async move {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pump_task = tauri::async_runtime::spawn(pump(
            app_for_turn.clone(),
            track_t.clone(),
            name_t.clone(),
            run_t.clone(),
            rx,
        ));
        let _ = tx.send(LaneEvent::Started {
            session_id: session.session_id().to_string(),
            cwd: cwd_t,
        });
        match tokio::time::timeout(LANE_TURN_TIMEOUT, session.prompt(text_t, tx.clone())).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = tx.send(LaneEvent::Failed { error: err.to_string() });
            }
            Err(_) => {
                let _ = tx.send(LaneEvent::Failed { error: "lane turn timed out".to_string() });
            }
        }
        drop(tx);
        let _ = pump_task.await;

        {
            let state = app_for_turn.state::<AppState>();
            let mut lanes = state.sessions.lanes.lock().await;
            if let Some(live) = lanes.get_mut(&lane_key(&track_t, &name_t)) {
                if live.running.as_deref() == Some(run_t.as_str()) {
                    live.running = None;
                }
            }
            drop(lanes);
        }
        report_to_conductor(app_for_turn, track_t, name_t, run_t).await;
    });

    let mut result = json!({
        "run": run,
        "lane": name,
        "agent": agent_id,
        "turn": turns,
        "resumed": resumed,
        "status": "running",
        "note": format!("The lane is working. Its report will arrive as a {REPORT_PREFIX} message; tell the human what you delegated and end your turn."),
    });
    if let Some(note) = note {
        result["memory"] = Value::String(note.to_string());
    }
    Ok(result)
}

/// Hand a finished lane run to its track's conductor as a new turn.
async fn report_to_conductor(app: AppHandle, track: String, lane: String, run: String) {
    let state = app.state::<AppState>();
    let summary = match state.store.run(&run) {
        Ok(Some(s)) => s,
        _ => {
            tracing::warn!(%run, "lane run vanished before reporting");
            return;
        }
    };
    let mut output = summary.output.trim().to_string();
    const MAX: usize = 12_000;
    if output.chars().count() > MAX {
        output = format!("{}…\n(truncated: read_report(\"{run}\") has it all)", output.chars().take(MAX).collect::<String>());
    }
    let text = format!(
        "{REPORT_PREFIX} lane={lane} run={run} status={} tools={} duration_ms={}{}\n\n{}",
        summary.status.as_str(),
        summary.tool_count,
        summary.duration_ms.unwrap_or(0),
        summary.error.as_ref().map(|e| format!(" error={e}")).unwrap_or_default(),
        if output.is_empty() { "(no text output)" } else { &output },
    );

    // Wait for the conductor to be free, then run the report as a turn.
    let deadline = std::time::Instant::now() + REPORT_WAIT;
    while state.sessions.is_busy(&track) {
        if std::time::Instant::now() > deadline {
            tracing::warn!(%track, %lane, %run, "conductor stayed busy; report dropped");
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if !state.sessions.conductors.lock().await.contains_key(&track) {
        tracing::warn!(%track, %lane, %run, "no conductor session to report to");
        return;
    }
    let lang = state.store.get_meta("setting:language").ok().flatten().unwrap_or_default();
    if let Err(err) = conductor_turn(app.clone(), track.clone(), text, None, lang).await {
        tracing::warn!(%track, %lane, %run, %err, "could not deliver lane report to the conductor");
    }
}

/// Send one message to a track's conductor, opening (or resuming) its
/// session on first use or when the agent changed. `agent` overrides the
/// track's recorded agent and becomes it. Returns the conductor run id; the
/// turn continues in the background and streams under that id. `lang`
/// picks the preamble's language on a fresh session.
pub async fn conductor_turn(
    app: AppHandle,
    track: String,
    prompt: String,
    agent: Option<String>,
    lang: String,
) -> Result<String, String> {
    let st = app.state::<AppState>();
    let mut info = track_info(&st, &track)?;
    if let Some(agent) = agent.filter(|a| *a != info.agent) {
        let patch = orchestra_store::TrackPatch {
            agent: Some(agent),
            ..Default::default()
        };
        info = st.store.update_track(&track, &patch).map_err(|e| e.to_string())?;
    }
    let agent = info.agent.clone();
    let wanted = fingerprint(&agent, &info.conductor_config);

    if !st.sessions.begin_turn(&track) {
        return Err("conductor is still responding".to_string());
    }

    let outcome: Result<String, String> = async {
        let mut guard = st.sessions.conductors.lock().await;
        let needs_open = match guard.get(&track) {
            Some(c) => c.fingerprint != wanted,
            None => true,
        };
        if needs_open {
            if let Some(old) = guard.remove(&track) {
                tracing::info!(%track, agent = %old.live.agent, "closing conductor session (agent or options changed)");
                drop(old);
            }
            let spec = st.spec_for(&agent)?;
            // This track's tools, on their own server: every call is scoped.
            let mcp = McpServer::start("orchestra", tools(app.clone(), track.clone()))
                .await
                .map_err(|e| e.to_string())?;
            let mut opts = session_options(&st, &agent, &info.cwd, &info.conductor_config, Some(&mcp));
            // Bring back the conductor this track had last time, with its memory.
            let key = format!("conductor_session:{track}:{agent}");
            opts.resume = st.store.get_meta(&key).ok().flatten();
            tracing::info!(%track, agent = %agent, mcp = %mcp.url(), resume = ?opts.resume, "opening conductor session");
            let session = AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?;
            let resumed = session.resumed();
            if let Err(err) = st.store.set_meta(&key, session.session_id()) {
                tracing::warn!(%err, "could not remember conductor session id");
            }
            guard.insert(
                track.clone(),
                Conductor {
                    live: Live {
                        agent: agent.clone(),
                        session: Arc::new(session),
                        // A resumed session already had its preamble.
                        turns: if resumed { 1 } else { 0 },
                        running: None,
                    },
                    fingerprint: wanted.clone(),
                    _mcp: mcp,
                },
            );
        }
        let live = &mut guard.get_mut(&track).expect("conductor session just ensured").live;
        live.turns += 1;
        let first = live.turns == 1;
        let session_id = live.session.session_id().to_string();
        let session = live.session.clone();
        drop(guard);

        let run = st
            .store
            .begin_run(&track, CONDUCTOR_LANE, &agent, &prompt, &info.cwd)
            .map_err(|e| e.to_string())?;

        let text = if first { format!("{}\n\n---\n\n{prompt}", preamble(&lang, &info)) } else { prompt };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tauri::async_runtime::spawn(pump(app.clone(), track.clone(), CONDUCTOR_LANE.to_string(), run.clone(), rx));
        let _ = tx.send(LaneEvent::Started {
            session_id,
            cwd: info.cwd.clone(),
        });

        // The turn runs in the background; the busy mark clears when it ends.
        let app_for_turn = app.clone();
        let run_for_turn = run.clone();
        let track_for_turn = track.clone();
        tauri::async_runtime::spawn(async move {
            let st = app_for_turn.state::<AppState>();
            // No lock held during the turn: the tools it calls lock too.
            let started = std::time::Instant::now();
            tracing::info!(track = %track_for_turn, run = %run_for_turn, "conductor turn starting");
            let result = session.prompt(text, tx.clone()).await;
            tracing::info!(run = %run_for_turn, elapsed_ms = started.elapsed().as_millis() as u64, ok = result.is_ok(), "conductor turn ended");
            if let Err(err) = result {
                let _ = tx.send(LaneEvent::Failed { error: err.to_string() });
                // A failed turn kills the session; drop it so the next prompt reopens.
                st.sessions.conductors.lock().await.remove(&track_for_turn);
            }
            st.sessions.end_turn(&track_for_turn);
        });

        Ok(run)
    }
    .await;

    if outcome.is_err() {
        st.sessions.end_turn(&track);
    }
    outcome
}
