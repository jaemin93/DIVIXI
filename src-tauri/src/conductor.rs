//! The conductor and its lanes.
//!
//! The conductor is an agent session at Track level. What it can do to the
//! app arrives as MCP tools (served in-process, see `orchestra_mcp`): open a
//! lane, ask a lane a follow-up, read a report, record a decision. Whether
//! a human message is a question to answer or work to delegate is the
//! conductor's call, made in its prompt, not in Rust — the core is the
//! mechanism, the agent is the policy.
//!
//! Lanes are agent sessions too, one per lane name, alive across runs, so
//! a lane is a Claude Code (or Codex, …) session the conductor can keep
//! talking to. Every turn, on the conductor or a lane, is one run in the
//! store; the timeline shows them in creation order.

use std::collections::HashMap;
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

/// A lane run waits at most this long for the agent to finish a turn.
const LANE_TURN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// One open agent session and what it runs on.
pub struct Live {
    pub agent: String,
    /// Shared so a turn can run without holding the sessions lock; the
    /// session ends when the last clone drops.
    pub session: Arc<AgentSession>,
    pub turns: u32,
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
- 같은 레인에 이어서 시킬 일은 `ask_lane`으로 보냅니다. 레인은 이전 대화를 기억합니다. 어떤 레인이 열려 있는지 모르면 `list_lanes`를 먼저 봅니다.
- 도구가 오류를 돌려주면 오류 문구에 적힌 대로 한 번만 다시 시도하고, 그래도 안 되면 사람에게 무엇이 막혔는지 말합니다. 같은 도구를 반복해서 부르지 않습니다.
- 레인의 보고를 받으면 사람에게 무슨 일이 있었는지 한두 문단으로 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다.
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
    let list_app = app.clone();
    let report_app = app.clone();
    let close_app = app.clone();
    let decide_app = app;

    vec![
        Tool::new(
            "spawn_lane",
            "Open a new lane (a separate agent session) and give it a task. Blocks until the lane finishes its turn and returns its report: status, output text, tool count, duration. Use for real work; answer questions yourself instead.",
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
                    lane_turn(app, name, task, agent, true).await
                }
            },
        ),
        Tool::new(
            "ask_lane",
            "Send a follow-up message to an existing lane. The lane remembers its earlier turns. Blocks until it answers and returns its report.",
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
                    lane_turn(app, name, message, None, false).await
                }
            },
        ),
        Tool::new(
            "list_lanes",
            "List open lanes with their agent and how many turns they have run.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let app = list_app.clone();
                async move {
                    let state = app.state::<AppState>();
                    let lanes = state.sessions.lanes.lock().await;
                    let list: Vec<Value> = lanes
                        .iter()
                        .map(|(name, live)| json!({ "name": name, "agent": live.agent, "turns": live.turns }))
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
    }
}

/// Run one turn on a lane, opening it first when `open` is set. Records a
/// run, streams its events to the webview, waits for the turn, and returns
/// the folded report.
async fn lane_turn(
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

    // Resolve or open the lane session.
    let (agent_id, turns) = {
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
                live.turns += 1;
                (live.agent.clone(), live.turns)
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
                let session = AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?;
                lanes.insert(
                    name.clone(),
                    Live {
                        agent: agent_id.clone(),
                        session: Arc::new(session),
                        turns: 1,
                    },
                );
                (agent_id, 1)
            }
        }
    };

    let run = state
        .store
        .begin_run(&name, &agent_id, &text, &workspace_root().display().to_string())
        .map_err(|e| e.to_string())?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let pump_task = tauri::async_runtime::spawn(pump(app.clone(), name.clone(), run.clone(), rx));

    // Take a handle and release the lanes lock: other tools stay usable
    // during the turn, and the session itself serialises prompts.
    let session = {
        let lanes = state.sessions.lanes.lock().await;
        lanes.get(&name).map(|l| l.session.clone()).ok_or_else(|| format!("lane {name} vanished"))?
    };
    let _ = tx.send(LaneEvent::Started {
        session_id: session.session_id().to_string(),
        cwd: workspace_root().display().to_string(),
    });
    let result = tokio::time::timeout(LANE_TURN_TIMEOUT, session.prompt(text, tx.clone())).await;
    match result {
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

    let summary = state
        .store
        .run(&run)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("run {run} not recorded"))?;
    Ok(json!({
        "run": summary.id,
        "lane": name,
        "agent": agent_id,
        "turn": turns,
        "status": summary.status.as_str(),
        "output": summary.output,
        "tool_count": summary.tool_count,
        "duration_ms": summary.duration_ms,
        "error": summary.error,
    }))
}

/// Send one human message to the conductor, opening its session on first
/// use or when the chosen agent changed. Returns the conductor run id; the
/// turn continues in the background and streams under that id.
pub async fn conductor_prompt(
    app: AppHandle,
    state: Arc<AppStateRef>,
    prompt: String,
    agent: String,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    let st = state.get();
    if st.sessions.conductor_busy.swap(true, Ordering::SeqCst) {
        return Err("지휘자가 아직 응답 중입니다".to_string());
    }

    let outcome: Result<String, String> = async {
        // Open or replace the conductor session.
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
            let opts = session_options(&st, &agent, Some(&st.mcp));
            tracing::info!(agent = %agent, mcp = %st.mcp.url(), "opening conductor session");
            let session = AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?;
            *guard = Some(Live {
                agent: agent.clone(),
                session: Arc::new(session),
                turns: 0,
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
        let state_for_turn = state.clone();
        let run_for_turn = run.clone();
        tauri::async_runtime::spawn(async move {
            let st = state_for_turn.get();
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

/// A clonable handle to the app state for spawned tasks.
pub struct AppStateRef(pub AppHandle);

impl AppStateRef {
    pub fn get(&self) -> tauri::State<'_, AppState> {
        self.0.state::<AppState>()
    }
}
