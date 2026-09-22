//! The conductor and its lanes.
//!
//! The conductor is an agent session at Track level. What it can do to the
//! app arrives as MCP tools (served in-process, see `orchestra_mcp`): open a
//! lane, ask a lane a follow-up, check on it, read a report, record a
//! decision. Whether a human message is a question to answer or work to
//! delegate is the conductor's call, made in its prompt, not in Rust — the
//! core is the mechanism, the agent is the policy.
//!
//! Lanes are agent sessions too, one per lane name, alive across runs, so
//! a lane is a Claude Code (or Codex, …) session the conductor keeps
//! talking to. Every turn, on the conductor or a lane, is one run in the
//! store.
//!
//! Lane work is asynchronous from the conductor's point of view: `spawn_lane`
//! and `ask_lane` return as soon as the lane has the task, and when the lane
//! finishes, Orchestra hands its report to the conductor as a new turn. A
//! blocking tool would trip the agent's own MCP call timeout on any lane
//! that runs for minutes, which real work does.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use orchestra_acp::{AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_core::LaneEvent;
use orchestra_mcp::{McpServer, Tool};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::{pump, workspace_root, AppState};

/// The lane name conductor turns are recorded under.
pub const CONDUCTOR_LANE: &str = "conductor";

/// Prefix of conductor prompts that Orchestra itself injects (lane reports).
/// The timeline shows these as system lines, not as the human speaking.
pub const REPORT_PREFIX: &str = "[레인 보고]";

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

/// Conductor and lane sessions. Held behind async mutexes because opening
/// a session and running a turn both await.
#[derive(Default)]
pub struct Sessions {
    pub conductor: Mutex<Option<Live>>,
    pub lanes: Mutex<HashMap<String, Live>>,
    /// Set while a conductor turn is in flight.
    pub conductor_busy: std::sync::atomic::AtomicBool,
}

/// What the conductor is told once, at the start of its session.
fn preamble() -> String {
    r#"당신은 Orchestra의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 레인(lane)이라는 별도의 에이전트 세션에 맡깁니다.

규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 레인을 열지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_lane`으로 레인을 열어 맡깁니다. 레인 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 레인이 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- `spawn_lane`과 `ask_lane`은 레인이 일을 받는 즉시 돌아옵니다. 결과를 기다리지 말고, 사람에게 무엇을 맡겼는지 한 문장으로 알린 뒤 턴을 끝냅니다. 레인이 끝나면 `[레인 보고]`로 시작하는 메시지가 당신에게 옵니다. 그때 무슨 일이 있었는지 한두 문단으로 사람에게 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다.
- 같은 레인에 이어서 시킬 일은 `ask_lane`으로 보냅니다. 레인은 이전 대화를 기억합니다. 어떤 레인이 열려 있고 무엇을 하는 중인지는 `lane_status`로 봅니다.
- 도구가 오류를 돌려주면 오류 문구에 적힌 대로 한 번만 다시 시도하고, 그래도 안 되면 사람에게 무엇이 막혔는지 말합니다. 같은 도구를 반복해서 부르지 않습니다.
- 사람이 결정해야 할 일(되돌리기 어려운 변경, 여러 갈래 중 선택)은 스스로 정하지 말고 선택지를 제시하고 묻습니다. 사람이 정하면 `record_decision`으로 남깁니다.
- 한국어로 말합니다. 짧게, 명확하게.

지금 작업 디렉터리는 사람이 연 저장소입니다. 레인도 같은 디렉터리에서 일합니다.
"#
    .to_string()
}

/// Build the MCP tools that give the conductor its hands. Each captures the
/// app handle and reaches the state through it.
pub fn tools(app: AppHandle) -> Vec<Tool> {
    let spawn_app = app.clone();
    let ask_app = app.clone();
    let status_app = app.clone();
    let report_app = app.clone();
    let close_app = app.clone();
    let decide_app = app;

    vec![
        Tool::new(
            "spawn_lane",
            "Open a new lane (a separate agent session) and give it a task. Returns at once with the run id; the lane works in the background and its report reaches you later as a message starting with [레인 보고]. Use for real work; answer questions yourself instead.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short lane name, lowercase, e.g. fix-parser. Must be new." },
                    "task": { "type": "string", "description": "What the lane should do, specific enough to finish alone." },
                    "agent": { "type": "string", "description": "Agent id to run the lane on: claude_code, codex, copilot, antigravity. Defaults to the conductor's agent." }
                },
                "required": ["name", "task"]
            }),
            move |args| {
                let app = spawn_app.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let task = str_arg(&args, "task")?;
                    let agent = args.get("agent").and_then(Value::as_str).map(str::to_owned);
                    start_lane_turn(app, name, task, agent, true).await
                }
            },
        ),
        Tool::new(
            "ask_lane",
            "Send a follow-up message to an existing lane. The lane remembers its earlier turns. Returns at once; the lane's answer reaches you later as a [레인 보고] message.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Lane name given to spawn_lane." },
                    "message": { "type": "string" }
                },
                "required": ["name", "message"]
            }),
            move |args| {
                let app = ask_app.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let message = str_arg(&args, "message")?;
                    start_lane_turn(app, name, message, None, false).await
                }
            },
        ),
        Tool::new(
            "lane_status",
            "List open lanes: agent, turns run, and whether a turn is in flight (with its run id).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let app = status_app.clone();
                async move {
                    let state = app.state::<AppState>();
                    let lanes = state.sessions.lanes.lock().await;
                    let list: Vec<Value> = lanes
                        .iter()
                        .map(|(name, live)| {
                            json!({
                                "name": name,
                                "agent": live.agent,
                                "turns": live.turns,
                                "running": live.running.is_some(),
                                "run": live.running,
                            })
                        })
                        .collect();
                    Ok(Value::Array(list))
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
                let app = report_app.clone();
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
            "Close a lane's session. Its runs stay in the record.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            move |args| {
                let app = close_app.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let state = app.state::<AppState>();
                    let removed = state.sessions.lanes.lock().await.remove(&name);
                    match removed {
                        Some(live) => {
                            drop(live);
                            Ok(Value::String(format!("closed {name}")))
                        }
                        None => Err(format!("no lane {name}")),
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
                let app = decide_app.clone();
                async move {
                    let state = app.state::<AppState>();
                    let mut list: Vec<Value> = state
                        .store
                        .get_meta("decisions")
                        .map_err(|e| e.to_string())?
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default();
                    let mut entry = args.clone();
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

/// Session options for an agent: workspace, autonomous mode, chosen model.
fn session_options(state: &AppState, agent: &str, mcp: Option<&McpServer>) -> SessionOptions {
    let model = state.chosen_model(agent);
    let config = model
        .and_then(|m| state.model_option_id(agent).map(|id| vec![(id, m)]))
        .unwrap_or_default();
    SessionOptions {
        cwd: workspace_root(),
        mode: None,
        config,
        mcp_servers: mcp
            .map(|m| {
                vec![McpHttp {
                    name: "orchestra".to_string(),
                    url: m.url(),
                    headers: vec![m.auth_header()],
                }]
            })
            .unwrap_or_default(),
        resume: None,
    }
}

/// Give a lane a turn and return at once. Opens the lane first when `open`
/// is set. The turn runs in the background; when it ends, its report is
/// handed to the conductor as a new turn.
async fn start_lane_turn(
    app: AppHandle,
    name: String,
    text: String,
    agent: Option<String>,
    open: bool,
) -> Result<Value, String> {
    if name == CONDUCTOR_LANE {
        return Err("that name is reserved".to_string());
    }
    let state = app.state::<AppState>();

    // Resolve or open the lane session; refuse a second turn on a busy lane.
    let (agent_id, turns, session) = {
        let mut lanes = state.sessions.lanes.lock().await;
        match (lanes.get_mut(&name), open) {
            (Some(live), true) => {
                return Err(format!(
                    "lane {name} already exists (agent {}, {} turns). Send follow-ups with ask_lane(name=\"{name}\", message=...), or spawn_lane with a new name.",
                    live.agent, live.turns
                ))
            }
            (None, false) => {
                let open: Vec<String> = lanes.keys().cloned().collect();
                return Err(format!("no lane {name}. Open lanes: {open:?}. Use spawn_lane to create one."));
            }
            (Some(live), false) => {
                if let Some(run) = &live.running {
                    return Err(format!(
                        "lane {name} is still working on run {run}. Wait for its [레인 보고] before sending more."
                    ));
                }
                live.turns += 1;
                (live.agent.clone(), live.turns, live.session.clone())
            }
            (None, true) => {
                let agent_id = match agent {
                    Some(a) => a,
                    None => state
                        .sessions
                        .conductor
                        .lock()
                        .await
                        .as_ref()
                        .map(|c| c.agent.clone())
                        .unwrap_or_else(|| "claude_code".to_string()),
                };
                let spec: AgentSpec = state.spec_for(&agent_id)?;
                let opts = session_options(&state, &agent_id, None);
                tracing::info!(lane = %name, agent = %agent_id, "opening lane session");
                let session = Arc::new(AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?);
                lanes.insert(
                    name.clone(),
                    Live {
                        agent: agent_id.clone(),
                        session: session.clone(),
                        turns: 1,
                        running: None,
                    },
                );
                (agent_id, 1, session)
            }
        }
    };

    let run = state
        .store
        .begin_run(&name, &agent_id, &text, &workspace_root().display().to_string())
        .map_err(|e| e.to_string())?;
    if let Some(live) = state.sessions.lanes.lock().await.get_mut(&name) {
        live.running = Some(run.clone());
    }

    // The turn itself, in the background.
    let app_for_turn = app.clone();
    let (name_t, run_t, text_t) = (name.clone(), run.clone(), text);
    tauri::async_runtime::spawn(async move {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pump_task = tauri::async_runtime::spawn(pump(app_for_turn.clone(), name_t.clone(), run_t.clone(), rx));
        let _ = tx.send(LaneEvent::Started {
            session_id: session.session_id().to_string(),
            cwd: workspace_root().display().to_string(),
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
            if let Some(live) = lanes.get_mut(&name_t) {
                if live.running.as_deref() == Some(run_t.as_str()) {
                    live.running = None;
                }
            }
            drop(lanes);
        }
        report_to_conductor(app_for_turn, name_t, run_t).await;
    });

    Ok(json!({
        "run": run,
        "lane": name,
        "agent": agent_id,
        "turn": turns,
        "status": "running",
        "note": "The lane is working. Its report will arrive as a [레인 보고] message; tell the human what you delegated and end your turn.",
    }))
}

/// Hand a finished lane run to the conductor as a new turn.
async fn report_to_conductor(app: AppHandle, lane: String, run: String) {
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
        output = format!("{}…\n(잘림: 전체는 read_report(\"{run}\"))", output.chars().take(MAX).collect::<String>());
    }
    let text = format!(
        "{REPORT_PREFIX} lane={lane} run={run} status={} tools={} duration_ms={}{}\n\n{}",
        summary.status.as_str(),
        summary.tool_count,
        summary.duration_ms.unwrap_or(0),
        summary.error.as_ref().map(|e| format!(" error={e}")).unwrap_or_default(),
        if output.is_empty() { "(텍스트 출력 없음)" } else { &output },
    );

    // Wait for the conductor to be free, then run the report as a turn.
    let deadline = std::time::Instant::now() + REPORT_WAIT;
    while state.sessions.conductor_busy.load(Ordering::SeqCst) {
        if std::time::Instant::now() > deadline {
            tracing::warn!(%lane, %run, "conductor stayed busy; report dropped");
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let agent = state.sessions.conductor.lock().await.as_ref().map(|c| c.agent.clone());
    let Some(agent) = agent else {
        tracing::warn!(%lane, %run, "no conductor session to report to");
        return;
    };
    if let Err(err) = conductor_turn(app.clone(), text, agent).await {
        tracing::warn!(%lane, %run, %err, "could not deliver lane report to the conductor");
    }
}

/// Send one message to the conductor, opening (or resuming) its session on
/// first use or when the chosen agent changed. Returns the conductor run
/// id; the turn continues in the background and streams under that id.
pub async fn conductor_turn(app: AppHandle, prompt: String, agent: String) -> Result<String, String> {
    let st = app.state::<AppState>();
    if st.sessions.conductor_busy.swap(true, Ordering::SeqCst) {
        return Err("지휘자가 아직 응답 중입니다".to_string());
    }

    let outcome: Result<String, String> = async {
        let mut guard = st.sessions.conductor.lock().await;
        let needs_open = match guard.as_ref() {
            Some(live) => live.agent != agent,
            None => true,
        };
        if needs_open {
            if let Some(old) = guard.take() {
                tracing::info!(agent = %old.agent, "closing conductor session (agent changed)");
                drop(old);
            }
            let spec = st.spec_for(&agent)?;
            let mut opts = session_options(&st, &agent, Some(&st.mcp));
            // Bring back the conductor this agent had last time, with its memory.
            let key = format!("conductor_session:{agent}");
            opts.resume = st.store.get_meta(&key).ok().flatten();
            tracing::info!(agent = %agent, mcp = %st.mcp.url(), resume = ?opts.resume, "opening conductor session");
            let session = AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?;
            let resumed = session.resumed();
            if let Err(err) = st.store.set_meta(&key, session.session_id()) {
                tracing::warn!(%err, "could not remember conductor session id");
            }
            *guard = Some(Live {
                agent: agent.clone(),
                session: Arc::new(session),
                // A resumed session already had its preamble.
                turns: if resumed { 1 } else { 0 },
                running: None,
            });
        }
        let live = guard.as_mut().expect("conductor session just ensured");
        live.turns += 1;
        let first = live.turns == 1;
        let session_id = live.session.session_id().to_string();
        let session = live.session.clone();
        drop(guard);

        let run = st
            .store
            .begin_run(CONDUCTOR_LANE, &agent, &prompt, &workspace_root().display().to_string())
            .map_err(|e| e.to_string())?;

        let text = if first { format!("{}\n\n---\n\n{prompt}", preamble()) } else { prompt };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tauri::async_runtime::spawn(pump(app.clone(), CONDUCTOR_LANE.to_string(), run.clone(), rx));
        let _ = tx.send(LaneEvent::Started {
            session_id,
            cwd: workspace_root().display().to_string(),
        });

        // The turn runs in the background; the busy flag clears when it ends.
        let app_for_turn = app.clone();
        let run_for_turn = run.clone();
        tauri::async_runtime::spawn(async move {
            let st = app_for_turn.state::<AppState>();
            // No lock held during the turn: the tools it calls lock too.
            let started = std::time::Instant::now();
            tracing::info!(run = %run_for_turn, "conductor turn starting");
            let result = session.prompt(text, tx.clone()).await;
            tracing::info!(run = %run_for_turn, elapsed_ms = started.elapsed().as_millis() as u64, ok = result.is_ok(), "conductor turn ended");
            if let Err(err) = result {
                let _ = tx.send(LaneEvent::Failed { error: err.to_string() });
                // A failed turn kills the session; drop it so the next prompt reopens.
                st.sessions.conductor.lock().await.take();
            }
            st.sessions.conductor_busy.store(false, Ordering::SeqCst);
        });

        Ok(run)
    }
    .await;

    if outcome.is_err() {
        st.sessions.conductor_busy.store(false, Ordering::SeqCst);
    }
    outcome
}
