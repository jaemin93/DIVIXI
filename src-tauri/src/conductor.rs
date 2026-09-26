//! The conductor and its workers.
//!
//! A track has one conductor: an agent session at Track level. What it can
//! do to the app arrives as MCP tools (served in-process, see
//! `orchestra_mcp`): open a worker, ask a worker a follow-up, check on it, read
//! a report, record a decision. Whether a human message is a question to
//! answer or work to delegate is the conductor's call, made in its prompt,
//! not in Rust — the core is the mechanism, the agent is the policy.
//!
//! Each track's conductor gets its own MCP server, so every tool call is
//! already scoped to the track that made it; the conductor never names its
//! track and cannot reach another.
//!
//! Workers are agent sessions too, one per worker name within a track, alive
//! across runs, so a worker is a Claude Code (or Codex, …) session the
//! conductor keeps talking to. Every turn, on the conductor or a worker, is one
//! run in the store. A worker's session id is remembered in the store as well,
//! so a closed worker (closed on purpose, or gone with an app restart) reopens
//! with its conversation when the conductor calls it by name again.
//!
//! Worker work is asynchronous from the conductor's point of view: `spawn_worker`
//! and `ask_worker` return as soon as the worker has the task, and when the worker
//! finishes, Divixi hands its report to the conductor as a new turn. A
//! blocking tool would trip the agent's own MCP call timeout on any worker
//! that runs for minutes, which real work does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use orchestra_acp::{AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_core::report::{self, Report};
use orchestra_core::{AgentEvent, RunStatus};
use orchestra_mcp::{McpServer, Tool};
use orchestra_store::{Decision, DecisionOption, DecisionStatus, NewDecision, PermissionAsk, TrackInfo};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;

use crate::{pump, worktree, AppState};

/// The worker name conductor turns are recorded under.
pub const CONDUCTOR_SESSION: &str = "conductor";

/// Prefix of conductor prompts that Divixi itself injects (worker reports).
/// Language-neutral; the timeline shows these as system lines, not as the
/// human speaking, and the preamble tells the conductor what it means.
pub const REPORT_PREFIX: &str = "[worker-report]";
/// Heads the list of files attached to a human message, in the stored
/// prompt and in what the conductor reads. The UI splits on it to show the
/// files as chips under the message.
pub const ATTACH_MARK: &str = "[attachments]";

/// The message with its attached files listed under it, one path a line:
/// the record keeps them, and an agent that ignores file blocks still sees
/// where the files are.
pub(crate) fn with_attachments(prompt: &str, files: &[PathBuf]) -> String {
    if files.is_empty() {
        return prompt.to_string();
    }
    let list: Vec<String> = files.iter().map(|f| format!("- {}", f.display())).collect();
    format!("{prompt}\n\n{ATTACH_MARK}\n{}", list.join("\n"))
}

/// First line of a turn that carries the human's answer to a decision card.
pub const DECISION_PREFIX: &str = "[decision]";
/// First line of a turn that hands the conductor a worker's permission
/// question, for it to answer with `answer_worker` (or put to the human).
pub const PERMISSION_PREFIX: &str = "[worker-permission]";

/// A worker run waits at most this long for the agent to finish a turn.
const WORKER_TURN_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// How long a worker report waits for the conductor to become free.
const REPORT_WAIT: Duration = Duration::from_secs(30 * 60);
/// What `conductor_turn` returns while an earlier turn is still running.
pub(crate) const BUSY: &str = "conductor is still responding";

/// One open agent session and what it runs on.
pub struct Live {
    pub agent: String,
    /// The folder the session works in; a session in another folder than
    /// the track now wants is reopened.
    pub cwd: String,
    /// Shared so a turn can run without holding the sessions lock; the
    /// session ends when the last clone drops.
    pub session: Arc<AgentSession>,
    pub turns: u32,
    /// Run id of the turn in flight, if any.
    pub running: Option<String>,
    /// When a turn last started or ended: an idle session is closed after a while.
    pub used: std::time::Instant,
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
pub(crate) fn fingerprint(agent: &str, config: &BTreeMap<String, String>) -> String {
    format!("{agent}\n{}", serde_json::to_string(config).unwrap_or_default())
}

/// Conductor and worker sessions, across tracks. Held behind async mutexes
/// because opening a session and running a turn both await.
#[derive(Default)]
pub struct Sessions {
    /// By track id.
    pub conductors: Mutex<HashMap<String, Conductor>>,
    /// By `track/worker` (see [`worker_key`]).
    pub workers: Mutex<HashMap<String, Live>>,
    /// Workers whose session is being opened (the lock is not held for it).
    opening: parking_lot::Mutex<HashSet<String>>,
    /// One conductor opening per track at a time, without holding
    /// `conductors` (and so every other track) while an agent starts.
    conductor_gates: parking_lot::Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Tracks whose conductor has a turn in flight.
    busy: parking_lot::Mutex<HashSet<String>>,
}

impl Sessions {
    fn conductor_gate(&self, track: &str) -> Arc<Mutex<()>> {
        self.conductor_gates.lock().entry(track.to_string()).or_default().clone()
    }

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

    /// Open agent sessions across tracks, and how many are mid-turn.
    pub async fn counts(&self) -> (usize, usize) {
        let conductors = self.conductors.lock().await;
        let workers = self.workers.lock().await;
        let open = conductors.len() + workers.len();
        let working = self.busy.lock().len() + workers.values().filter(|l| l.running.is_some()).count();
        (open, working)
    }

    /// Whether the conductor or any worker of a track has a turn in flight.
    pub async fn is_active(&self, track: &str) -> bool {
        if self.is_busy(track) {
            return true;
        }
        let prefix = worker_key(track, "");
        self.workers
            .lock()
            .await
            .iter()
            .any(|(key, live)| key.starts_with(&prefix) && live.running.is_some())
    }

    /// Forget every session of a track (its conductor and workers). Turns in
    /// flight are cancelled so they end soon; nothing new starts.
    pub async fn close_track(&self, track: &str) {
        if let Some(conductor) = self.conductors.lock().await.remove(track) {
            conductor.live.session.cancel();
        }
        let prefix = worker_key(track, "");
        let mut workers = self.workers.lock().await;
        let gone: Vec<String> = workers.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        for key in gone {
            if let Some(live) = workers.remove(&key) {
                live.session.cancel();
            }
        }
    }
}

/// Key of a worker in [`Sessions::workers`] and in the store's memory.
fn worker_key(track: &str, worker: &str) -> String {
    format!("{track}/{worker}")
}

/// Where a track's workers work, as the conductor is told.
enum Folders {
    Own,
    Checkout,
    Shared,
}

/// What the conductor is told once, at the start of its session, in the
/// interface language. Only the wording differs; the rules are the same.
fn preamble(lang: &str, track: &TrackInfo) -> String {
    let intent = track.intent.trim();
    let folders = match track.worker_folder.as_str() {
        "shared" => Folders::Shared,
        "worktree" if worktree::repository(std::path::Path::new(&track.cwd)).is_some() => Folders::Checkout,
        _ => Folders::Own,
    };
    if lang.starts_with("ko") {
        let about = if intent.is_empty() {
            String::new()
        } else {
            format!("\n이 트랙의 목적: {intent}\n")
        };
        return format!(
            r#"당신은 Divixi의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 작업자(worker)라는 별도의 에이전트 세션에 맡깁니다.

트랙 이름: {name}{about}
규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 작업자를 부르지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_worker`로 작업자를 불러 맡깁니다. 작업자 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 작업자가 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- `spawn_worker`와 `ask_worker`는 작업자가 일을 받는 즉시 돌아옵니다. 결과를 기다리지 말고, 사람에게 무엇을 맡겼는지 한 문장으로 알린 뒤 턴을 끝냅니다. 작업자가 끝나면 `{REPORT_PREFIX}`로 시작하는 메시지가 당신에게 옵니다. 앱이 작업자에게 받은 정해진 보고입니다: status(done·partial·blocked·failed), summary, changes, checks(검증과 결과), risks, questions, next, 그리고 앱이 본 편집 중 보고에 없는 것. 무슨 일이 있었는지 한두 문단으로 사람에게 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다. checks가 없거나 실패가 있으면 그렇다고 말합니다. questions는 당신이 답할 수 있으면 `ask_worker`로 답하고, 사람이 정할 일이면 `request_decision`으로 올립니다. 작업자의 전체 답은 `read_report`에 있습니다.
- 같은 작업자에게 이어서 시킬 일은 `ask_worker`로 보냅니다. 작업자는 이전 대화를 기억합니다.
- 작업자 목록은 열린 것과 닫힌 것 모두 `worker_status`로 봅니다. 터미널이나 파일을 뒤져 작업자를 찾지 않습니다.
- 닫힌 작업자는 같은 이름으로 `spawn_worker`나 `ask_worker`를 부르면 이전 대화를 기억한 채 다시 열립니다. 기억을 버리고 처음부터 시작하려면 `spawn_worker`에 fresh=true를 줍니다. `close_worker`는 세션만 닫고 기록은 남깁니다.
- 도구가 오류를 돌려주면 오류 문구에 적힌 대로 한 번만 다시 시도하고, 그래도 안 되면 사람에게 무엇이 막혔는지 말합니다. 같은 도구를 반복해서 부르지 않습니다.
- 사람이 골라야 할 일(여러 갈래 중 선택, 되돌리기 어려운 변경, 취향이나 우선순위)은 스스로 정하지 않습니다. 선택지를 본문에 A/B/C로 늘어놓지 말고 `request_decision`을 부르세요. 앱이 선택지를 버튼이 있는 결정 카드로 보여 줍니다. 부른 뒤에는 무엇을 물었는지 한 문장만 말하고 턴을 끝냅니다. 사람의 답은 `{DECISION_PREFIX}`로 시작하는 메시지로 옵니다. 작업자 보고에 사람이 정해야 할 질문이 있으면 그것도 `request_decision`으로 올립니다. 사람이 대화 중에 직접 정한 것은 `record_decision`으로 남깁니다.
- 사람은 작업자와 직접 이야기하지 않습니다. 작업자가 무언가를 해도 되는지 물으면 `{PERMISSION_PREFIX}`로 시작하는 메시지로 당신에게 옵니다. 사람의 지시와 맡긴 일의 범위 안이면 당신이 직접 골라 `answer_worker`로 답합니다(허용할 때는 보통 이번만 허용). 되돌리기 어렵거나 맡긴 범위를 벗어나거나 사람이 정해야 할 일이면 `request_decision`으로 사람에게 묻고, 답이 오면 그대로 `answer_worker`로 전합니다. 작업자는 답을 받을 때까지 기다리므로 미루지 않습니다.
- 사람이 고른 문서가 모인 지식 라이브러리가 있습니다. 사람이 "우리가 아는 것", 자기 문서·노트, 이름으로 특정 문서를 언급하거나, 맡기려는 일이 라이브러리가 다루는 주제에 닿으면 `knowledge_search`로 찾습니다(무엇이 있는지는 `knowledge_list_sources`). 일반적인 코딩 질문이나 작업 폴더만 봐도 되는 일에는 부르지 않습니다. 작업자는 라이브러리를 볼 수 없으므로, 작업자에게 필요한 내용은 핵심 사실과 읽을 파일 경로를 task에 직접 담아 넘깁니다. 라이브러리에서 가져온 내용은 출처(파일)를 밝힙니다.
- 한국어로 말합니다. 짧게, 명확하게.

작업 디렉터리는 {cwd} 입니다. {folders}
"#,
            name = track.name,
            cwd = track.cwd,
            folders = match folders {
                Folders::Own => "작업자는 저마다 이 디렉터리 아래 자기 이름의 폴더(예: {cwd}/fix-parser)에서 일하고, 결과물도 거기에 생깁니다. 앱이 폴더를 만들고 작업자에게 그 밖에는 쓰지 말라고 알립니다(읽기는 됩니다). 여러 작업자의 결과를 한곳에 모으거나 기존 파일을 고쳐야 하면, 그 일은 한 작업자에게 맡기고 경로를 task에 적습니다. 작업자가 폴더 밖을 편집하면 보고에 따로 표시됩니다.".replace("{cwd}", &track.cwd),
                Folders::Checkout => "git 저장소이므로 작업자는 저마다 자기 checkout(git worktree)에서 일합니다. 작업자의 변경은 사람이 보고 카드에서 합치기를 눌러야 이 디렉터리에 들어옵니다. 합치기 전에는 당신도 다른 작업자도 그 변경을 볼 수 없으니, 보고를 전할 때 변경이 아직 합쳐지지 않았다고 말하고, 한 작업자의 결과가 다음 작업의 전제면 사람에게 먼저 합쳐 달라고 한 뒤 맡깁니다. 사람이 새로 만들었지만 git에 추가하지 않은 파일은 작업자 checkout에 없으니, 그런 파일은 이 디렉터리의 경로를 task에 적어 줍니다. `worker_status`의 unmerged_files가 합쳐지지 않은 파일 수입니다.".to_string(),
                Folders::Shared => "작업자는 모두 이 디렉터리를 같이 씁니다. 같은 파일을 두 작업자에게 동시에 맡기지 않습니다.".to_string(),
            },
        );
    }
    let about = if intent.is_empty() {
        String::new()
    } else {
        format!("\nWhat this track is for: {intent}\n")
    };
    format!(
        r#"You are Divixi's conductor. You are the only one who talks to the human; real work is delegated to workers, which are separate agent sessions.

Track: {name}{about}
Rules:
- If the human's message is a question or small talk, answer it yourself. Do not open a worker.
- If real work is needed (reading, changing or investigating code), open a worker with `spawn_worker`. Name it short and lowercase (e.g. fix-parser) and make the task specific enough for the worker to finish alone.
- `spawn_worker` and `ask_worker` return as soon as the worker has the task. Do not wait for the result: tell the human in one sentence what you delegated and end your turn. When the worker finishes, a message starting with `{REPORT_PREFIX}` reaches you: the structured report the app took from the worker — status (done, partial, blocked, failed), summary, changes, checks (what was verified and how it came out), risks, questions, next, and any edits the app saw that the report does not list. Explain to the human in a paragraph or two what happened. Do not paste the report; give the gist. If there are no checks or a check failed, say so. Answer questions yourself with `ask_worker` when you can; put those only the human can answer to them with `request_decision`. The worker's whole reply is in `read_report`.
- Follow-ups for the same worker go through `ask_worker`; the worker remembers its earlier turns.
- `worker_status` lists every worker, open and closed. Never hunt for workers through the terminal or files.
- A closed worker reopens with its earlier conversation when you call `spawn_worker` or `ask_worker` with its name. To drop that memory and start over, pass fresh=true to `spawn_worker`. `close_worker` only closes the session; the record stays.
- If a tool returns an error, retry once as the message suggests; if that fails, tell the human what is blocked. Never call the same tool repeatedly.
- Choices that belong to the human (a choice between directions, hard-to-undo changes, taste or priorities) are not yours to make. Do not list options as A/B/C in prose: call `request_decision`, and the app shows them as a decision card with buttons. After calling it, say in one sentence what you asked and end your turn. The human's answer arrives as a message starting with `{DECISION_PREFIX}`. If a worker report raises a question only the human can answer, put that to them with `request_decision` too. Decisions the human makes in conversation are recorded with `record_decision`.
- The human does not talk to workers. When a worker asks whether it may do something, the question reaches you as a message starting with `{PERMISSION_PREFIX}`. If it is within the human's instructions and the task you gave, choose yourself and answer with `answer_worker` (usually allow once). If it is hard to undo, outside the task, or the human's call, ask them with `request_decision` and pass their answer on with `answer_worker`. The worker waits until answered, so do not leave it.
- There is a knowledge library of documents the human chose. When the human asks what we know about something, refers to their docs or notes or to a document by name, or when work you are about to delegate touches a topic the library covers, search it with `knowledge_search` (`knowledge_list_sources` shows what is there). Do not call it for general coding questions or what the working folder answers. Workers cannot see the library: put what they need from it (the key facts and the file paths to read) into their task. Name the file when you use something from the library.
- Speak English. Short and clear.

The working directory is {cwd}. {folders}
"#,
        name = track.name,
        cwd = track.cwd,
        folders = match folders {
            Folders::Own => "Each worker works in a folder of its own under this directory, named after it (e.g. {cwd}/fix-parser), and its results appear there. The app makes the folder and tells the worker not to write outside it (reading is fine). When results must come together in one place or existing files must change, give that to one worker and put the paths in its task. Edits a worker makes outside its folder are flagged in its report.".replace("{cwd}", &track.cwd),
            Folders::Checkout => "It is a git repository, so each worker works in a checkout of its own (a git worktree). A worker's changes reach this directory only when the human merges them from its report card. Until then neither you nor other workers can see them: when you relay a report, say the changes are not merged yet, and when one worker's result is the ground for the next task, ask the human to merge it first. Files the human made but has not added to git are not in a worker's checkout: put their path in this directory into the task. `worker_status` shows unmerged_files per worker.".to_string(),
            Folders::Shared => "All workers share this directory. Never give two workers the same files at the same time.".to_string(),
        },
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
    let ask_human = (app.clone(), track.clone());
    let answer = (app.clone(), track.clone());
    let search = app.clone();
    let list = app.clone();
    let decide = (app, track);

    vec![
        Tool::new(
            "spawn_worker",
            &format!("Open a worker (a separate agent session) and give it a task. Returns at once with the run id; the worker works in the background and its report reaches you later as a message starting with {REPORT_PREFIX}. A closed worker with this name is reopened with its earlier conversation (the result says resumed=true); pass fresh=true to start it over without that memory. Use for real work; answer questions yourself instead."),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short worker name, lowercase, e.g. fix-parser. New, or the name of a closed worker to reopen." },
                    "task": { "type": "string", "description": "What the worker should do, specific enough to finish alone." },
                    "agent": { "type": "string", "description": "Agent id to run the worker on: claude_code, codex, copilot, antigravity. Defaults to the worker's earlier agent, else the conductor's." },
                    "fresh": { "type": "boolean", "description": "Start over without the closed worker's earlier conversation. Default false." }
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
                    start_worker_turn(app, track, name, task, agent, true, fresh).await
                }
            },
        ),
        Tool::new(
            "ask_worker",
            &format!("Send a follow-up message to a worker. The worker remembers its earlier turns; a closed worker is reopened with that memory. Returns at once; the worker's answer reaches you later as a {REPORT_PREFIX} message."),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Worker name given to spawn_worker." },
                    "message": { "type": "string" }
                },
                "required": ["name", "message"]
            }),
            move |args| {
                let (app, track) = ask.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let message = str_arg(&args, "message")?;
                    start_worker_turn(app, track, name, message, None, false, false).await
                }
            },
        ),
        Tool::new(
            "worker_status",
            "List every worker of this track, open or closed: agent, whether its session is open, whether a turn is in flight (with its run id), how many runs it has, and its last run with status. A closed worker marked resumable reopens with its memory when you call spawn_worker or ask_worker with its name.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let (app, track) = status.clone();
                async move {
                    let state = app.state::<AppState>();
                    Ok(Value::Array(worker_list(&state, &track).await))
                }
            },
        ),
        Tool::new(
            "read_report",
            "Read a run by id (e.g. t004): status, prompt, the worker's whole reply (output), tools, duration, and its checked report.",
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
                        Some(summary) => {
                            let mut value = serde_json::to_value(summary).map_err(|e| e.to_string())?;
                            if let Some(report) = stored_report(&state, &run) {
                                value["report"] = serde_json::to_value(report).map_err(|e| e.to_string())?;
                            }
                            Ok(value)
                        }
                        None => Err(format!("no run {run}")),
                    }
                }
            },
        ),
        Tool::new(
            "close_worker",
            "Close a worker's session. Its runs and its memory stay in the record; spawn_worker or ask_worker with the same name reopens it.",
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
                    let mut workers = state.sessions.workers.lock().await;
                    let key = worker_key(&track, &name);
                    match workers.get(&key) {
                        Some(live) if live.running.is_some() => Err(format!(
                            "worker {name} is still working on run {}. Wait for its {REPORT_PREFIX} before closing it.",
                            live.running.as_deref().unwrap_or_default()
                        )),
                        Some(_) => {
                            workers.remove(&key);
                            Ok(Value::String(format!("closed {name}")))
                        }
                        None => Err(format!("no open worker {name}")),
                    }
                }
            },
        ),
        Tool::new(
            "request_decision",
            &format!("Put a choice to the human as a decision card with one button per option. Use it whenever the human has to pick (directions, hard-to-undo changes, taste, priorities) instead of listing options in prose. Returns at once: tell the human in one sentence what you asked, then end your turn. Their answer arrives later as a message starting with {DECISION_PREFIX}. Several decisions may be open at once."),
            json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question, one sentence, in the human's language." },
                    "context": { "type": "string", "description": "Why it matters or what it depends on, one to three sentences. Optional." },
                    "options": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 6,
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "Short: what the button says." },
                                "detail": { "type": "string", "description": "What choosing it means: consequences, trade-offs, cost of undoing." }
                            },
                            "required": ["label"]
                        }
                    },
                    "recommended": { "type": "integer", "description": "0-based index of the option you recommend, if any." },
                    "allow_other": { "type": "boolean", "description": "Let the human answer in their own words too. Default true." }
                },
                "required": ["question", "options"]
            }),
            move |args| {
                let (app, track) = ask_human.clone();
                async move {
                    let question = str_arg(&args, "question")?;
                    let options: Vec<DecisionOption> = args
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|list| {
                            list.iter()
                                .filter_map(|o| match o {
                                    // Tolerate a bare list of strings.
                                    Value::String(s) => Some(DecisionOption { label: s.trim().to_string(), detail: String::new(), id: String::new() }),
                                    Value::Object(_) => Some(DecisionOption {
                                        label: o.get("label").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
                                        detail: o.get("detail").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
                                        id: String::new(),
                                    }),
                                    _ => None,
                                })
                                .filter(|o| !o.label.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    if options.len() < 2 {
                        return Err("request_decision needs at least two options, each with a label".to_string());
                    }
                    let state = app.state::<AppState>();
                    let run = conductor_run(&state, &track).await;
                    let decision = state
                        .store
                        .open_decision(&NewDecision {
                            track: track.clone(),
                            run,
                            question,
                            context: args.get("context").and_then(Value::as_str).unwrap_or_default().to_string(),
                            options,
                            recommended: args.get("recommended").and_then(Value::as_u64).map(|i| i as usize),
                            allow_other: args.get("allow_other").and_then(Value::as_bool).unwrap_or(true),
                            permission: None,
                        })
                        .map_err(|e| e.to_string())?;
                    let _ = app.emit("decision", &decision);
                    Ok(json!({
                        "decision": decision.id,
                        "status": "waiting for the human",
                        "note": format!("The human sees this as a card with buttons. Do not repeat the options in prose; say in one sentence what you asked and end your turn. The answer arrives as a {DECISION_PREFIX} message."),
                    }))
                }
            },
        ),
        Tool::new(
            "answer_worker",
            &format!("Answer a permission question a worker is waiting on (a {PERMISSION_PREFIX} message): the id of one option it offered, or \"cancel\" to refuse. Decide it yourself when the human's instructions and the task cover it; when it is the human's call, ask with request_decision first and answer with their choice."),
            json!({
                "type": "object",
                "properties": {
                    "worker": { "type": "string", "description": "The worker that asked." },
                    "request": { "type": "string", "description": "The request id from the message, e.g. p1." },
                    "option": { "type": "string", "description": "One of the option ids it offered, or \"cancel\"." }
                },
                "required": ["worker", "request", "option"]
            }),
            move |args| {
                let (app, track) = answer.clone();
                async move {
                    let worker = str_arg(&args, "worker")?;
                    let request = str_arg(&args, "request")?;
                    let option = str_arg(&args, "option")?;
                    let state = app.state::<AppState>();
                    let session = {
                        let workers = state.sessions.workers.lock().await;
                        workers.get(&worker_key(&track, &worker)).map(|w| w.session.clone())
                    };
                    let session = session.ok_or_else(|| format!("no open worker {worker}; its question went with its session"))?;
                    let choice = if option == "cancel" { None } else { Some(option.as_str()) };
                    session.answer_permission(&request, choice).map_err(|e| e.to_string())?;
                    Ok(json!({ "answered": request, "worker": worker, "option": option }))
                }
            },
        ),
        Tool::new(
            "knowledge_search",
            "Search the human's knowledge library: documents they chose to add, split into sections, each with a title, a summary and the entities it names. Call it when the human asks what we know about something, refers to their docs or notes or to a stored document by name, or when a task you are about to delegate touches a topic the library covers (knowledge_list_sources shows the topics). Do NOT call it for general coding questions, file operations, debugging, or anything the working folder or the conversation already answers. Matching is by keyword, by entity and, when embeddings are set up, by meaning: use the distinctive words a document would contain, and try other wording once if nothing comes back. Workers cannot search the library; pass them what they need.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to find in the documents." },
                    "limit": { "type": "integer", "description": "Max results (default 3, max 5). One extra may be added when the best keyword match would otherwise be dropped.", "default": 3 },
                    "source_id": { "type": "string", "description": "Optional source id (from knowledge_list_sources) to search one document." }
                },
                "required": ["query"]
            }),
            move |args| {
                let app = search.clone();
                async move {
                    let query = str_arg(&args, "query")?;
                    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(3).clamp(1, 5) as usize;
                    let source = args.get("source_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                    let vector = crate::knowledge::query_vector(&app, &query).await;
                    let db = app.state::<AppState>().library.db.clone();
                    if let Some(id) = &source {
                        if db.source(id).map_err(|e| e.to_string())?.is_none() {
                            return Err(format!("No knowledge source with id {id}. Call knowledge_list_sources to see the valid ids."));
                        }
                    }
                    let hits = tokio::task::spawn_blocking(move || db.search(&query, limit, source.as_deref(), vector.as_ref().map(|(v, s)| (v.as_slice(), s.as_str()))))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    Ok(Value::String(orchestra_knowledge::format_hits(&hits)))
                }
            },
        ),
        Tool::new(
            "knowledge_list_sources",
            "What is in the human's knowledge library: counts, then one line per document with its id, item count, sync status and topic. Read-only. Use it to see which topics the library covers and to find a source_id for knowledge_search.",
            json!({ "type": "object", "properties": {} }),
            move |_args| {
                let app = list.clone();
                async move {
                    let db = app.state::<AppState>().library.db.clone();
                    let (sources, stats) = tokio::task::spawn_blocking(move || Ok::<_, anyhow::Error>((db.sources()?, db.stats()?)))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    Ok(Value::String(orchestra_knowledge::format_sources(&sources, &stats)))
                }
            },
        ),
        Tool::new(
            "record_decision",
            "Record a decision the human made in conversation: the question, the options, the choice and the rationale. Use after the human decides, never to decide for them. Answers given on a decision card are recorded already.",
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
                    let question = str_arg(&args, "question")?;
                    let choice = str_arg(&args, "choice")?;
                    let options: Vec<String> = args
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                        .unwrap_or_default();
                    let rationale = args.get("rationale").and_then(Value::as_str).unwrap_or_default();
                    let run = conductor_run(&state, &track).await;
                    let decision = state
                        .store
                        .record_decision(&track, run.as_deref(), &question, &options, &choice, rationale)
                        .map_err(|e| e.to_string())?;
                    let _ = app.emit("decision", &decision);
                    Ok(json!({ "recorded": decision.id }))
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

/// The conductor run in flight on a track, if any: what a decision hangs off.
async fn conductor_run(state: &AppState, track: &str) -> Option<String> {
    state.sessions.conductors.lock().await.get(track).and_then(|c| c.live.running.clone())
}

/// Where an agent's permission question goes. A worker's goes to its
/// conductor, which answers for the human; the conductor's own, and an
/// artifact agent's, go to the human as a card (nobody above them answers).
pub fn route_permission(app: &AppHandle, track: &str, session: &str, run: &str, event: &AgentEvent) {
    let AgentEvent::Permission { request, title, tool_kind, input, options } = event else {
        return;
    };
    let state = app.state::<AppState>();
    let artifact = track.starts_with("artifact:");
    if artifact || session == CONDUCTOR_SESSION {
        let ko = state.store.get_meta("setting:language").ok().flatten().as_deref() != Some("en");
        let who = match (artifact, ko) {
            (true, true) => "디자인 에이전트",
            (true, false) => "The design agent",
            (false, true) => "지휘자",
            (false, false) => "The conductor",
        };
        let question = if ko { format!("{who}가 허락을 구합니다: {title}") } else { format!("{who} asks to: {title}") };
        let mut context = tool_kind.clone();
        if !input.is_empty() {
            context = format!("{context}\n{input}");
        }
        let choices: Vec<DecisionOption> = options
            .iter()
            .map(|o| DecisionOption { label: o.name.clone(), detail: String::new(), id: o.id.clone() })
            .collect();
        let opened = state.store.open_decision(&NewDecision {
            track: track.to_string(),
            run: Some(run.to_string()),
            question,
            context,
            options: choices,
            recommended: None,
            allow_other: false,
            permission: Some(PermissionAsk {
                session: if artifact { "artifact".to_string() } else { CONDUCTOR_SESSION.to_string() },
                request: request.clone(),
            }),
        });
        match opened {
            Ok(d) => {
                let _ = app.emit("decision", &d);
            }
            Err(err) => tracing::warn!(%err, %track, "could not put a permission question to the human"),
        }
        return;
    }
    // A worker's: to the conductor, which answers with answer_worker.
    let list: Vec<String> = options.iter().map(|o| format!("- {}: {} ({})", o.id, o.name, o.kind)).collect();
    let mut text = format!("{PERMISSION_PREFIX} worker={session} request={request}\nwants to: {title} ({tool_kind})");
    if !input.is_empty() {
        text.push_str(&format!("\ninput: {input}"));
    }
    text.push_str(&format!(
        "\noptions:\n{}\n\nAnswer with answer_worker(worker=\"{session}\", request=\"{request}\", option=...). Decide yourself unless it is the human's call; then ask with request_decision and pass their choice on.",
        list.join("\n")
    ));
    let (app, track, worker, request) = (app.clone(), track.to_string(), session.to_string(), request.clone());
    tauri::async_runtime::spawn(async move {
        if deliver(app.clone(), track.clone(), text, true, format!("permission {request} of {worker}")).await {
            return;
        }
        // Nobody will answer it: refuse, so the worker carries on (or reports) instead of waiting out its turn.
        let st = app.state::<AppState>();
        let live = st.sessions.workers.lock().await.get(&worker_key(&track, &worker)).map(|w| w.session.clone());
        if let Some(session) = live {
            let _ = session.answer_permission(&request, None);
        }
    });
}

/// Permission cards are answered or set aside one at a time, each after
/// checking the card is still open, so an agent is never told one thing
/// while the record says another.
static PERMISSION_CARDS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Set a decision aside; a permission card's agent is refused.
pub async fn dismiss_decision(app: &AppHandle, id: i64) -> Result<Decision, String> {
    let _one = PERMISSION_CARDS.lock().await;
    let st = app.state::<AppState>();
    let decision = st.store.dismiss_decision(id).map_err(|e| e.to_string())?;
    refuse_permission(app, &decision).await;
    let _ = app.emit("decision", &decision);
    Ok(decision)
}

/// Set aside the open permission cards of a run that ended (or of every
/// run, at start): the questions went with the turn. Under the cards'
/// lock, so an answer being given right now is not overtaken.
pub async fn dismiss_stale_permissions(app: AppHandle, run: Option<String>) {
    let _one = PERMISSION_CARDS.lock().await;
    let st = app.state::<AppState>();
    let Ok(open) = st.store.decisions(None) else { return };
    for d in open {
        let stale = d.status == DecisionStatus::Open && d.permission.is_some() && run.as_deref().is_none_or(|r| d.run.as_deref() == Some(r));
        if stale {
            if let Ok(gone) = st.store.dismiss_decision(d.id) {
                let _ = app.emit("decision", &gone);
            }
        }
    }
}

/// Answer the agent behind a permission card: the chosen option, or `None`
/// to cancel.
async fn answer_asker(app: &AppHandle, decision: &Decision, ask: &PermissionAsk, option: Option<&str>) -> Result<(), String> {
    let st = app.state::<AppState>();
    if ask.session == "artifact" {
        let id = decision.track.strip_prefix("artifact:").unwrap_or(&decision.track);
        return crate::artifact::answer_permission(app, id, &ask.request, option).await;
    }
    let session = st.sessions.conductors.lock().await.get(&decision.track).map(|c| c.live.session.clone());
    let session = session.ok_or("the conductor's session is closed; the question went with it")?;
    session.answer_permission(&ask.request, option).map_err(|e| e.to_string())
}

/// Refuse the agent behind a permission card the human set aside.
pub async fn refuse_permission(app: &AppHandle, decision: &Decision) {
    if let Some(ask) = &decision.permission {
        if let Err(err) = answer_asker(app, decision, ask, None).await {
            tracing::info!(%err, decision = decision.id, "no agent waiting on a dismissed permission card");
        }
    }
}

/// The turn that hands the human's answer to the conductor.
fn decision_text(d: &Decision) -> String {
    let letter = |i: usize| char::from(b'A' + (i % 26) as u8);
    let answer = match (d.choice, d.answer.as_deref()) {
        (Some(i), Some(label)) => format!("{}. {label}", letter(i)),
        (None, Some(own)) => format!("(in their own words) {own}"),
        _ => "(no answer)".to_string(),
    };
    let mut text = format!("{DECISION_PREFIX} #{}\nquestion: {}\nanswer: {answer}", d.id, d.question);
    if !d.note.is_empty() {
        text.push_str(&format!("\nnote: {}", d.note));
    }
    text.push_str("\n\nThe human decided on the card. Carry on accordingly; this is recorded already.");
    text
}

/// Record the human's answer and hand it to the conductor as a turn,
/// opening the conductor if it is closed and waiting while it is busy.
pub async fn answer_decision(
    app: AppHandle,
    id: i64,
    choice: Option<usize>,
    own: Option<String>,
    note: String,
) -> Result<Decision, String> {
    let st = app.state::<AppState>();
    // A permission card answers the agent that asked, not the conductor's
    // conversation: the agent is mid-turn, waiting.
    let open = st.store.decision(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no decision {id}"))?;
    if open.permission.is_some() {
        let _one = PERMISSION_CARDS.lock().await;
        // Re-read under the lock: it may have been answered or set aside meanwhile.
        let open = st.store.decision(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no decision {id}"))?;
        if open.status != DecisionStatus::Open {
            return Err("this question was already answered or set aside".to_string());
        }
        let Some(ask) = &open.permission else { return Err("not a permission question".to_string()) };
        let option = choice.and_then(|i| open.options.get(i)).map(|o| o.id.clone()).ok_or("choose one of the options")?;
        if let Err(err) = answer_asker(&app, &open, ask, Some(&option)).await {
            if let Ok(gone) = st.store.dismiss_decision(id) {
                let _ = app.emit("decision", &gone);
            }
            return Err(err);
        }
        let decision = st.store.answer_decision(id, choice, None, &note).map_err(|e| e.to_string())?;
        let _ = app.emit("decision", &decision);
        return Ok(decision);
    }
    let decision = st
        .store
        .answer_decision(id, choice, own.as_deref(), &note)
        .map_err(|e| e.to_string())?;
    let _ = app.emit("decision", &decision);
    let text = decision_text(&decision);
    let track = decision.track.clone();
    tauri::async_runtime::spawn(deliver(app.clone(), track, text, true, format!("decision #{id}")));
    Ok(decision)
}

/// Run `text` as a conductor turn once the conductor is free. The turn
/// itself claims the busy mark, so a refusal is retried rather than
/// trusting an earlier free check that another turn may have overtaken.
/// `open` lets it start a closed conductor; a worker report does not.
/// Returns whether the turn started.
async fn deliver(app: AppHandle, track: String, text: String, open: bool, what: String) -> bool {
    let state = app.state::<AppState>();
    let deadline = std::time::Instant::now() + REPORT_WAIT;
    let lang = state.store.get_meta("setting:language").ok().flatten().unwrap_or_default();
    loop {
        if !open && !state.sessions.conductors.lock().await.contains_key(&track) {
            tracing::warn!(%track, %what, "no conductor session to deliver to");
            return false;
        }
        match conductor_turn(app.clone(), track.clone(), text.clone(), None, lang.clone(), Vec::new()).await {
            Ok(_) => return true,
            Err(err) if err == BUSY => {
                if std::time::Instant::now() > deadline {
                    tracing::warn!(%track, %what, "conductor stayed busy; dropped");
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(err) => {
                tracing::warn!(%track, %what, %err, "could not deliver to the conductor");
                return false;
            }
        }
    }
}

/// What the store remembers about a worker's last agent session, so the worker
/// can be reopened with its conversation.
#[derive(serde::Serialize, serde::Deserialize)]
struct WorkerRecord {
    session_id: String,
    agent: String,
}

fn worker_record_key(track: &str, name: &str) -> String {
    format!("worker_session:{}", worker_key(track, name))
}

fn worker_record(state: &AppState, track: &str, name: &str) -> Option<WorkerRecord> {
    state
        .store
        .get_meta(&worker_record_key(track, name))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn remember_worker(state: &AppState, track: &str, name: &str, record: &WorkerRecord) {
    match serde_json::to_string(record) {
        Ok(json) => {
            if let Err(err) = state.store.set_meta(&worker_record_key(track, name), &json) {
                tracing::warn!(worker = %name, %err, "could not remember worker session id");
            }
        }
        Err(err) => tracing::warn!(worker = %name, %err, "could not encode worker record"),
    }
}

/// Every worker the store knows in a track, merged with what is open right
/// now. The conductor's own worker is not a worker to it.
async fn worker_list(state: &AppState, track: &str) -> Vec<Value> {
    let history = state.store.sessions(track).unwrap_or_else(|err| {
        tracing::warn!(%err, "could not list workers from the store");
        Vec::new()
    });
    let workers = state.sessions.workers.lock().await;
    let mut list: Vec<Value> = history
        .iter()
        .filter(|info| info.name != CONDUCTOR_SESSION)
        .map(|info| {
            let live = workers.get(&worker_key(track, &info.name));
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
                "resumable": live.is_none() && worker_record(state, track, &info.name).is_some(),
            })
        })
        .collect();
    // A worker opened so recently that its first run is not in the store yet.
    let prefix = worker_key(track, "");
    for (key, live) in workers.iter() {
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
    drop(workers);
    for item in list.iter_mut() {
        let Some(name) = item["name"].as_str().map(str::to_string) else { continue };
        if let Ok(c) = worktree::changes(&state.store, track, &name) {
            if c.isolated && !c.files.is_empty() {
                item["unmerged_files"] = json!(c.files.len());
            }
        }
    }
    list
}

/// Whether a worker is in a turn right now.
pub async fn worker_running(state: &AppState, track: &str, worker: &str) -> bool {
    state.sessions.workers.lock().await.get(&worker_key(track, worker)).is_some_and(|l| l.running.is_some())
}

/// Session options for an agent: working directory, the track's choices
/// for that agent (`option id → value id`), and the conductor's tools when
/// it is the conductor.
///
/// The agent's mode option is sent as `session/set_mode`; every other
/// option goes through `session/set_config_option`. No mode chosen means
/// the most autonomous one the agent offers.
pub(crate) fn session_options(
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
    SessionOptions {
        cwd: PathBuf::from(cwd),
        mode,
        config,
        mcp_servers: mcp
            .map(|m| {
                vec![McpHttp {
                    name: "divixi".to_string(),
                    url: m.url(),
                    headers: vec![m.auth_header()],
                }]
            })
            .unwrap_or_default(),
        resume: None,
        restricted: false,
    }
}

fn track_info(state: &AppState, track: &str) -> Result<TrackInfo, String> {
    state
        .store
        .track(track)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no track {track}"))
}

/// Give a worker a turn and return at once. `open` is `spawn_worker` (a worker
/// that is already open is refused); `ask_worker` needs the worker to exist,
/// open or in the record. A worker that is not open but has a remembered
/// session is reopened with it, unless `fresh` says to forget. The turn
/// runs in the background; when it ends, its report is handed to the
/// conductor as a new turn.
async fn start_worker_turn(
    app: AppHandle,
    track: String,
    name: String,
    text: String,
    agent: Option<String>,
    open: bool,
    fresh: bool,
) -> Result<Value, String> {
    if name == CONDUCTOR_SESSION {
        return Err("that name is reserved".to_string());
    }
    let state = app.state::<AppState>();
    let info = track_info(&state, &track)?;
    let key = worker_key(&track, &name);

    // The worker's own checkout when the track is in git (made the first
    // time, brought up to date when nothing is pending), else the track
    // folder. Never touched while the worker is in a turn.
    if let Some(run) = state.sessions.workers.lock().await.get(&key).and_then(|l| l.running.clone()) {
        return Err(format!("worker {name} is still working on run {run}. Wait for its {REPORT_PREFIX} before sending more."));
    }
    let place = worker_place(&app, &info, &name).await?;
    let (worker_cwd, folder) = (place.cwd.clone(), place.for_conductor.clone());

    // Resolve or open the worker session; refuse a second turn on a busy
    // worker. Opening an agent takes seconds: the sessions lock is not held
    // for it (other workers and tracks go on); the name is marked as
    // opening instead, so a second call for it is refused meanwhile.
    // Made once per call and taken at once: its size does not matter.
    #[allow(clippy::large_enum_variant)]
    enum Found {
        Live { run: String, turns: u32, session: Arc<AgentSession>, agent: String },
        Open { agent_id: String, spec: AgentSpec, opts: SessionOptions, wanted: bool, past_runs: Option<u32>, handoff: Option<String> },
    }
    let found = {
        let mut workers = state.sessions.workers.lock().await;
        if state.sessions.opening.lock().contains(&key) {
            return Err(format!("worker {name} is still starting; wait for it before sending more."));
        }
        // The track now wants the worker elsewhere (its folder setting
        // changed): the session is reopened there, with its memory if it can.
        if workers.get(&key).is_some_and(|l| l.cwd != worker_cwd) {
            workers.remove(&key);
        }
        match (workers.get_mut(&key), open) {
            (Some(live), true) => {
                return Err(format!(
                    "worker {name} is already open (agent {}, {} turns). Send follow-ups with ask_worker(name=\"{name}\", message=...), or spawn_worker with a new name.",
                    live.agent, live.turns
                ))
            }
            (Some(live), false) => {
                if let Some(run) = &live.running {
                    return Err(format!(
                        "worker {name} is still working on run {run}. Wait for its {REPORT_PREFIX} before sending more."
                    ));
                }
                // Claimed under the lock the busy check was made under, so
                // two parallel asks cannot both pass it.
                let run = state.store.begin_run(&track, &name, &live.agent, &text, &worker_cwd).map_err(|e| e.to_string())?;
                live.turns += 1;
                live.running = Some(run.clone());
                live.used = std::time::Instant::now();
                Found::Live { run, turns: live.turns, session: live.session.clone(), agent: live.agent.clone() }
            }
            (None, _) => {
                let record = if fresh { None } else { worker_record(&state, &track, &name) };
                let history = state.store.sessions(&track).unwrap_or_default();
                let past = history.iter().find(|l| l.name == name);
                if !open && record.is_none() && past.is_none() {
                    let prefix = worker_key(&track, "");
                    let open_names: Vec<&str> = workers.keys().filter_map(|k| k.strip_prefix(&prefix)).collect();
                    let closed: Vec<&str> = history
                        .iter()
                        .filter(|l| l.name != CONDUCTOR_SESSION && !workers.contains_key(&worker_key(&track, &l.name)))
                        .map(|l| l.name.as_str())
                        .collect();
                    return Err(format!(
                        "no worker {name}. Open workers: {open_names:?}. Closed workers: {closed:?}. Use spawn_worker to create one."
                    ));
                }
                // Agent: the caller's choice, else the worker's earlier one, else
                // the track's worker agent.
                let agent_id = match (agent, &record, past) {
                    (Some(a), _, _) => a,
                    (None, Some(r), _) => r.agent.clone(),
                    (None, None, Some(p)) if !fresh => p.agent.clone(),
                    _ => info.effective_worker_agent().to_string(),
                };
                // Memory only carries over on the agent that made it.
                let resume = record.filter(|r| r.agent == agent_id).map(|r| r.session_id);
                let wanted = resume.is_some();
                let spec: AgentSpec = state.spec_for(&agent_id)?;
                // The track's worker options are in its worker agent's terms;
                // a worker on some other agent gets that agent's defaults.
                let empty = BTreeMap::new();
                let chosen = if agent_id == info.effective_worker_agent() { info.effective_worker_config() } else { &empty };
                let mut opts = session_options(&state, &agent_id, &worker_cwd, chosen, None);
                opts.resume = resume;
                state.sessions.opening.lock().insert(key.clone());
                // Reopened on another agent than before (not a fresh start):
                // it reads its earlier turns, as the record has them.
                let before = if fresh { None } else { worker_record(&state, &track, &name).map(|r| r.agent).or_else(|| past.map(|p| p.agent.clone())) };
                let handoff = before.filter(|b| *b != agent_id).map(|b| handoff_text(&state, &track, &name, &b, None));
                Found::Open { agent_id, spec, opts, wanted, past_runs: past.map(|p| p.runs), handoff }
            }
        }
    };
    let mut handoff_for_worker: Option<String> = None;
    let (agent_id, turns, session, resumed, note, run) = match found {
        Found::Live { run, turns, session, agent } => (agent, turns, session, false, None, run),
        Found::Open { agent_id, spec, opts, wanted, past_runs, handoff } => {
            handoff_for_worker = handoff;
            // The mark goes however the opening ends.
            struct Opening<'a>(&'a parking_lot::Mutex<HashSet<String>>, String);
            impl Drop for Opening<'_> {
                fn drop(&mut self) {
                    self.0.lock().remove(&self.1);
                }
            }
            let _opening = Opening(&state.sessions.opening, key.clone());
            tracing::info!(%track, worker = %name, agent = %agent_id, resume = ?opts.resume, "opening worker session");
            let session = Arc::new(AgentSession::open(&spec, opts).await.map_err(|e| e.to_string())?);
            let resumed = session.resumed();
            remember_worker(
                &state,
                &track,
                &name,
                &WorkerRecord {
                    session_id: session.session_id().to_string(),
                    agent: agent_id.clone(),
                },
            );
            let turns = if resumed { past_runs.unwrap_or(0) + 1 } else { 1 };
            let note = if resumed {
                Some("Reopened with its earlier conversation.")
            } else if wanted {
                Some("Its earlier conversation could not be restored; the worker starts fresh.")
            } else if past_runs.is_some() {
                Some("Started fresh; earlier runs stay in the record but the worker does not remember them.")
            } else {
                None
            };
            let run = state.store.begin_run(&track, &name, &agent_id, &text, &worker_cwd).map_err(|e| e.to_string())?;
            state.sessions.workers.lock().await.insert(
                key.clone(),
                Live {
                    agent: agent_id.clone(),
                    cwd: worker_cwd.clone(),
                    session: session.clone(),
                    turns,
                    running: Some(run.clone()),
                    used: std::time::Instant::now(),
                },
            );
            (agent_id, turns, session, resumed, note, run)
        }
    };

    // The turn itself, in the background. The task ends with how to report;
    // the stored prompt stays what the conductor wrote.
    let app_for_turn = app.clone();
    let (track_t, name_t, run_t, cwd_t, agent_t) = (track.clone(), name.clone(), run.clone(), worker_cwd.clone(), agent_id.clone());
    let task = format!("{text}\n\n---\n{}\n\n{}", place.for_worker, report::instructions());
    let task = match handoff_for_worker {
        Some(h) => format!("{h}\n\n---\n\n{task}"),
        None => task,
    };
    tauri::async_runtime::spawn(async move {
        let app = app_for_turn;
        drive_worker_turn(&app, &track_t, &name_t, &run_t, &session, task, &cwd_t).await;
        let mut outcome = checked_report(&app, &run_t);
        let mut current = run_t.clone();
        // A turn that ended well but without a usable block is asked once more,
        // in the same session, for the block alone.
        if let Err(problem) = &outcome {
            if ended_well(&app, &run_t) {
                let again_text = report::reminder(problem);
                let again = app.state::<AppState>().store.begin_run(&track_t, &name_t, &agent_t, &again_text, &cwd_t);
                match again {
                    Ok(again) => {
                        hand_running(&app, &track_t, &name_t, &current, Some(again.clone())).await;
                        current = again.clone();
                        drive_worker_turn(&app, &track_t, &name_t, &again, &session, again_text, &cwd_t).await;
                        outcome = checked_report(&app, &again).map(|mut r| {
                            r.reminder_run = Some(again.clone());
                            r
                        });
                    }
                    Err(err) => tracing::warn!(%err, "could not ask the worker again for its report"),
                }
            }
        }
        hand_running(&app, &track_t, &name_t, &current, None).await;
        let report = finish_report(&app, &run_t, (current != run_t).then_some(current.as_str()), outcome, &cwd_t);
        report_to_conductor(app, track_t, name_t, run_t, report).await;
    });

    let mut result = json!({
        "run": run,
        "worker": name,
        "agent": agent_id,
        "turn": turns,
        "resumed": resumed,
        "status": "running",
        "folder": folder,
        "note": format!("The worker is working. Its report will arrive as a {REPORT_PREFIX} message; tell the human what you delegated and end your turn."),
    });
    if let Some(note) = note {
        result["memory"] = Value::String(note.to_string());
    }
    Ok(result)
}

/// One worker turn: the prompt in flight, its events stored and shown.
async fn drive_worker_turn(app: &AppHandle, track: &str, name: &str, run: &str, session: &AgentSession, text: String, cwd: &str) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let pump_task = tauri::async_runtime::spawn(pump(app.clone(), track.to_string(), name.to_string(), run.to_string(), rx));
    let _ = tx.send(AgentEvent::Started {
        session_id: session.session_id().to_string(),
        cwd: cwd.to_string(),
    });
    match tokio::time::timeout(WORKER_TURN_TIMEOUT, session.prompt(text, tx.clone())).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let _ = tx.send(AgentEvent::Failed { error: err.to_string() });
        }
        Err(_) => {
            // A turn an hour long is stuck; a cancel may not reach an agent
            // that has stopped answering, so the session ends and the worker
            // reopens (with its memory) on its next task.
            session.cancel();
            session.kill();
            let _ = tx.send(AgentEvent::Failed { error: "worker turn timed out; its session was ended".to_string() });
            let st = app.state::<AppState>();
            let mut workers = st.sessions.workers.lock().await;
            if workers.get(&worker_key(track, name)).is_some_and(|l| std::ptr::eq(Arc::as_ptr(&l.session), session)) {
                workers.remove(&worker_key(track, name));
            }
        }
    }
    drop(tx);
    let _ = pump_task.await;
}

/// Move a worker's "turn in flight" from one run to the next (or to none).
async fn hand_running(app: &AppHandle, track: &str, name: &str, from: &str, to: Option<String>) {
    let state = app.state::<AppState>();
    let mut workers = state.sessions.workers.lock().await;
    if let Some(live) = workers.get_mut(&worker_key(track, name)) {
        if live.running.as_deref() == Some(from) {
            live.running = to;
            live.used = std::time::Instant::now();
        }
    }
}

/// Whether a run ended normally (not failed, cancelled or timed out).
fn ended_well(app: &AppHandle, run: &str) -> bool {
    matches!(app.state::<AppState>().store.run(run), Ok(Some(s)) if s.status == RunStatus::Done && !s.stop_reason.as_deref().is_some_and(|r| r.eq_ignore_ascii_case("cancelled")))
}

/// The report block of a run's reply, checked.
fn checked_report(app: &AppHandle, run: &str) -> Result<Report, String> {
    match app.state::<AppState>().store.run(run) {
        Ok(Some(s)) if s.status == RunStatus::Done => report::parse(&s.output),
        Ok(Some(s)) => Err(format!("the turn {}{}", s.status.as_str(), s.error.map(|e| format!(": {e}")).unwrap_or_default())),
        _ => Err("the run is gone".to_string()),
    }
}

/// The report as kept: the worker's, or one made from its reply; with the
/// files the app saw its edit tools touch. Stored for the timeline's card.
fn finish_report(app: &AppHandle, run: &str, reminder: Option<&str>, outcome: Result<Report, String>, cwd: &str) -> Report {
    let state = app.state::<AppState>();
    let mut report = outcome.unwrap_or_else(|problem| {
        let (well, reply) = match state.store.run(run) {
            Ok(Some(s)) => (s.status == RunStatus::Done, if s.output.trim().is_empty() { s.error.unwrap_or_default() } else { s.output }),
            _ => (false, String::new()),
        };
        report::Report::unstructured(well, &reply, &problem)
    });
    // A worker told to do no more work in the reminder may still have.
    for r in std::iter::once(run).chain(reminder) {
        let (inside, outside) = edits_seen(&state, r, cwd);
        for p in inside {
            if !report.edits_seen.contains(&p) {
                report.edits_seen.push(p);
            }
        }
        for p in outside {
            if !report.outside.contains(&p) {
                report.outside.push(p);
            }
        }
    }
    match serde_json::to_string(&report) {
        Ok(json) => {
            if let Err(err) = state.store.set_meta(&report_key(run), &json) {
                tracing::warn!(%run, %err, "could not keep the worker's report");
            }
        }
        Err(err) => tracing::warn!(%run, %err, "could not encode the worker's report"),
    }
    report
}

fn report_key(run: &str) -> String {
    format!("report:{run}")
}

/// A worker run's kept report, if it has one.
pub fn stored_report(state: &AppState, run: &str) -> Option<Report> {
    let json = state.store.get_meta(&report_key(run)).ok().flatten()?;
    serde_json::from_str(&json).ok()
}

/// Files a run's edit, delete and move tool calls touched: inside the
/// working folder (relative to it), and outside it.
fn edits_seen(state: &AppState, run: &str, cwd: &str) -> (Vec<String>, Vec<String>) {
    let root = cwd.replace('\\', "/").trim_end_matches('/').to_string() + "/";
    let mut seen: Vec<String> = Vec::new();
    let mut outside: Vec<String> = Vec::new();
    // Calls that change files; their updates may name the files later.
    let mut editing: HashSet<String> = HashSet::new();
    for e in state.store.events(run).unwrap_or_default() {
        let paths = match e.event {
            AgentEvent::ToolCall { id, tool_kind, paths, .. } => {
                if !matches!(tool_kind.as_str(), "edit" | "delete" | "move") {
                    continue;
                }
                editing.insert(id);
                paths
            }
            AgentEvent::ToolUpdate { id, paths, .. } if editing.contains(&id) => paths,
            _ => continue,
        };
        for p in paths {
            let p = p.replace('\\', "/");
            // Windows paths differ in case only; the prefix is compared bytewise so slicing stays on a boundary.
            let inside = p.is_char_boundary(root.len()) && p.get(..root.len()).is_some_and(|head| head.eq_ignore_ascii_case(&root));
            // A relative path is the agent's own, inside its folder, unless it climbs out.
            let relative = !p.starts_with('/') && !p.get(1..3).is_some_and(|s| s == ":/") && !climbs_out(&p);
            if inside || relative {
                let p = if inside { p[root.len()..].to_string() } else { p };
                if !seen.contains(&p) {
                    seen.push(p);
                }
            } else if !outside.contains(&p) {
                outside.push(p);
            }
        }
    }
    seen.truncate(50);
    outside.truncate(50);
    (seen, outside)
}

/// Whether a relative path leaves the folder it is relative to ("../x", "a/../../x").
fn climbs_out(rel: &str) -> bool {
    let mut depth: i32 = 0;
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}

/// A worker's own folder under the track's: the one it was given before;
/// for a worker that has worked in this track already, the folder named
/// after it (its own from before, even when nothing recorded it); for a new
/// worker, one named after it that the track does not have yet (a new
/// worker called "src" must not work in the project's src).
fn own_dir(state: &AppState, track: &str, track_dir: &str, name: &str) -> Result<std::path::PathBuf, String> {
    let key = format!("worker_dir:{track}/{name}");
    let dir = match state.store.get_meta(&key).ok().flatten() {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            let base = worktree::folder_name(name);
            let root = std::path::Path::new(track_dir);
            let worked_here = last_agent(state, track, name).is_some();
            let dir = if worked_here {
                root.join(&base)
            } else {
                (1..)
                    .map(|n| root.join(if n == 1 { base.clone() } else { format!("{base}-{n}") }))
                    .find(|d| !d.exists())
                    .expect("some name is free")
            };
            if let Err(err) = state.store.set_meta(&key, &dir.to_string_lossy()) {
                tracing::warn!(%err, "could not remember a worker's folder");
            }
            dir
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the worker's folder {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Where a worker works, and what the conductor and the worker are told of it.
struct Place {
    cwd: String,
    for_conductor: String,
    for_worker: String,
}

/// The worker's folder, per the track's choice: its own folder under the
/// track's (made here), its own git checkout (made or brought up to date;
/// outside git, or when that fails, its own folder instead), or the track
/// folder itself.
async fn worker_place(app: &AppHandle, info: &TrackInfo, name: &str) -> Result<Place, String> {
    let track_dir = info.cwd.clone();
    let own_folder = |why: Option<String>| -> Result<Place, String> {
        let dir = own_dir(&app.state::<AppState>(), &info.id, &track_dir, name)?;
        let dir = dir.to_string_lossy().into_owned();
        Ok(Place {
            for_conductor: format!("its own folder {dir}{}", why.map(|w| format!(" ({w})")).unwrap_or_default()),
            for_worker: format!(
                "Your folder is {dir}. Write everything you make or change inside it. The folder around it, {track_dir}, is shared with the human and other workers: read from it as you need, but do not write outside your folder."
            ),
            cwd: dir,
        })
    };
    match info.worker_folder.as_str() {
        "shared" => Ok(Place {
            cwd: track_dir.clone(),
            for_conductor: "the track folder, shared with other workers".to_string(),
            for_worker: format!("You work in {track_dir}, shared with the human and other workers: change only the files your task is about."),
        }),
        "worktree" => {
            let (app2, track, worker, cwd) = (app.clone(), info.id.clone(), name.to_string(), track_dir.clone());
            let checkout = tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                worktree::ensure(&state.store, &state.worktrees_dir, &track, &worker, std::path::Path::new(&cwd))
            })
            .await
            .map_err(|e| e.to_string())?;
            match checkout {
                Ok(Some(c)) => {
                    let dir = c.cwd().to_string_lossy().into_owned();
                    Ok(Place {
                        for_conductor: format!("its own git checkout (branch {}); the human merges its changes into the track folder", c.branch),
                        for_worker: format!(
                            "You work in your own checkout of the repository at {dir}; the human merges your changes into the track folder {track_dir}. Files the human made there but has not added to git are not in your checkout: when you are told of one, read it from {track_dir}. Write only in your checkout."
                        ),
                        cwd: dir,
                    })
                }
                Ok(None) => own_folder(Some("the track is not a git repository, so no checkout".to_string())),
                Err(err) => {
                    tracing::warn!(%err, worker = %name, "could not make the worker's checkout; it works in a folder of its own");
                    own_folder(Some(format!("its checkout failed: {err}")))
                }
            }
        }
        _ => own_folder(None),
    }
}

/// Hand a finished worker run to its track's conductor as a new turn: the
/// report, not the whole reply (that stays below the membrane, in read_report).
async fn report_to_conductor(app: AppHandle, track: String, worker: String, run: String, report: Report) {
    let state = app.state::<AppState>();
    let summary = match state.store.run(&run) {
        Ok(Some(s)) => s,
        _ => {
            tracing::warn!(%run, "worker run vanished before reporting");
            return;
        }
    };
    let text = format!(
        "{REPORT_PREFIX} worker={worker} run={run} status={} tools={} duration_ms={}{}\n\n{}",
        summary.status.as_str(),
        summary.tool_count,
        summary.duration_ms.unwrap_or(0),
        summary.error.as_ref().map(|e| format!(" error={e}")).unwrap_or_default(),
        report.for_conductor(&run),
    );

    deliver(app.clone(), track, text, false, format!("report of {worker} run {run}")).await;
}

/// The track's conductor session, opened (or reopened after its agent or
/// options changed) when it is not there. With `take_turn`, the turn count
/// moves and the second value says whether this is the session's first
/// turn, which is when the preamble goes in.
async fn open_conductor(app: &AppHandle, track: &str, info: &TrackInfo, take_turn: bool) -> Result<(Arc<AgentSession>, bool), String> {
    let st = app.state::<AppState>();
    let agent = info.agent.clone();
    let wanted = fingerprint(&agent, &info.conductor_config);
    let gate = st.sessions.conductor_gate(track);
    let _one_opener = gate.lock().await;
    let (needs_open, old) = {
        let mut guard = st.sessions.conductors.lock().await;
        let needs_open = match guard.get(track) {
            Some(c) => c.fingerprint != wanted,
            None => true,
        };
        let old = if needs_open { guard.remove(track) } else { None };
        (needs_open, old)
    };
    if needs_open {
        // Another agent takes over: the one before leaves a note (when its
        // session is still open) and the recent conversation, which the new
        // one reads before its first message.
        let previous = old.as_ref().map(|c| c.live.agent.clone()).or_else(|| last_agent(&st, track, CONDUCTOR_SESSION));
        if let Some(previous) = previous.filter(|p| *p != agent) {
            let note = match &old {
                Some(c) => write_handoff_note(app, track, &c.live.session, &previous, &agent).await,
                None => None,
            };
            let text = handoff_text(&st, track, CONDUCTOR_SESSION, &previous, note.as_deref());
            if let Err(err) = st.store.set_meta(&handoff_key(track), &text) {
                tracing::warn!(%err, "could not keep the handoff");
            }
        }
        if let Some(old) = old {
            tracing::info!(%track, agent = %old.live.agent, "closing conductor session (agent or options changed)");
            drop(old);
        }
        let spec = st.spec_for(&agent)?;
        // This track's tools, on their own server: every call is scoped.
        let mcp = McpServer::start("divixi", tools(app.clone(), track.to_string()))
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
        st.sessions.conductors.lock().await.insert(
            track.to_string(),
            Conductor {
                live: Live {
                    agent: agent.clone(),
                    cwd: info.cwd.clone(),
                    session: Arc::new(session),
                    // A resumed session already had its preamble.
                    turns: if resumed { 1 } else { 0 },
                    running: None,
                    used: std::time::Instant::now(),
                },
                fingerprint: wanted,
                _mcp: mcp,
            },
        );
    }
    let mut guard = st.sessions.conductors.lock().await;
    // Closed meanwhile (the track was deleted or moved): nothing to talk to.
    let live = &mut guard.get_mut(track).ok_or("the conductor session was closed")?.live;
    if take_turn {
        live.turns += 1;
    }
    live.used = std::time::Instant::now();
    Ok((live.session.clone(), take_turn && live.turns == 1))
}

fn handoff_key(track: &str) -> String {
    format!("handoff:{track}")
}

/// The handoff waiting for a track's new conductor, taken (it is read once).
fn take_handoff(st: &AppState, track: &str) -> Option<String> {
    let text = st.store.get_meta(&handoff_key(track)).ok().flatten()?;
    let _ = st.store.delete_meta(&handoff_key(track));
    Some(text).filter(|t| !t.trim().is_empty())
}

/// The agent of a session's latest run in the record.
fn last_agent(st: &AppState, track: &str, session: &str) -> Option<String> {
    st.store.sessions(track).ok()?.into_iter().find(|s| s.name == session).map(|s| s.agent)
}

/// How long the agent being replaced has to write its note.
const HANDOFF_NOTE_TIME: Duration = Duration::from_secs(90);

/// Ask the conductor being replaced for a note to its successor. Not a run
/// of the track: nothing of it is stored but the note. `None` when it did
/// not write one in time.
async fn write_handoff_note(app: &AppHandle, track: &str, session: &AgentSession, from: &str, to: &str) -> Option<String> {
    #[derive(Clone, serde::Serialize)]
    struct Handoff<'a> {
        track: &'a str,
        from: &'a str,
        to: &'a str,
        writing: bool,
    }
    let _ = app.emit("conductor_handoff", Handoff { track, from, to, writing: true });
    let ask = format!(
        "The human is switching this track's conductor from you to another agent ({to}). Write a handoff note for your successor, in the language of your conversation with the human: what the track is for, what has been decided and why, what is in progress or delegated (which workers, doing what, where their results are), what waits on the human, open questions, and anything you would tell a colleague taking over. Markdown, at most about 400 words. Do not call tools and do no other work: only the note."
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let collect = async {
        let mut text = String::new();
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Message { text: t } = ev {
                text.push_str(&t);
            }
        }
        text
    };
    let turn = async {
        let out = tokio::time::timeout(HANDOFF_NOTE_TIME, session.prompt(ask, tx)).await;
        if out.is_err() {
            session.cancel();
        }
    };
    let ((), note) = tokio::join!(turn, collect);
    let _ = app.emit("conductor_handoff", Handoff { track, from, to, writing: false });
    let note = note.trim().to_string();
    tracing::info!(%track, %from, %to, chars = note.chars().count(), "handoff note written");
    (!note.is_empty()).then_some(note)
}

/// What a session's successor on another agent reads first: the note, if
/// any, and the session's recent turns from the record (about 8k chars,
/// newest kept).
fn handoff_text(st: &AppState, track: &str, session: &str, from: &str, note: Option<&str>) -> String {
    const BUDGET: usize = 8_000;
    const PER_TURN: usize = 1_500;
    let clip = |s: &str| {
        let s = s.trim();
        if s.chars().count() > PER_TURN {
            format!("{}…", s.chars().take(PER_TURN).collect::<String>())
        } else {
            s.to_string()
        }
    };
    let runs: Vec<_> = st
        .store
        .runs()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.track == track && r.session == session)
        .collect();
    let mut turns: Vec<String> = Vec::new();
    let mut used = 0;
    for r in runs.iter().rev().take(20) {
        let who = if r.prompt.starts_with(REPORT_PREFIX) || r.prompt.starts_with(DECISION_PREFIX) || r.prompt.starts_with(PERMISSION_PREFIX) {
            "app"
        } else if session == CONDUCTOR_SESSION {
            "human"
        } else {
            "conductor"
        };
        let mut turn = format!("### {who}\n{}\n", clip(&r.prompt));
        if !r.output.trim().is_empty() {
            turn.push_str(&format!("### {} ({})\n{}\n", if session == CONDUCTOR_SESSION { "conductor" } else { "worker" }, r.agent, clip(&r.output)));
        }
        if used + turn.len() > BUDGET && !turns.is_empty() {
            break;
        }
        used += turn.len();
        turns.push(turn);
    }
    turns.reverse();
    let role = if session == CONDUCTOR_SESSION { "this track's conductor" } else { "this worker" };
    format!(
        "[handoff]\nYou take over as {role} from {from}: the human switched agents. You do not have its memory; below is {} and the recent conversation, oldest first. Read it, then answer the message after it.\n\n## Its note\n{}\n\n## Recent turns\n{}[/handoff]",
        if note.is_some() { "the note it left" } else { "what the record has (it left no note)" },
        note.unwrap_or("(none)"),
        if turns.is_empty() { "(none)\n".to_string() } else { turns.join("\n") },
    )
}

/// What the UI shows about a track's conductor session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConductorState {
    pub open: bool,
    pub busy: bool,
    pub agent: Option<String>,
    pub commands: Vec<orchestra_core::SlashCommand>,
}

/// The conductor's state, without opening anything.
pub async fn conductor_state(app: &AppHandle, track: &str) -> ConductorState {
    let st = app.state::<AppState>();
    let guard = st.sessions.conductors.lock().await;
    let live = guard.get(track).map(|c| &c.live);
    ConductorState {
        open: live.is_some(),
        busy: st.sessions.is_busy(track),
        agent: live.map(|l| l.agent.clone()),
        commands: live.map(|l| l.session.commands()).unwrap_or_default(),
    }
}

/// Every track with an open conductor session, with its state.
pub async fn conductor_states(app: &AppHandle) -> Vec<(String, ConductorState)> {
    let st = app.state::<AppState>();
    let guard = st.sessions.conductors.lock().await;
    guard
        .iter()
        .map(|(track, c)| {
            (
                track.clone(),
                ConductorState {
                    open: true,
                    busy: st.sessions.is_busy(track),
                    agent: Some(c.live.agent.clone()),
                    commands: c.live.session.commands(),
                },
            )
        })
        .collect()
}

/// Open the track's conductor session ahead of a message (so its slash
/// commands are known and the first turn is quick). A session already
/// open, or busy with a turn, is left as it is.
pub async fn conductor_open(app: AppHandle, track: String) -> Result<ConductorState, String> {
    let st = app.state::<AppState>();
    if !st.sessions.is_busy(&track) {
        let info = track_info(&st, &track)?;
        open_conductor(&app, &track, &info, false).await?;
    }
    Ok(conductor_state(&app, &track).await)
}

/// Stop the conductor's turn in flight (Ctrl+C). The session stays open;
/// the run ends with the agent's `cancelled` stop reason.
pub async fn conductor_cancel(app: AppHandle, track: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    if !st.sessions.is_busy(&track) {
        return Ok(());
    }
    let guard = st.sessions.conductors.lock().await;
    let Some(c) = guard.get(&track) else {
        return Err("no conductor session".to_string());
    };
    c.live.session.cancel();
    let (turn, session) = (c.live.turns, c.live.session.clone());
    drop(guard);
    // An agent that does not end the turn after a cancel is stuck: after a
    // grace its session is ended, so the track is free again; the next
    // message reopens it with its memory.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CANCEL_GRACE).await;
        let st = app.state::<AppState>();
        if !st.sessions.is_busy(&track) {
            return;
        }
        let mut guard = st.sessions.conductors.lock().await;
        let same_turn = guard.get(&track).is_some_and(|c| c.live.turns == turn && Arc::ptr_eq(&c.live.session, &session));
        if same_turn {
            tracing::warn!(%track, "the conductor did not stop after a cancel; ending its session");
            if let Some(old) = guard.remove(&track) {
                old.live.session.kill();
            }
        }
    });
    Ok(())
}

/// Close conductor and worker sessions nobody has used for a while (the
/// `sessions.idle_minutes` setting, 30 by default, 0 for never): each is an
/// agent process holding memory. Their conversations stay in the store and
/// the agent's own record, so the next message reopens them with memory.
pub fn sweep_idle(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let state = app.state::<AppState>();
            let minutes = state
                .store
                .get_meta(&format!("{}sessions.idle_minutes", crate::SETTING_PREFIX))
                .ok()
                .flatten()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(DEFAULT_IDLE_MINUTES);
            if minutes == 0 {
                continue;
            }
            let limit = Duration::from_secs(minutes * 60);
            {
                let mut conductors = state.sessions.conductors.lock().await;
                let idle: Vec<String> = conductors
                    .iter()
                    .filter(|(track, c)| !state.sessions.is_busy(track) && c.live.running.is_none() && c.live.used.elapsed() > limit)
                    .map(|(track, _)| track.clone())
                    .collect();
                for track in idle {
                    tracing::info!(%track, minutes, "closing an idle conductor session");
                    conductors.remove(&track);
                }
            }
            let mut workers = state.sessions.workers.lock().await;
            let idle: Vec<String> = workers
                .iter()
                .filter(|(_, l)| l.running.is_none() && l.used.elapsed() > limit)
                .map(|(key, _)| key.clone())
                .collect();
            for key in idle {
                tracing::info!(worker = %key, minutes, "closing an idle worker session");
                workers.remove(&key);
            }
        }
    });
}

/// Minutes a session may sit unused before it is closed, when not set.
const DEFAULT_IDLE_MINUTES: u64 = 30;

/// How long an agent has to end a turn after a cancel before its session is ended.
const CANCEL_GRACE: Duration = Duration::from_secs(15);

/// Close the track's conductor session. Its memory stays in the store, so
/// the next open resumes it. Refused while a turn is running.
pub async fn conductor_close(app: AppHandle, track: String) -> Result<ConductorState, String> {
    let st = app.state::<AppState>();
    if st.sessions.is_busy(&track) {
        return Err("the conductor is still responding".to_string());
    }
    if let Some(old) = st.sessions.conductors.lock().await.remove(&track) {
        tracing::info!(%track, agent = %old.live.agent, "closing conductor session (asked)");
        drop(old);
    }
    Ok(conductor_state(&app, &track).await)
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
    files: Vec<PathBuf>,
) -> Result<String, String> {
    let prompt = with_attachments(&prompt, &files);
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

    if !st.sessions.begin_turn(&track) {
        return Err(BUSY.to_string());
    }

    let outcome: Result<String, String> = async {
        let (session, first) = open_conductor(&app, &track, &info, true).await?;
        let session_id = session.session_id().to_string();

        let run = st
            .store
            .begin_run(&track, CONDUCTOR_SESSION, &agent, &prompt, &info.cwd)
            .map_err(|e| e.to_string())?;
        // Tools called during the turn (decisions) hang off this run.
        if let Some(c) = st.sessions.conductors.lock().await.get_mut(&track) {
            c.live.running = Some(run.clone());
        }

        let text = match take_handoff(&st, &track) {
            Some(handoff) => format!("{handoff}\n\n---\n\n{prompt}"),
            None => prompt,
        };
        let text = if first { format!("{}\n\n---\n\n{text}", preamble(&lang, &info)) } else { text };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tauri::async_runtime::spawn(pump(app.clone(), track.clone(), CONDUCTOR_SESSION.to_string(), run.clone(), rx));
        let _ = tx.send(AgentEvent::Started {
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
            let result = session.prompt_with(text, files, tx.clone()).await;
            tracing::info!(run = %run_for_turn, elapsed_ms = started.elapsed().as_millis() as u64, ok = result.is_ok(), "conductor turn ended");
            {
                let mut conductors = st.sessions.conductors.lock().await;
                if let Err(err) = result {
                    let _ = tx.send(AgentEvent::Failed { error: err.to_string() });
                    // A failed turn kills the session; drop it so the next prompt reopens.
                    conductors.remove(&track_for_turn);
                } else if let Some(c) = conductors.get_mut(&track_for_turn) {
                    if c.live.running.as_deref() == Some(run_for_turn.as_str()) {
                        c.live.running = None;
                    }
                    c.live.used = std::time::Instant::now();
                }
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

#[cfg(test)]
mod folder_tests {
    #[test]
    fn relative_paths_that_climb_out() {
        assert!(super::climbs_out("../x.md"));
        assert!(super::climbs_out("a/../../x"));
        assert!(!super::climbs_out("a/../b.md"));
        assert!(!super::climbs_out("./notes/x.md"));
    }
}
