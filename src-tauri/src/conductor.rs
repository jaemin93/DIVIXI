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
use orchestra_agents::{AgentKind, AgentStatus, Readiness};
use orchestra_core::report::{self, RanOn, Report};
use orchestra_core::{AgentEvent, RunStatus};
use orchestra_mcp::{McpServer, Tool};
use orchestra_store::{Decision, DecisionOption, DecisionStatus, NewDecision, PermissionAsk, RunSummary, TrackInfo};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::AppHandle;
use tokio::sync::{oneshot, Mutex};

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

/// A worker run waits at most this long for the agent to finish a turn,
/// however busy it looks.
///
/// This is the last defence, not the usual one: [`WORKER_QUIET_LIMIT`]
/// catches a worker that has stopped answering, and it catches it in
/// twenty minutes. What it cannot catch is an agent that started a tool
/// call and never reported its end — nothing outstanding ever clears, so
/// the quiet clock never runs. That is what this is for.
///
/// It was an hour, and an hour was too close to the work. Two workers in
/// one session here took 58 and 57 minutes: a task slightly larger than
/// those would have had a worker killed in the middle of doing its job
/// properly. Four hours leaves that room. Cutting a working worker off is
/// worse than taking its report late, and with the quiet clock in front
/// of it this cap only ever fires on a turn nothing else could see was
/// wrong.
const WORKER_TURN_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);

/// How long a worker may say nothing at all before its turn is taken as
/// stuck — but only while it has no tool call running and no permission
/// question waiting, so a long build or test says nothing and is left alone
/// (a release build here takes six minutes, `cargo test` far longer).
///
/// Twenty minutes is for the other silence: the agent between its own steps,
/// thinking or waiting on its model. Minutes there, not tens of minutes.
/// Killing a worker that is working is worse than taking its report late,
/// so this errs long and only ever fires on a worker that is doing nothing
/// the app can see.
const WORKER_QUIET_LIMIT: Duration = Duration::from_secs(20 * 60);

/// How often a worker turn is looked at while it runs.
const WORKER_QUIET_CHECK: Duration = Duration::from_secs(30);

/// How long a worker waits on its proposal card before it starts on the
/// track's own agent and model instead.
///
/// A proposal card is the one kind of card that must not be able to stop
/// the work. The others cannot be defaulted: a permission question is
/// about something irreversible, and a `request_decision` is a fork the
/// conductor genuinely cannot take alone. A proposal is different in kind —
/// there is always a right answer waiting underneath it, the track's own
/// choice, which the human set themselves and which is exactly what every
/// worker used before this feature existed. Falling back to it loses an
/// optimisation, not the work.
///
/// So the cost of the two mistakes is not symmetric. Waiting for someone
/// who has gone to bed stops a night of delegated work dead. Starting on
/// the track's default runs the same task the same way it would have run
/// yesterday, and it is cheap to undo: the worker reopens on another agent
/// by name, and [`handoff_text`] carries its conversation across. Ten
/// minutes is long enough for someone at their desk to answer and short
/// enough that nobody finds a night's work never started.
///
/// The card says this deadline out loud, so an unanswered card is never a
/// surprise, and the worker's report says what it ran on in the end.
const PROPOSAL_WAIT: Duration = Duration::from_secs(10 * 60);

/// How long a turn for the conductor spins waiting for it to be free before
/// it is written down instead (see [`park`]) and handed on by one of the
/// flush triggers. Short: spinning costs a wake-up twice a second and buys
/// nothing that parking does not, now that parking outlives the app.
const REPORT_WAIT: Duration = Duration::from_secs(60);
/// What `conductor_turn` returns while an earlier turn is still running.
///
/// The `busy:` head is the machine-readable half, for callers that must
/// tell "not now" from "cannot": [`deliver`] and [`flush_parked`] compare
/// against this constant, and the UI's `stillAnswering` (store.svelte.ts)
/// recognises the message so a queued chat line waits for the turn to end
/// instead of being handed back to the human as a failure.
///
/// **Whoever edits this string:** the UI matches on it. Keep the `busy:`
/// prefix and the words "still responding", or change `stillAnswering` in
/// the same commit — otherwise a queued message quietly stops queueing.
pub(crate) const BUSY: &str = "busy: conductor is still responding";

/// One open agent session and what it runs on.
pub struct Live {
    pub agent: String,
    /// The agent's session options this session was opened with (`option id
    /// → value id`), so what it is running on can be shown and reported
    /// without reopening anything.
    pub config: BTreeMap<String, String>,
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
    /// Runs the human stopped by hand, so the record and the report say
    /// "stopped" and not "failed". Forgotten once the run has reported.
    stopped: parking_lot::Mutex<HashSet<String>>,
    /// Spawns held back by a proposal card, by decision id.
    ///
    /// The human's answer does not go to the conductor — the conductor
    /// already ended its turn and is not waiting on anything. It goes here,
    /// to the task holding the worker back, which then opens it.
    proposals: parking_lot::Mutex<HashMap<i64, oneshot::Sender<Option<usize>>>>,
    /// Cards the human answered in their own words, by decision id, holding
    /// the worker each was about. The conductor reads the answer, works out
    /// which agent and model it means, and spawns again quoting the id;
    /// [`Sessions::take_settled`] is what lets that one call through the
    /// card, once.
    settled: parking_lot::Mutex<HashMap<i64, String>>,
    /// Workers whose record is being deleted, by `track/worker` key.
    ///
    /// Deleting takes a while — a git call reads the worker's checkout for
    /// unmerged work — and closing its session does not stop the conductor
    /// from opening it again with `ask_worker` meanwhile. A turn started in
    /// that window would have its run deleted out from under it: its events
    /// would fail the foreign key on `runs`, and an hour of work would go
    /// with no record at all. So the name is held here for the length of
    /// the delete and [`start_worker_turn`] refuses it.
    deleting: parking_lot::Mutex<HashSet<String>>,
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

    /// Wait for a proposal card's answer. The receiver ends with the chosen
    /// option's index, or `None` when the card was set aside.
    fn await_proposal(&self, decision: i64) -> oneshot::Receiver<Option<usize>> {
        let (tx, rx) = oneshot::channel();
        self.proposals.lock().insert(decision, tx);
        rx
    }

    /// Hand a proposal card's outcome to the spawn waiting on it. `true` when
    /// one was registered, which is also what tells a proposal card from an
    /// ordinary decision the conductor should be told about.
    ///
    /// True even when the send fails. A spawn that gave up on the deadline a
    /// moment ago has dropped its receiver but the card is still a proposal,
    /// and handing it to the conductor as an ordinary decision would have it
    /// reading an answer to a question it never asked.
    fn resolve_proposal(&self, decision: i64, choice: Option<usize>) -> bool {
        match self.proposals.lock().remove(&decision) {
            Some(tx) => {
                let _ = tx.send(choice);
                true
            }
            None => false,
        }
    }

    /// Whether a spawn is still held back by this card.
    fn awaits_proposal(&self, decision: i64) -> bool {
        self.proposals.lock().contains_key(&decision)
    }

    /// Note that a card was answered in the human's own words, so the one
    /// spawn carrying their answer back may pass the card.
    fn mark_settled(&self, decision: i64, worker: &str) {
        self.settled.lock().insert(decision, worker.to_string());
    }

    /// The worker such a card was about, without spending it.
    fn settled_worker(&self, decision: i64) -> Option<String> {
        self.settled.lock().get(&decision).cloned()
    }

    /// Spend that note: the worker it was about, if this is really one.
    fn take_settled(&self, decision: i64) -> Option<String> {
        self.settled.lock().remove(&decision)
    }

    fn mark_stopped(&self, run: &str) {
        self.stopped.lock().insert(run.to_string());
    }

    /// Whether the human stopped this run by hand.
    pub fn was_stopped(&self, run: &str) -> bool {
        self.stopped.lock().contains(run)
    }

    fn forget_stopped(&self, run: &str) {
        self.stopped.lock().remove(run);
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
    /// Close every session there is, across every track. Returns how many were
    /// closed, for the log.
    ///
    /// For process shutdown. Two things per session, and both matter: `cancel`
    /// interrupts a turn that is mid-flight, and dropping the last `Arc` is what
    /// actually ends the session -- the task's `turn_rx` closes, its loop
    /// returns, and it runs `process.shutdown().await`, which is where the
    /// agent's process group is taken down.
    ///
    /// Draining the maps here is therefore not bookkeeping, it is the mechanism.
    /// The tasks then unwind in parallel, so a caller waits one grace period
    /// rather than one per session.
    ///
    /// Both ways out of the app use this: the server's signal handler and the
    /// desktop's tray Quit. Neither can rely on destructors -- `process::exit`
    /// and `app.exit` do not unwind the tasks that hold the agents -- so the
    /// sessions have to be closed on the way out, on purpose.
    ///
    /// Windows would survive without it, since the kernel closes each agent's
    /// job object when the process dies. Unix has no such backstop: the agents
    /// are in process groups of their own and simply keep running.
    pub async fn shutdown_all(&self) -> usize {
        let mut closed = 0;
        for (_, conductor) in self.conductors.lock().await.drain() {
            conductor.live.session.cancel();
            closed += 1;
        }
        for (_, live) in self.workers.lock().await.drain() {
            live.session.cancel();
            closed += 1;
        }
        closed
    }

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

/// The tracks named by a set of worker session keys.
///
/// The inverse of [`worker_key`], and the reason it is a function: a worker's
/// name is the human's, so it can hold a `/`, and only the first one divides.
fn tracks_of<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    keys.into_iter().filter_map(|key| key.split_once('/').map(|(track, _)| track.to_string())).collect()
}

/// Where a track's workers work, as the conductor is told.
enum Folders {
    Own,
    Checkout,
    Shared,
}

/// What the conductor is told about the agents it may put workers on.
///
/// Facts only, and deliberately so. Which agent is better at what changes
/// faster than this app ships, has never been measured here, and a ranking
/// written into a prompt is a claim Divixi would then have to keep standing
/// behind. So the app says what exists — which agents are installed and
/// logged in, and what each one offers — and says nothing about which is
/// good at what. The conductor judges the work; the human judges the agent,
/// on the card.
///
/// Empty unless the track lets the conductor propose: telling it about
/// agents it may not choose would only invite it to try.
fn agent_facts(state: &AppState, lang: &str, track: &TrackInfo) -> String {
    if !track.proposes_worker_choice() {
        return String::new();
    }
    let ko = lang.starts_with("ko");
    let mut lines: Vec<String> = Vec::new();
    for status in agent_choices(state) {
        let options = state.config_options_for(status.kind.id());
        let models = options.iter().find(|o| o.category == "model");
        let detail = match models {
            Some(o) if !o.choices.is_empty() => {
                // The value id is what `spawn_worker` takes, so it leads;
                // the name and the agent's own description follow, and the
                // description is the agent's words, not the app's.
                let names: Vec<String> = o
                    .choices
                    .iter()
                    .map(|c| match c.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                        Some(d) => format!("{} ({} — {d})", c.id, c.name),
                        None => format!("{} ({})", c.id, c.name),
                    })
                    .collect();
                if ko {
                    format!("모델: {}", names.join(", "))
                } else {
                    format!("models: {}", names.join(", "))
                }
            }
            // Its model comes from its own CLI settings; naming one here
            // would be refused, so the conductor is told not to try.
            _ => {
                if ko {
                    "모델 선택 없음 (model을 주지 마세요)".to_string()
                } else {
                    "no model choice (do not pass model)".to_string()
                }
            }
        };
        lines.push(format!("- {} — {detail}", status.kind.id()));
    }
    if lines.is_empty() {
        return String::new();
    }
    let default = describe(state, track.effective_worker_agent(), track.effective_worker_config());
    if ko {
        format!(
            "\n쓸 수 있는 에이전트(설치·로그인 확인됨). 기본은 {default} 입니다. model에는 괄호 앞의 값을 그대로 넘깁니다.\n{}\n어느 에이전트가 무엇을 더 잘하는지, 그리고 **어느 모델이 얼마인지는 앱이 알지 못합니다.** 위 목록에 없는 것은 추측하지 마세요. 당신이 판단할 것은 맡기려는 일의 성질이고, 에이전트와 모델은 사람이 카드에서 정합니다.\n",
            lines.join("\n")
        )
    } else {
        format!(
            "\nAgents you can use (installed and logged in). The default is {default}. Pass the value before the bracket as `model`.\n{}\nDivixi does not tell you which agent is better at what, and **it does not know what any model costs.** Do not guess at anything the list above does not say. What you judge is the kind of work; the human decides the agent and the model, on the card.\n",
            lines.join("\n")
        )
    }
}

/// What the conductor is told once, at the start of its session, in the
/// interface language. Only the wording differs; the rules are the same.
fn preamble(lang: &str, track: &TrackInfo, agents: &str) -> String {
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
            r#"당신은 DIVIXI의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 작업자(worker)라는 별도의 에이전트 세션에 맡깁니다.

트랙 이름: {name}{about}
규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 작업자를 부르지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_worker`로 작업자를 불러 맡깁니다. 작업자 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 작업자가 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- `spawn_worker`와 `ask_worker`는 작업자가 일을 받는 즉시 돌아옵니다. 결과를 기다리지 말고, 사람에게 무엇을 맡겼는지 한 문장으로 알린 뒤 턴을 끝냅니다. 작업자가 끝나면 `{REPORT_PREFIX}`로 시작하는 메시지가 당신에게 옵니다. 앱이 작업자에게 받은 정해진 보고입니다: status(done·partial·blocked·failed), summary, changes, checks(검증과 결과), risks, questions, next, 그리고 앱이 본 편집 중 보고에 없는 것. 무슨 일이 있었는지 한두 문단으로 사람에게 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다. checks가 없거나 실패가 있으면 그렇다고 말합니다. questions는 당신이 답할 수 있으면 `ask_worker`로 답하고, 사람이 정할 일이면 `request_decision`으로 올립니다. 작업자의 전체 답은 `read_report`에 있습니다.
- 같은 작업자에게 이어서 시킬 일은 `ask_worker`로 보냅니다. 작업자는 이전 대화를 기억합니다.
- 작업자 목록은 열린 것과 닫힌 것 모두 `worker_status`로 봅니다. 터미널이나 파일을 뒤져 작업자를 찾지 않습니다.
- 닫힌 작업자는 같은 이름으로 `spawn_worker`나 `ask_worker`를 부르면 이전 대화를 기억한 채 다시 열립니다. 기억을 버리고 처음부터 시작하려면 `spawn_worker`에 fresh=true를 줍니다. `close_worker`는 세션만 닫고 기록은 남깁니다.
- 끝난 작업자 세션이 쌓이면(`worker_status`의 idle_sessions) 사람에게 정리를 제안하고, 사람이 그러라고 한 것만 `close_worker`로 닫습니다. 기록을 지우는 일은 당신이 하지 않습니다 — 사람이 작업자 목록에서 직접 합니다. 어떤 경우에도 작업자의 작업 폴더와 그 안의 파일은 지워지지 않습니다.
- 도구가 오류를 돌려주면 오류 문구에 적힌 대로 한 번만 다시 시도하고, 그래도 안 되면 사람에게 무엇이 막혔는지 말합니다. 같은 도구를 반복해서 부르지 않습니다.
- 사람이 골라야 할 일(여러 갈래 중 선택, 되돌리기 어려운 변경, 취향이나 우선순위)은 스스로 정하지 않습니다. 선택지를 본문에 A/B/C로 늘어놓지 말고 `request_decision`을 부르세요. 앱이 선택지를 버튼이 있는 결정 카드로 보여 줍니다. 부른 뒤에는 무엇을 물었는지 한 문장만 말하고 턴을 끝냅니다. 사람의 답은 `{DECISION_PREFIX}`로 시작하는 메시지로 옵니다. 작업자 보고에 사람이 정해야 할 질문이 있으면 그것도 `request_decision`으로 올립니다. 사람이 대화 중에 직접 정한 것은 `record_decision`으로 남깁니다.
- 사람은 작업자와 직접 이야기하지 않습니다. 작업자가 무언가를 해도 되는지 물으면 `{PERMISSION_PREFIX}`로 시작하는 메시지로 당신에게 옵니다. 사람의 지시와 맡긴 일의 범위 안이면 당신이 직접 골라 `answer_worker`로 답합니다(허용할 때는 보통 이번만 허용). 되돌리기 어렵거나 맡긴 범위를 벗어나거나 사람이 정해야 할 일이면 `request_decision`으로 사람에게 묻고, 답이 오면 그대로 `answer_worker`로 전합니다. 작업자는 답을 받을 때까지 기다리므로 미루지 않습니다.
- 사람이 앞으로도 여러 번 하게 될 일이라고 말하면("되풀이로 만들자", "다음에도 이렇게", "매번 이 작업을") `save_routine`으로 저장합니다. 당신이 하는 일은 **지시문을 쓰는 것**입니다. 되풀이는 작업자가 아니라 **자기 에이전트로 자기 폴더에서 혼자 도는 것**이라, 작업자 세션도 이 대화도 함께 넘어가지 않습니다. 그러니 방금 작업자에게 준 말을 그대로 옮기지 말고, **이 대화 없이 몇 주 뒤에 읽어도 뜻이 통하도록 다시 씁니다** — "아까 그 파일", "앞서 말한 대로"를 빼고 파일 경로와 명령과 기준을 그대로 적고, 무엇이 좋은 답인지도 적습니다. 폴더와 에이전트는 비워 두면 이 트랙의 것을 씁니다. 저장한 뒤 이름과 함께 어디서 무엇으로 돌게 되는지, 그 셋 다 사이드바의 되풀이에서 사람이 바꿀 수 있다는 것을 한 문장으로 알립니다. 같은 일을 두 번 적지 않도록 `list_routines`로 먼저 봅니다. 지금 돌리려면 `run_routine`입니다.
- 사람이 고른 문서가 모인 지식 라이브러리가 있습니다. 사람이 "우리가 아는 것", 자기 문서·노트, 이름으로 특정 문서를 언급하거나, 맡기려는 일이 라이브러리가 다루는 주제에 닿으면 `knowledge_search`로 찾습니다(무엇이 있는지는 `knowledge_list_sources`). 일반적인 코딩 질문이나 작업 폴더만 봐도 되는 일에는 부르지 않습니다. 작업자는 라이브러리를 볼 수 없으므로, 작업자에게 필요한 내용은 핵심 사실과 읽을 파일 경로를 task에 직접 담아 넘깁니다. 라이브러리에서 가져온 내용은 출처(파일)를 밝힙니다.
- 한국어로 말합니다. 짧게, 명확하게.
{agents}
작업 디렉터리는 {cwd} 입니다. {folders}
"#,
            name = track.name,
            agents = if agents.is_empty() {
                String::new()
            } else {
                format!("- **이 트랙에서는 작업자의 에이전트를 사람이 정합니다.** `spawn_worker`를 부를 때마다 앱이 결정 카드를 올리고, 사람이 고르기 전에는 작업자가 시작하지 않습니다. 당신이 고를지 말지를 판단하지 않습니다 — 카드는 항상 올라갑니다. `kind`(일의 성질)는 사람이 판단할 재료이므로 늘 적습니다. 특정 에이전트가 맞다고 보면 agent(필요하면 model)와 why를 함께 적어 제안으로 표시되게 하되, 그것도 제안일 뿐입니다. 카드를 올린 뒤에는 무엇을 물었는지 한 문장만 말하고 턴을 끝냅니다. 같은 작업자로 `spawn_worker`를 다시 부르지 않습니다.\n- 사람이 버튼 대신 직접 글로 답하면 그 말이 `{DECISION_PREFIX}` 메시지로 당신에게 옵니다. 그 말이 어느 에이전트와 어느 모델을 뜻하는지 해석해서 `spawn_worker(name=…, task=…, agent=…, model=…, decided=<카드 번호>)`로 엽니다. `decided`는 그 한 번만 카드 없이 통과시킵니다. 해석할 수 없거나, 그 에이전트가 실제로 제공하지 않는 모델을 가리키거나, 추측이 필요하면 고르지 말고 `request_decision`으로 되물으세요. **앱은 모델의 가격을 알지 못합니다** — \"가장 싼 모델\" 같은 말은 사람이 어느 모델을 뜻하는지 확인한 뒤에만 고릅니다.\n{agents}")
            },
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
        r#"You are DIVIXI's conductor. You are the only one who talks to the human; real work is delegated to workers, which are separate agent sessions.

Track: {name}{about}
Rules:
- If the human's message is a question or small talk, answer it yourself. Do not open a worker.
- If real work is needed (reading, changing or investigating code), open a worker with `spawn_worker`. Name it short and lowercase (e.g. fix-parser) and make the task specific enough for the worker to finish alone.
- `spawn_worker` and `ask_worker` return as soon as the worker has the task. Do not wait for the result: tell the human in one sentence what you delegated and end your turn. When the worker finishes, a message starting with `{REPORT_PREFIX}` reaches you: the structured report the app took from the worker — status (done, partial, blocked, failed), summary, changes, checks (what was verified and how it came out), risks, questions, next, and any edits the app saw that the report does not list. Explain to the human in a paragraph or two what happened. Do not paste the report; give the gist. If there are no checks or a check failed, say so. Answer questions yourself with `ask_worker` when you can; put those only the human can answer to them with `request_decision`. The worker's whole reply is in `read_report`.
- Follow-ups for the same worker go through `ask_worker`; the worker remembers its earlier turns.
- `worker_status` lists every worker, open and closed. Never hunt for workers through the terminal or files.
- A closed worker reopens with its earlier conversation when you call `spawn_worker` or `ask_worker` with its name. To drop that memory and start over, pass fresh=true to `spawn_worker`. `close_worker` only closes the session; the record stays.
- When finished worker sessions pile up (`worker_status` lists them under idle_sessions), offer the human to tidy them away and call `close_worker` only for the ones they agree to. Deleting a record is never yours to do: the human does that from the worker list. No deletion ever touches a worker's folder or the files in it.
- If a tool returns an error, retry once as the message suggests; if that fails, tell the human what is blocked. Never call the same tool repeatedly.
- Choices that belong to the human (a choice between directions, hard-to-undo changes, taste or priorities) are not yours to make. Do not list options as A/B/C in prose: call `request_decision`, and the app shows them as a decision card with buttons. After calling it, say in one sentence what you asked and end your turn. The human's answer arrives as a message starting with `{DECISION_PREFIX}`. If a worker report raises a question only the human can answer, put that to them with `request_decision` too. Decisions the human makes in conversation are recorded with `record_decision`.
- The human does not talk to workers. When a worker asks whether it may do something, the question reaches you as a message starting with `{PERMISSION_PREFIX}`. If it is within the human's instructions and the task you gave, choose yourself and answer with `answer_worker` (usually allow once). If it is hard to undo, outside the task, or the human's call, ask them with `request_decision` and pass their answer on with `answer_worker`. The worker waits until answered, so do not leave it.
- When the human says a job is one they will want again ("make this a routine", "we will do this every release", "save this for next time"), save it with `save_routine`. Your part is the WORDS. A routine is not a worker: it runs **on its own agent, in its own folder**, and neither a worker session nor this conversation goes with it. So do not copy the task you just gave a worker — **rewrite it to stand alone, read weeks from now with none of this around it**: drop "the file we just looked at" and "as before", name the paths, the commands and the standards outright, and say what a good answer looks like. Leaving the folder and the agent out takes this track's. Afterwards say in one sentence what you saved, where and on what it will run, and that all three are theirs to change under Routines in the sidebar. Read `list_routines` first so the same job is not written down twice. To run a saved one now, use `run_routine`.
- There is a knowledge library of documents the human chose. When the human asks what we know about something, refers to their docs or notes or to a document by name, or when work you are about to delegate touches a topic the library covers, search it with `knowledge_search` (`knowledge_list_sources` shows what is there). Do not call it for general coding questions or what the working folder answers. Workers cannot see the library: put what they need from it (the key facts and the file paths to read) into their task. Name the file when you use something from the library.
- Speak English. Short and clear.
{agents}
The working directory is {cwd}. {folders}
"#,
        name = track.name,
        agents = if agents.is_empty() {
            String::new()
        } else {
            format!("- **On this track the human picks every worker's agent.** Each `spawn_worker` raises a decision card and the worker does not start until they choose. You do not decide whether to ask — the card always goes up. Always pass `kind` (what kind of work it is): that is what they judge it on. When you do think a particular agent fits, pass `agent` (and `model`) with `why` and it is marked as your suggestion — but it is only a suggestion. After the card goes up, say in one sentence what you asked and end your turn. Do not call `spawn_worker` for that worker again.\n- When the human writes their own answer instead of taking a button, their words reach you as a `{DECISION_PREFIX}` message. Work out which agent and which model they mean and open the worker with `spawn_worker(name=…, task=…, agent=…, model=…, decided=<card number>)`. `decided` lets that one call through without another card, once. If you cannot tell what they mean, or they name a model no agent offers, or you would have to guess, do not pick for them: ask with `request_decision`. **The app does not know what any model costs** — for words like \"the cheapest model\", find out which model they mean before choosing one.\n{agents}")
        },
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
    let library = app.clone();
    let save_routine = (app.clone(), track.clone());
    let list_routines = app.clone();
    let run_routine = app.clone();
    let decide = (app, track);

    let mut tools = vec![
        Tool::new(
            "spawn_worker",
            &format!("Open a worker (a separate agent session) and give it a task. Returns at once with the run id; the worker works in the background and its report reaches you later as a message starting with {REPORT_PREFIX}. A closed worker with this name is reopened with its earlier conversation (the result says resumed=true); pass fresh=true to start it over without that memory. Use for real work; answer questions yourself instead."),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short worker name, lowercase, e.g. fix-parser. New, or the name of a closed worker to reopen." },
                    "task": { "type": "string", "description": "What the worker should do, specific enough to finish alone." },
                    "agent": { "type": "string", "description": "Agent id to run the worker on: claude_code, codex, copilot, antigravity. On a track where the human picks, this is a suggestion on their card, not the answer. Otherwise: the worker's earlier agent if it has one, else the track's worker agent (which is the conductor's unless the human set one apart)." },
                    "model": { "type": "string", "description": "Model value id, exactly as listed for that agent in the agents section of your preamble. Leave it out to take the agent's own." },
                    "kind": { "type": "string", "description": "What kind of work this is, in the human's language: research, implementation, review, docs, … Always give it on a track where the human picks the agent: it is what they judge the choice on." },
                    "why": { "type": "string", "description": "Why that agent or model suits this kind of work, one or two sentences in the human's language. Give it whenever you name an agent. Say what about the task calls for it, not that one agent is better than another." },
                    "decided": { "type": "integer", "description": "The number of the card the human answered in their own words, when you are opening the worker with what those words meant. Only use an id the app gave you for this worker; it works once." },
                    "fresh": { "type": "boolean", "description": "Start over without the closed worker's earlier conversation. Default false." }
                },
                "required": ["name", "task"]
            }),
            move |args| {
                let (app, track) = spawn.clone();
                async move {
                    let name = str_arg(&args, "name")?;
                    let task = str_arg(&args, "task")?;
                    let ask = Ask {
                        agent: args.get("agent").and_then(Value::as_str).map(str::to_owned),
                        model: args.get("model").and_then(Value::as_str).map(str::to_owned),
                        kind: args.get("kind").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
                        why: args.get("why").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
                        decided: args.get("decided").and_then(Value::as_i64),
                    };
                    let fresh = args.get("fresh").and_then(Value::as_bool).unwrap_or(false);
                    start_worker_turn(app, track, name, task, ask, true, fresh).await
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
                    start_worker_turn(app, track, name, message, Ask::default(), false, false).await
                }
            },
        ),
        Tool::new(
            "worker_status",
            "Every worker of this track, open or closed, under `workers`: agent, whether its session is open, whether a turn is in flight (with its run id), how many runs it has, its last run with status, and how long its open session has been idle. A closed worker marked resumable reopens with its memory when you call spawn_worker or ask_worker with its name. `working` names the ones in a turn, `idle_sessions` the open sessions with nothing to do, and `note` says what to do about a long list.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let (app, track) = status.clone();
                async move {
                    let state = app.state::<AppState>();
                    let workers = worker_list(&state, &track).await;
                    let named = |open: bool, running: bool| -> Vec<String> {
                        workers
                            .iter()
                            .filter(|w| w["open"].as_bool().unwrap_or(false) == open && w["running"].as_bool().unwrap_or(false) == running)
                            .filter_map(|w| w["name"].as_str().map(str::to_string))
                            .collect()
                    };
                    let working = named(true, true);
                    let idle = named(true, false);
                    let note = if idle.is_empty() {
                        "Nothing to tidy up.".to_string()
                    } else {
                        format!(
                            "{} worker session(s) are open with nothing to do: {}. An open session is an agent process holding memory. When the list has grown long, say so to the human and offer to close the ones they are done with, then call close_worker for those; the record and the memory stay, so the worker reopens by name. You cannot delete a worker's record — only the human can, by right-clicking it in the worker list — and no deletion ever touches a worker's folder or the files in it.",
                            idle.len(),
                            idle.join(", ")
                        )
                    };
                    Ok(json!({ "workers": workers, "working": working, "idle_sessions": idle, "note": note }))
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
            "save_routine",
            "Write work down as a routine: a named instruction the human runs again whenever they want it, without dictating it to you a second time. Use when they say a job is one they will want repeatedly (\"make this a routine\", \"we will do this every release\", \"save this for next time\").

Your part is the WORDS. A routine runs on its own agent in its own folder — not as a worker, not in a worker's session, and nothing of the conversation you had goes with it. So the instruction is read again weeks later with NONE of this around it: write it to stand alone. No \"the file we just discussed\", no \"as before\" — name the files, the commands and the standards outright, and say what a good answer looks like.

The folder defaults to this track's and the agent to the one workers here use; say so in your reply, because the human can change both on the Routines page.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "What the human calls it, e.g. 'Dependency check'." },
                    "instruction": { "type": "string", "description": "What the routine's agent is told, written to stand alone with no reference to this conversation." },
                    "folder": { "type": "string", "description": "Absolute path the routine works in. Omit for this track's folder." },
                    "agent": { "type": "string", "description": "Agent id to run it on: claude_code, codex, copilot, antigravity. Omit for the agent this track's workers use." }
                },
                "required": ["name", "instruction"]
            }),
            move |args| {
                let (app, track) = save_routine.clone();
                async move {
                    let state = app.state::<AppState>();
                    let info = track_info(&state, &track)?;
                    let agent = match args.get("agent").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()) {
                        Some(a) => {
                            state.spec_for(a)?;
                            a.to_string()
                        }
                        // A routine has no track to fall back to at run time,
                        // so what the track would have lent it is resolved
                        // now and written down.
                        None => info.effective_worker_agent().to_string(),
                    };
                    let folder = args
                        .get("folder")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|f| !f.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| info.cwd.clone());
                    let saved = state
                        .store
                        .create_routine(&orchestra_store::RoutinePatch {
                            name: Some(str_arg(&args, "name")?),
                            instruction: Some(str_arg(&args, "instruction")?),
                            cwd: Some(folder),
                            agent: Some(agent),
                            config: None,
                        })
                        .map_err(|e| e.to_string())?;
                    let _ = crate::routine::notify(&app);
                    Ok(json!({
                        "routine": saved.id,
                        "name": saved.name,
                        "folder": saved.cwd,
                        "agent": saved.agent,
                        "note": format!(
                            "Saved. Tell the human what it is called, that it will run on {} in {}, and that the instruction, the folder and the agent are all theirs to change under Routines in the sidebar. When it runs, they are told on the bell — not here.",
                            saved.agent, saved.cwd
                        ),
                    }))
                }
            },
        ),
        Tool::new(
            "list_routines",
            "Every routine saved on this machine: its id, name, the agent and folder it runs on, how many times it has run and when it last did. Read it before saving one, so the same job is not written down twice under two names.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let app = list_routines.clone();
                async move {
                    let state = app.state::<AppState>();
                    let saved = state.store.routines().map_err(|e| e.to_string())?;
                    let rows: Vec<Value> = saved
                        .iter()
                        .map(|r| {
                            json!({
                                "routine": r.id,
                                "name": r.name,
                                "agent": r.agent,
                                "folder": r.cwd,
                                "runs": r.runs,
                                "last_run": r.last_run,
                            })
                        })
                        .collect();
                    Ok(json!({ "routines": rows }))
                }
            },
        ),
        Tool::new(
            "run_routine",
            "Run a saved routine now. Returns at once with the run id. It runs on its own agent in its own folder, on a session of its own that ends with the run — it is not a worker, and there is nothing of it to follow up with. What it found is NOT sent to you: the human is told on the bell and reads it on the Routines page. Say you started it and end your turn.",
            json!({
                "type": "object",
                "properties": { "routine": { "type": "string", "description": "Routine id from list_routines." } },
                "required": ["routine"]
            }),
            move |args| {
                let app = run_routine.clone();
                async move {
                    let id = str_arg(&args, "routine")?;
                    crate::routine::run(app, &id).await
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
    ];
    // The library: the same two tools the knowledge graph's conversation has.
    tools.extend(crate::knowledge::library_tools(library));
    tools
}

pub(crate) fn str_arg(args: &Value, key: &str) -> Result<String, String> {
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
    // A routine has no conductor to decide for the human, so its agent asks
    // them directly — the same way an artifact's does.
    let routine = track.starts_with("routine:");
    if artifact || routine || session == CONDUCTOR_SESSION {
        let ko = state.store.get_meta("setting:language").ok().flatten().as_deref() != Some("en");
        let who = match (artifact, routine, ko) {
            (true, _, true) => "디자인 에이전트",
            (true, _, false) => "The design agent",
            (_, true, true) => "되풀이",
            (_, true, false) => "The routine",
            (_, _, true) => "지휘자",
            (_, _, false) => "The conductor",
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
                session: if artifact {
                    "artifact".to_string()
                } else if routine {
                    crate::routine::ASK_SESSION.to_string()
                } else {
                    CONDUCTOR_SESSION.to_string()
                },
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
        let hand = Hand {
            track: track.clone(),
            text,
            open: true,
            what: format!("permission {request} of {worker}"),
            // Refused right below when it cannot go through: keeping it
            // would answer a worker that has long stopped waiting.
            keep: false,
            worker: Some(worker.clone()),
            run: None,
        };
        if deliver(app.clone(), hand).await {
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
    // Setting a proposal aside is an answer too: the worker starts on the
    // track's own choice rather than waiting out the deadline.
    st.sessions.resolve_proposal(id, None);
    let _ = app.emit("decision", &decision);
    Ok(decision)
}

/// Set aside proposal cards left open by a restart.
///
/// The spawn that raised one lives in a task, and a task does not survive
/// the process. Its card would sit there looking answerable, and answering
/// it would do nothing at all. Called once at start, when by definition no
/// spawn is waiting on anything yet.
pub async fn dismiss_stale_proposals(app: AppHandle) {
    let st = app.state::<AppState>();
    let Ok(open) = st.store.decisions(None) else { return };
    for d in open {
        let stale = d.status == DecisionStatus::Open && d.permission.is_none() && !st.sessions.awaits_proposal(d.id) && is_proposal(&d);
        if stale {
            if let Ok(gone) = st.store.dismiss_decision(d.id) {
                tracing::info!(decision = gone.id, "a worker proposal did not survive the restart");
                let _ = app.emit("decision", &gone);
            }
        }
    }
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
    if ask.session == crate::routine::ASK_SESSION && decision.track.starts_with("routine:") {
        let run = decision.track.strip_prefix("routine:").unwrap_or(&decision.track);
        return crate::routine::answer_permission(app, run, &ask.request, option).await;
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
    // A proposal card answers the spawn holding a worker back, not the
    // conductor: the conductor ended its turn when the card went up, and
    // handing it this would only invite it to open the worker a second time.
    //
    // Words instead of a button go on to the conductor all the same, but
    // through that spawn, which knows which worker they are about — see
    // [`propose_worker`]. An answer with no button and no words is a button
    // that does not exist, and falls back like an unanswered card.
    let outcome = match (choice, own.as_deref().map(str::trim).filter(|w| !w.is_empty())) {
        (Some(i), _) => Some(i),
        (None, Some(_)) => Some(OWN_ANSWER),
        (None, None) => None,
    };
    if st.sessions.resolve_proposal(id, outcome) {
        return Ok(decision);
    }
    let text = decision_text(&decision);
    let track = decision.track.clone();
    let hand = Hand { track, text, open: true, what: format!("decision #{id}"), keep: true, worker: None, run: None };
    tauri::async_runtime::spawn(deliver(app.clone(), hand));
    Ok(decision)
}

/// One thing to hand a track's conductor as a turn.
struct Hand {
    track: String,
    text: String,
    /// May start a closed conductor. A worker report does not.
    open: bool,
    /// What it is, for the log and for the human.
    what: String,
    /// Kept and handed on later when it cannot go through now, instead of
    /// being dropped. False only for a question whose asker is told
    /// straight after that nobody answered it.
    keep: bool,
    /// The worker and run behind it, when it is a report.
    worker: Option<String>,
    run: Option<String>,
}

/// Run a hand's text as a conductor turn once the conductor is free. The
/// turn itself claims the busy mark, so a refusal is retried rather than
/// trusting an earlier free check that another turn may have overtaken.
/// Returns whether the turn started; a `keep` hand that did not start is
/// parked (see [`park`]) and never lost.
async fn deliver(app: AppHandle, hand: Hand) -> bool {
    let Hand { track, text, open, what, keep, worker, run } = hand;
    let deadline = std::time::Instant::now() + REPORT_WAIT;
    let lang = {
        let state = app.state::<AppState>();
        state.store.get_meta("setting:language").ok().flatten().unwrap_or_default()
    };
    let why = loop {
        if !open && !app.state::<AppState>().sessions.conductors.lock().await.contains_key(&track) {
            break "the conductor session is closed".to_string();
        }
        match conductor_turn(app.clone(), track.clone(), text.clone(), None, lang.clone(), Vec::new()).await {
            Ok(_) => return true,
            Err(err) if err == BUSY => {
                if std::time::Instant::now() > deadline {
                    break "the conductor stayed busy".to_string();
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(err) => break err,
        }
    };
    if keep {
        let at = now_ms();
        park(&app, &track, Parked { id: park_id(at), text, open, what, worker, run, at, why, tries: 1 }).await;
    } else {
        tracing::warn!(%track, %what, %why, "could not hand this to the conductor");
    }
    false
}

/// A turn the conductor could not take, kept in the record until it can.
///
/// A worker can run for an hour and its report is the only thing that
/// crosses the membrane. Losing it to a closed or busy conductor would lose
/// the work, so it waits here instead, and the human is told it is waiting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Parked {
    /// Tells this entry from every other, so a flush removes the one it
    /// handed on rather than whatever is at the front now. Entries written
    /// before this existed have none; [`Parked::is`] falls back for those.
    #[serde(default)]
    id: String,
    text: String,
    #[serde(default)]
    open: bool,
    what: String,
    #[serde(default)]
    worker: Option<String>,
    #[serde(default)]
    run: Option<String>,
    /// Unix milliseconds it was first tried.
    at: i64,
    /// Why it could not go through last time.
    #[serde(default)]
    why: String,
    #[serde(default)]
    tries: u32,
}

impl Parked {
    /// Whether this is the same waiting turn as `other`.
    fn is(&self, other: &Parked) -> bool {
        if !self.id.is_empty() && !other.id.is_empty() {
            return self.id == other.id;
        }
        // Written before ids: what identified an entry then. One report per
        // run, so a report is unique by its run; a decision by its text and
        // the millisecond it was put aside.
        self.at == other.at && self.what == other.what && self.run == other.run
    }
}

/// Ids handed out within this run of the app; `at` separates the rest.
static PARK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn park_id(at: i64) -> String {
    format!("{at}-{}", PARK_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// How many full report texts one track's waiting list keeps.
///
/// The list is one JSON row rewritten whole on every change, and one
/// report's text runs to 12k characters. Left alone it grows with every
/// worker that finishes while the conductor is shut, and never shrinks by
/// itself. Past this many the older ones are folded to a stub — nothing is
/// dropped, because the report itself lives in `meta report:<run>` and both
/// the conversation and `read_report` take it from there; only the verbatim
/// copy the conductor would have been handed goes.
const PARKED_FULL_TEXT: usize = 10;

/// The stub an old waiting report is folded to. Keeps the header shape, so
/// the timeline still finds its worker and run and draws its card.
fn folded_text(item: &Parked) -> Option<String> {
    let (worker, run) = (item.worker.as_deref()?, item.run.as_deref()?);
    Some(format!(
        "{REPORT_PREFIX} worker={worker} run={run} status=kept\n\nThis report waited behind several others, so only its place was kept here. The report itself is in the record: call read_report(\"{run}\") to read it, then tell the human what happened."
    ))
}

/// Fold every waiting report but the newest few (see [`PARKED_FULL_TEXT`]).
/// Idempotent: folding an already folded entry writes the same stub.
fn fold_old(list: &mut [Parked]) {
    let fold_before = list.len().saturating_sub(PARKED_FULL_TEXT);
    for item in list.iter_mut().take(fold_before) {
        if let Some(stub) = folded_text(item) {
            item.text = stub;
        }
    }
}

/// What the UI hears when a track's waiting list changes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Waiting {
    pub track: String,
    /// Turns waiting for this conductor now.
    pub pending: usize,
    /// Set when one was just put aside: what it is.
    pub added: Option<String>,
    pub worker: Option<String>,
    pub run: Option<String>,
    pub why: String,
}

fn parked_key(track: &str) -> String {
    format!("parked:{track}")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// What is waiting for a track's conductor, oldest first.
fn parked_list(store: &orchestra_store::Store, track: &str) -> Vec<Parked> {
    store
        .get_meta(&parked_key(track))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn keep_parked(store: &orchestra_store::Store, track: &str, list: &[Parked]) {
    if list.is_empty() {
        if let Err(err) = store.delete_meta(&parked_key(track)) {
            tracing::error!(%track, %err, "could not clear the conductor's waiting list");
        }
        return;
    }
    match serde_json::to_string(list) {
        Ok(json) => {
            if let Err(err) = store.set_meta(&parked_key(track), &json) {
                tracing::error!(%track, %err, "could not keep what the conductor has not taken");
            }
        }
        Err(err) => tracing::error!(%track, %err, "could not encode the conductor's waiting list"),
    }
}

/// One waiting list is read and written at a time, so two reports parked
/// at once cannot overwrite each other.
static PARKED: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Put a turn aside for later and tell the human it is waiting.
async fn park(app: &AppHandle, track: &str, item: Parked) {
    let _one = PARKED.lock().await;
    let state = app.state::<AppState>();
    let mut list = parked_list(&state.store, track);
    list.push(item.clone());
    fold_old(&mut list);
    keep_parked(&state.store, track, &list);
    tracing::warn!(%track, what = %item.what, why = %item.why, pending = list.len(), "the conductor could not take this; it waits");
    let _ = app.emit(
        "parked",
        Waiting {
            track: track.to_string(),
            pending: list.len(),
            added: Some(item.what),
            worker: item.worker,
            run: item.run,
            why: item.why,
        },
    );
}

/// One turn waiting for a conductor, as the UI shows it.
///
/// `run` is what makes the wait bearable: the report is already kept
/// (`meta report:<run>`), so the conversation can show the card itself
/// while the conductor is still out of reach.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WaitingItem {
    /// Unique among what is waiting; the UI keys its list on this.
    pub id: String,
    pub track: String,
    /// What it is, in one phrase.
    pub what: String,
    /// The worker it came from, when it is a report.
    pub worker: Option<String>,
    /// The worker run it reports on.
    pub run: Option<String>,
    /// Unix milliseconds it was first tried.
    pub at: i64,
    /// Why it has not gone through yet.
    pub why: String,
    /// How many times it has been offered to the conductor.
    pub tries: u32,
}

/// Everything waiting for a conductor, across tracks, oldest first within
/// each. The UI reads this rather than counting the `parked` events: an
/// event can be missed (the app was closed), the record cannot.
pub fn parked_all(state: &AppState) -> Vec<WaitingItem> {
    let mut out = Vec::new();
    for track in state.store.tracks().unwrap_or_default() {
        for item in parked_list(&state.store, &track.id) {
            out.push(WaitingItem {
                id: item.id,
                track: track.id.clone(),
                what: item.what,
                worker: item.worker,
                run: item.run,
                at: item.at,
                why: item.why,
                tries: item.tries,
            });
        }
    }
    out
}

/// Hand the conductor what has been waiting for it, oldest first, until one
/// cannot go through either. Called when a conductor turn ends, when a
/// conductor is opened, and once a minute.
///
/// Boxed rather than a plain `async fn`: a conductor turn ends by flushing,
/// and a flush starts a conductor turn, so the compiler cannot work out on
/// its own whether either future is `Send`. Saying so here cuts the circle.
pub fn flush_parked(app: AppHandle, track: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
    loop {
        let next = {
            let _one = PARKED.lock().await;
            let state = app.state::<AppState>();
            match parked_list(&state.store, &track).first() {
                Some(first) => first.clone(),
                None => return,
            }
        };
        let lang = {
            let st = app.state::<AppState>();
            if st.sessions.is_busy(&track) {
                return;
            }
            // A report still does not open a closed conductor; it waits for
            // the human to talk to it, and the bell already says it is there.
            if !next.open && !st.sessions.conductors.lock().await.contains_key(&track) {
                return;
            }
            st.store.get_meta("setting:language").ok().flatten().unwrap_or_default()
        };
        match conductor_turn(app.clone(), track.clone(), next.text.clone(), None, lang, Vec::new()).await {
            Ok(_) => {
                let _one = PARKED.lock().await;
                let state = app.state::<AppState>();
                let mut list = parked_list(&state.store, &track);
                // By identity, not by position. Three things call this (a
                // conductor opening, a turn ending, the minute sweep), so
                // two flushes on one track are ordinary; `begin_turn` keeps
                // them from both delivering, but the copy taken above can
                // still be stale by the time we get back here. Taking the
                // front on trust would then drop a report nobody delivered.
                if list.iter().any(|item| item.is(&next)) {
                    list.retain(|item| !item.is(&next));
                    keep_parked(&state.store, &track, &list);
                    tracing::info!(%track, what = %next.what, left = list.len(), "handed the conductor what was waiting");
                    let _ = app.emit(
                        "parked",
                        Waiting { track: track.clone(), pending: list.len(), added: None, worker: None, run: None, why: String::new() },
                    );
                } else {
                    tracing::info!(%track, what = %next.what, "what was waiting had already been taken off the list");
                }
                // The turn it just started holds the conductor; the next
                // round sees that and comes back when the turn ends.
            }
            Err(err) => {
                if err != BUSY {
                    let _one = PARKED.lock().await;
                    let state = app.state::<AppState>();
                    let mut list = parked_list(&state.store, &track);
                    // The one that failed, not the one at the front now.
                    if let Some(item) = list.iter_mut().find(|item| item.is(&next)) {
                        item.tries += 1;
                        item.why = err.clone();
                        keep_parked(&state.store, &track, &list);
                    }
                    tracing::warn!(%track, what = %next.what, %err, "what was waiting still cannot go through");
                }
                return;
            }
        }
    }
    })
}

/// Try every track's waiting list once a minute, so nothing sits there
/// because no other trigger happened to fire.
pub fn sweep_parked(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let waiting: Vec<String> = {
                let state = app.state::<AppState>();
                let mut tracks: Vec<String> = Vec::new();
                for item in parked_all(&state) {
                    if !tracks.contains(&item.track) {
                        tracks.push(item.track);
                    }
                }
                tracks
            };
            for track in waiting {
                flush_parked(app.clone(), track).await;
            }
        }
    });
}

/// What the store remembers about a worker's last agent session, so the worker
/// can be reopened with its conversation.
#[derive(serde::Serialize, serde::Deserialize)]
struct WorkerRecord {
    session_id: String,
    agent: String,
    /// The agent's session options the worker last opened with (`option id
    /// → value id`), so reopening it keeps the model it was given rather
    /// than falling back to the agent's own.
    ///
    /// Absent in records written before a worker could differ from its
    /// track, which `default` reads as "whatever the track says".
    #[serde(default)]
    config: BTreeMap<String, String>,
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

/// What the conductor asked for when opening a worker: an agent and model
/// other than the track's, and the case it makes for them.
///
/// Empty is the ordinary call, and the ordinary call never raises a card.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ask {
    agent: Option<String>,
    model: Option<String>,
    /// What kind of work this is, in the human's words: research,
    /// implementation, review, docs… The human judges the choice on this.
    kind: String,
    /// Why this agent or model suits that kind of work.
    why: String,
    /// The card the human already answered in their own words, whose answer
    /// the conductor has read and turned into an agent and model.
    ///
    /// This is the one way past the card on a `propose` track, and it is
    /// spent when used: without it the conductor could answer its own card
    /// by spawning again, and with it re-usable it could loop.
    decided: Option<i64>,
}

impl Ask {
    fn names_something(&self) -> bool {
        self.agent.is_some() || self.model.is_some()
    }
}

/// The id of an agent's model option (`model` for most), when it has one.
fn model_option_id(state: &AppState, agent: &str) -> Option<String> {
    state.config_options_for(agent).into_iter().find(|o| o.category == "model").map(|o| o.id)
}

/// A model's display name as the agent advertised it, else the raw id.
fn model_name(state: &AppState, agent: &str, value: &str) -> String {
    state
        .config_options_for(agent)
        .into_iter()
        .find(|o| o.category == "model")
        .and_then(|o| o.choices.into_iter().find(|c| c.id == value))
        .map(|c| c.name)
        .unwrap_or_else(|| value.to_string())
}

/// The model in a config map, for an agent, when one is set.
fn model_of(state: &AppState, agent: &str, config: &BTreeMap<String, String>) -> String {
    model_option_id(state, agent)
        .and_then(|id| config.get(&id).cloned())
        .unwrap_or_default()
}

/// `agent · model` for a card and for the log.
fn describe(state: &AppState, agent: &str, config: &BTreeMap<String, String>) -> String {
    let label = AgentKind::parse(agent).map(|k| k.name().to_string()).unwrap_or_else(|| agent.to_string());
    match model_of(state, agent, config) {
        m if m.is_empty() => label,
        m => format!("{label} · {}", model_name(state, agent, &m)),
    }
}

/// Turn an ask into the agent and options to open with, refusing anything
/// the agent does not actually offer.
///
/// A bad id is the conductor's mistake, and it is answered to the conductor
/// straight away: putting a card to the human naming a model that does not
/// exist would waste the one thing a card is for.
fn resolve_ask(state: &AppState, info: &TrackInfo, ask: &Ask) -> Result<(String, BTreeMap<String, String>), String> {
    let agent_id = ask.agent.clone().unwrap_or_else(|| info.effective_worker_agent().to_string());
    if AgentKind::parse(&agent_id).is_none() {
        return Err(format!("unknown agent {agent_id}. Use one of: {:?}", AgentKind::ALL.map(|k| k.id())));
    }
    // Options are in one agent's terms, so the track's only carry over to a
    // worker on the track's own agent.
    let mut config = if agent_id == info.effective_worker_agent() {
        info.effective_worker_config().clone()
    } else {
        BTreeMap::new()
    };
    if let Some(model) = ask.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        let options = state.config_options_for(&agent_id);
        let Some(option) = options.iter().find(|o| o.category == "model") else {
            return Err(format!(
                "{agent_id} offers no model choice over ACP; its model comes from its own CLI settings. Open the worker without a model."
            ));
        };
        if !option.choices.iter().any(|c| c.id == model) {
            let known: Vec<&str> = option.choices.iter().map(|c| c.id.as_str()).collect();
            return Err(format!("{agent_id} does not offer the model {model:?}. It offers: {known:?}"));
        }
        config.insert(option.id.clone(), model.to_string());
    }
    Ok((agent_id, config))
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
            let kept = worker_record(state, track, &info.name);
            // What it is running on now, else what it last ran on. The
            // record is written when the session opens, so both come from
            // the same place for an open worker.
            let agent = live.map(|l| l.agent.clone()).unwrap_or_else(|| info.agent.clone());
            let config = live.map(|l| l.config.clone()).or_else(|| kept.as_ref().map(|r| r.config.clone())).unwrap_or_default();
            json!({
                "name": info.name,
                "agent": agent,
                "model": model_of(state, &agent, &config),
                "open": live.is_some(),
                "running": live.map(|l| l.running.is_some()).unwrap_or(false),
                "run": live.and_then(|l| l.running.clone()),
                "runs": info.runs,
                "last_run": info.last_run,
                "last_status": info.last_status.as_str(),
                "last_at": info.last_at,
                "resumable": live.is_none() && kept.is_some(),
                "idle_minutes": live.filter(|l| l.running.is_none()).map(|l| l.used.elapsed().as_secs() / 60),
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
                "model": model_of(state, &live.agent, &live.config),
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

/// Give a worker a turn and return at once.
///
/// On a track whose worker choice is `propose`, **every** worker opened here
/// goes to the human on a card first. That is what turning it on means: not
/// "ask when the conductor thinks it is worth asking", but "I pick the agent
/// for each worker myself". A conductor deciding when to raise the card
/// would be deciding the thing the human took for themselves — and in
/// practice it simply never raised one.
///
/// The conductor's `agent`/`model`/`why` are a suggestion on that card, not
/// a condition for it. The call still returns at once either way.
async fn start_worker_turn(
    app: AppHandle,
    track: String,
    name: String,
    text: String,
    ask: Ask,
    open: bool,
    fresh: bool,
) -> Result<Value, String> {
    if name == CONDUCTOR_SESSION {
        return Err("that name is reserved".to_string());
    }
    let (chosen, ignored) = {
        let state = app.state::<AppState>();
        let info = track_info(&state, &track)?;
        // The human already answered a card for this worker and the
        // conductor is bringing their answer back. Checked against the
        // record, and it is spent by the check, so it cannot re-card and
        // cannot be replayed.
        if let Some(decision) = ask.decided {
            if state.sessions.settled_worker(decision).as_deref() != Some(name.as_str()) {
                return Err(format!(
                    "decision {decision} is not an answered card waiting for worker {name}. Do not pass `decided` unless the app gave you that id for this worker."
                ));
            }
            // Spent only once the agent and model check out. A refused model
            // here is the conductor misreading the human's words, and it must
            // be able to try again — the alternative is their answer going
            // nowhere and the worker never starting.
            let (agent_id, config) = resolve_ask(&state, &info, &ask)?;
            state.sessions.take_settled(decision);
            (Some((agent_id, config)), None)
        } else if !open {
            // `ask_worker` continues a conversation; its session exists
            // already and its agent came with it.
            let ignored = ask
                .names_something()
                .then(|| "a follow-up stays on the agent the worker is already open with; the agent and model you named were not used.".to_string());
            (None, ignored)
        } else if !info.proposes_worker_choice() {
            let ignored = ask.names_something().then(|| {
                format!(
                    "this track keeps every worker on {}, so the agent and model you named were not used. The human can let you propose others in the track's worker settings.",
                    describe(&state, info.effective_worker_agent(), info.effective_worker_config())
                )
            });
            (None, ignored)
        } else {
            // A bad id is answered now, to the conductor, rather than put to
            // the human as a card naming something that does not exist.
            let suggestion = ask.names_something().then(|| resolve_ask(&state, &info, &ask)).transpose()?;
            // A worker that is already open keeps its session whatever is
            // asked for; the body below says so rather than a card.
            let live = state.sessions.workers.lock().await.contains_key(&worker_key(&track, &name));
            if !live {
                return propose_worker(&app, &track, &name, &text, &ask, suggestion, fresh).await;
            }
            (None, None)
        }
    };
    let mut result = run_worker_turn(app, track, name, text, chosen, open, fresh).await?;
    if let Some(note) = ignored {
        result["ignored"] = Value::String(note);
    }
    Ok(result)
}

/// Heads every option id of a proposal card, followed by the agent that
/// option opens the worker on.
const PROPOSAL_OPTION: &str = "agent:";

/// Whether a decision is a worker proposal rather than something the
/// conductor asked with `request_decision`.
///
/// Told apart by its option ids: only a proposal card sets them, and only
/// to an agent. The two kinds are answered in completely different places,
/// so the app must never guess wrong about which it has.
fn is_proposal(d: &Decision) -> bool {
    !d.options.is_empty() && d.options.iter().all(|o| o.id.starts_with(PROPOSAL_OPTION))
}

/// Put the conductor's proposal to the human and hold the worker back until
/// they answer, or until [`PROPOSAL_WAIT`] runs out and the track's own
/// choice is used instead.
async fn propose_worker(
    app: &AppHandle,
    track: &str,
    name: &str,
    text: &str,
    ask: &Ask,
    suggestion: Option<(String, BTreeMap<String, String>)>,
    fresh: bool,
) -> Result<Value, String> {
    let (decision, ours, minutes, choices) = {
        let state = app.state::<AppState>();
        let info = track_info(&state, track)?;
        let fallback = (info.effective_worker_agent().to_string(), info.effective_worker_config().clone());
        let ko = state.store.get_meta("setting:language").ok().flatten().as_deref() != Some("en");
        let ours = describe(&state, &fallback.0, &fallback.1);
        let minutes = PROPOSAL_WAIT.as_secs() / 60;
        let suggested = suggestion.as_ref().map(|s| s.0.clone());

        // One button per agent that could actually take the work, each
        // saying which models it offers. Agent times model as separate
        // buttons would be dozens of them and unreadable; the models go in
        // the line under the agent, and naming one is what the free answer
        // at the bottom is for.
        let mut options: Vec<DecisionOption> = Vec::new();
        let mut settings: Vec<(String, BTreeMap<String, String>)> = Vec::new();
        for status in agent_choices(&state) {
            let id = status.kind.id().to_string();
            let mut marks: Vec<String> = Vec::new();
            if id == fallback.0 {
                marks.push(if ko { "트랙 기본".to_string() } else { "track default".to_string() });
            }
            if suggested.as_deref() == Some(id.as_str()) {
                marks.push(if ko { "지휘자 제안".to_string() } else { "the conductor suggests this".to_string() });
            }
            let label = match marks.is_empty() {
                true => status.kind.name().to_string(),
                false => format!("{} · {}", status.kind.name(), marks.join(" · ")),
            };
            let mut detail = model_line(&state, &id, ko);
            // Why, when it is about this agent: the conductor's case belongs
            // beside the button it argues for, not at the top of the card.
            if suggested.as_deref() == Some(id.as_str()) && !ask.why.is_empty() {
                detail = format!("{detail}\n{}", ask.why);
            }
            options.push(DecisionOption { label, detail, id: format!("{PROPOSAL_OPTION}{id}") });
            // The track's options only mean anything on the track's agent;
            // any other starts from that agent's own, or the suggestion when
            // the conductor named a model on it.
            let config = match (&suggestion, id == fallback.0) {
                (Some((sug, cfg)), _) if *sug == id => cfg.clone(),
                (_, true) => fallback.1.clone(),
                _ => BTreeMap::new(),
            };
            settings.push((id, config));
        }
        if options.is_empty() {
            return Err("no agent is ready, so there is nothing to open a worker on. Tell the human to check the agents in settings.".to_string());
        }

        let question = if ko {
            format!("작업자 {name}을(를) 어느 에이전트로 열까요?")
        } else {
            format!("Which agent should worker {name} run on?")
        };
        // What the work is, so the choice can be made on something. The
        // conductor's own view, when it has one, sits beside its button.
        let mut context = String::new();
        if !ask.kind.is_empty() {
            context.push_str(&if ko { format!("작업 성질: {}\n", ask.kind) } else { format!("Kind of work: {}\n", ask.kind) });
        }
        context.push_str(&clip_task(text));
        context.push('\n');
        context.push_str(&if ko {
            format!("\n모델을 직접 고르려면 아래에 적으세요(예: codex 가장 싼 모델). {minutes}분 안에 답이 없으면 트랙 기본({ours})으로 시작합니다.")
        } else {
            format!("\nTo pick a model, write it below (e.g. codex, the cheapest model). With no answer in {minutes} minutes it starts on this track's default ({ours}).")
        });

        let recommended = suggested
            .as_deref()
            .or(Some(fallback.0.as_str()))
            .and_then(|want| settings.iter().position(|(id, _)| id == want));
        let decision = state
            .store
            .open_decision(&NewDecision {
                track: track.to_string(),
                run: conductor_run(&state, track).await,
                question,
                context,
                options,
                recommended,
                // A model is named in words, not in buttons: the answer goes
                // to the conductor, which turns it into an agent and model.
                allow_other: true,
                permission: None,
            })
            .map_err(|e| e.to_string())?;
        let _ = app.emit("decision", &decision);
        let waiter = state.sessions.await_proposal(decision.id);

        let (app_t, track_t, name_t, text_t) = (app.clone(), track.to_string(), name.to_string(), text.to_string());
        let id = decision.id;
        let choices: Vec<String> = settings.iter().map(|(a, _)| a.clone()).collect();
        tauri::async_runtime::spawn(async move {
            let answered = tokio::time::timeout(PROPOSAL_WAIT, waiter).await;
            let st = app_t.state::<AppState>();
            // Forget the waiter however this ended, so an answer arriving
            // late is handled as an ordinary decision instead of being sent
            // to a channel nobody holds.
            st.sessions.resolve_proposal(id, None);
            let picked = match answered {
                Ok(Ok(choice)) => choice,
                _ => None,
            };
            // Answered in the human's own words: the conductor reads it,
            // works out what it means, and opens the worker itself. Nothing
            // starts here — guessing at "the cheapest model" is exactly what
            // this hands off rather than does.
            if picked == Some(OWN_ANSWER) {
                let words = st.store.decision(id).ok().flatten().and_then(|d| d.answer).unwrap_or_default();
                st.sessions.mark_settled(id, &name_t);
                let hand = Hand {
                    track: track_t.clone(),
                    text: format!(
                        "{DECISION_PREFIX} #{id}\nThe human answered the card about worker {name_t} in their own words instead of taking a button:\n\n{words}\n\nWork out which agent and which model they mean and open the worker with spawn_worker(name=\"{name_t}\", task=…, agent=…, model=…, decided={id}). The task is the one you were about to give it. Agents and their models are listed in your preamble; `decided={id}` is what lets this one call through without another card, and it works once.\nIf their words do not name something an agent actually offers, or you would have to guess (the app does not know what any model costs), do not pick for them: ask with request_decision and pass their answer on afterwards."
                    ),
                    open: false,
                    what: format!("own answer for {name_t}"),
                    keep: true,
                    worker: None,
                    run: None,
                };
                deliver(app_t, hand).await;
                return;
            }
            // An unanswered card is closed rather than left open looking live.
            if let Ok(Some(d)) = st.store.decision(id) {
                if d.status == DecisionStatus::Open {
                    if let Ok(gone) = st.store.dismiss_decision(id) {
                        let _ = app_t.emit("decision", &gone);
                    }
                }
            }
            let chosen = picked.and_then(|i| settings.get(i).cloned()).unwrap_or(fallback);
            tracing::info!(track = %track_t, worker = %name_t, agent = %chosen.0, answered = picked.is_some(), "worker proposal settled");
            let outcome = run_worker_turn(app_t.clone(), track_t.clone(), name_t.clone(), text_t, Some(chosen), true, fresh).await;
            if let Err(err) = outcome {
                // The conductor ended its turn when the card went up, so a
                // failure here reaches nobody unless it is handed back.
                let hand = Hand {
                    track: track_t,
                    text: format!(
                        "Worker {name_t} could not be opened after the card about its agent was settled: {err}. Tell the human what is blocked; do not open it again on your own."
                    ),
                    open: false,
                    what: format!("failed proposal for {name_t}"),
                    keep: true,
                    worker: None,
                    run: None,
                };
                deliver(app_t, hand).await;
            }
        });
        (decision.id, ours, minutes, choices)
    };

    Ok(json!({
        "worker": name,
        "status": "proposed",
        "decision": decision,
        "offered": choices,
        "otherwise": ours,
        "note": format!("This track's human picks the agent for every worker, so {name} is waiting on card #{decision}. Say in one sentence what you asked and end your turn. If they take one of the buttons the worker starts by itself and its {REPORT_PREFIX} reaches you as usual; if they answer in their own words you get those words to act on. Either way, do not call spawn_worker for {name} again unless you are told to. With no answer in {minutes} minutes it starts on {ours}."),
    }))
}

/// The index a proposal card's own-words answer is reported under.
///
/// Options are answered by index and an own answer has none, so it needs a
/// value of its own that no option can collide with.
const OWN_ANSWER: usize = usize::MAX;

/// Agents a worker could actually be opened on: detected, and ready.
fn agent_choices(state: &AppState) -> Vec<AgentStatus> {
    state
        .load_agents()
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.readiness == Readiness::Ready)
        .collect()
}

/// What an agent offers as models, for the line under its button.
///
/// An agent that exposes none says so. Whether any given agent does is not
/// something the app can know before it has been probed, so this reads what
/// detection found rather than assuming.
fn model_line(state: &AppState, agent: &str, ko: bool) -> String {
    const SHOWN: usize = 6;
    let options = state.config_options_for(agent);
    let Some(model) = options.iter().find(|o| o.category == "model").filter(|o| !o.choices.is_empty()) else {
        return if ko {
            "모델 선택 불가 — 이 에이전트의 모델은 자체 CLI 설정을 따릅니다".to_string()
        } else {
            "no model choice — this agent takes its model from its own CLI settings".to_string()
        };
    };
    let mut names: Vec<String> = model.choices.iter().take(SHOWN).map(|c| c.name.clone()).collect();
    if model.choices.len() > SHOWN {
        names.push(format!("… (+{})", model.choices.len() - SHOWN));
    }
    let current = model
        .choices
        .iter()
        .find(|c| c.id == model.current)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| model.current.clone());
    if ko {
        format!("모델: {} · 기본 {current}", names.join(", "))
    } else {
        format!("models: {} · default {current}", names.join(", "))
    }
}

/// The task, short enough to read on a card.
fn clip_task(text: &str) -> String {
    const MAX: usize = 400;
    let text = text.trim();
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    format!("{}…", text.chars().take(MAX).collect::<String>())
}

/// Open or continue a worker and give it a turn, returning at once.
///
/// `open` is `spawn_worker` (a worker that is already open is refused);
/// `ask_worker` needs the worker to exist, open or in the record. A worker
/// that is not open but has a remembered session is reopened with it,
/// unless `fresh` says to forget. `chosen` is an agent and its options
/// settled on already — the track's, or what the human approved on a card;
/// `None` leaves it to the worker's own record, then the track.
async fn run_worker_turn(
    app: AppHandle,
    track: String,
    name: String,
    text: String,
    chosen: Option<(String, BTreeMap<String, String>)>,
    open: bool,
    fresh: bool,
) -> Result<Value, String> {
    let (agent, settled) = match chosen {
        Some((a, c)) => (Some(a), Some(c)),
        None => (None, None),
    };
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
        Live { run: String, turns: u32, session: Arc<AgentSession>, agent: String, config: BTreeMap<String, String> },
        Open {
            agent_id: String,
            spec: AgentSpec,
            opts: SessionOptions,
            config: BTreeMap<String, String>,
            wanted: bool,
            past_runs: Option<u32>,
            handoff: Option<String>,
        },
    }
    let found = {
        let mut workers = state.sessions.workers.lock().await;
        if state.sessions.opening.lock().contains(&key) {
            return Err(format!("worker {name} is still starting; wait for it before sending more."));
        }
        if state.sessions.deleting.lock().contains(&key) {
            return Err(format!("worker {name} is being deleted; its record is going. Use another name."));
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
                Found::Live {
                    run,
                    turns: live.turns,
                    session: live.session.clone(),
                    agent: live.agent.clone(),
                    config: live.config.clone(),
                }
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
                // Options, in the same order as the agent: what was settled
                // for this call, else what the worker last ran with, else the
                // track's — and the track's only on the track's own agent,
                // since an option id means nothing to a different one.
                //
                // Without the record step a worker reopened by name would
                // silently drop back to its agent's default model, which is
                // exactly what made a chosen model not stick before.
                let kept = record.as_ref().filter(|r| r.agent == agent_id).map(|r| r.config.clone()).unwrap_or_default();
                let config = match &settled {
                    Some(c) => c.clone(),
                    None if !kept.is_empty() => kept,
                    None if agent_id == info.effective_worker_agent() => info.effective_worker_config().clone(),
                    None => BTreeMap::new(),
                };
                // Memory only carries over on the agent that made it.
                let resume = record.filter(|r| r.agent == agent_id).map(|r| r.session_id);
                let wanted = resume.is_some();
                let spec: AgentSpec = state.spec_for(&agent_id)?;
                let mut opts = session_options(&state, &agent_id, &worker_cwd, &config, None);
                opts.resume = resume;
                state.sessions.opening.lock().insert(key.clone());
                // Reopened on another agent than before (not a fresh start):
                // it reads its earlier turns, as the record has them.
                let before = if fresh { None } else { worker_record(&state, &track, &name).map(|r| r.agent).or_else(|| past.map(|p| p.agent.clone())) };
                let handoff = before.filter(|b| *b != agent_id).map(|b| handoff_text(&state, &track, &name, &b, None));
                Found::Open { agent_id, spec, opts, config, wanted, past_runs: past.map(|p| p.runs), handoff }
            }
        }
    };
    let mut handoff_for_worker: Option<String> = None;
    // Options the agent would not take, when it refused any. The human
    // chose a model; if it did not take, they are the ones who have to know.
    let mut refused: Vec<String> = Vec::new();
    let (agent_id, config, turns, session, resumed, note, run) = match found {
        Found::Live { run, turns, session, agent, config } => (agent, config, turns, session, false, None, run),
        Found::Open { agent_id, spec, opts, config, wanted, past_runs, handoff } => {
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
            refused = session
                .refused_options()
                .iter()
                .map(|r| format!("{}={}: {}", r.option, r.value, r.error))
                .collect();
            if !refused.is_empty() {
                tracing::warn!(%track, worker = %name, agent = %agent_id, ?refused, "worker session did not take the options it was opened with");
            }
            remember_worker(
                &state,
                &track,
                &name,
                &WorkerRecord {
                    session_id: session.session_id().to_string(),
                    agent: agent_id.clone(),
                    config: config.clone(),
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
                    config: config.clone(),
                    cwd: worker_cwd.clone(),
                    session: session.clone(),
                    turns,
                    running: Some(run.clone()),
                    used: std::time::Instant::now(),
                },
            );
            (agent_id, config, turns, session, resumed, note, run)
        }
    };

    // The turn itself, in the background. The task ends with how to report;
    // the stored prompt stays what the conductor wrote.
    let app_for_turn = app.clone();
    let (track_t, name_t, run_t, cwd_t, agent_t) = (track.clone(), name.clone(), run.clone(), worker_cwd.clone(), agent_id.clone());
    let model = model_of(&state, &agent_id, &config);
    // Worth saying in the report when it is not simply the track's default,
    // or when something the session was opened with did not take.
    let plain = agent_id == info.effective_worker_agent() && model == model_of(&state, info.effective_worker_agent(), info.effective_worker_config());
    let ran_on = (!plain || !refused.is_empty()).then(|| RanOn {
        agent: agent_id.clone(),
        model: model.clone(),
        refused: refused.clone(),
    });
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
        let report = finish_report(&app, &run_t, (current != run_t).then_some(current.as_str()), outcome, &cwd_t, ran_on);
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
    if !model.is_empty() {
        result["model"] = Value::String(model);
    }
    if let Some(note) = note {
        result["memory"] = Value::String(note.to_string());
    }
    if !refused.is_empty() {
        // Said here as well as in the report: the human is waiting on this
        // turn's sentence, and "it is running on a model you did not pick"
        // should not have to wait for the work to finish.
        result["refused"] = json!(refused);
        result["warning"] = Value::String(format!(
            "{agent_id} would not take an option this worker was opened with, so it is NOT running on what was asked for. Tell the human now, in the same sentence as what you delegated."
        ));
    }
    Ok(result)
}

/// How a worker turn is going, read off its own event stream: when the
/// agent last said anything, and which of its tool calls have not reported
/// an end.
///
/// This is what tells silence from work. An agent running `cargo test`
/// sends one `ToolCall` and then nothing for as long as the test takes, so
/// time since the last event says nothing on its own; time since the last
/// event *with no call outstanding* does.
#[derive(Default)]
struct Pulse {
    inner: parking_lot::Mutex<PulseInner>,
}

struct PulseInner {
    last: std::time::Instant,
    /// Tool call ids the agent started and has not ended.
    running: HashSet<String>,
}

impl Default for PulseInner {
    fn default() -> Self {
        Self { last: std::time::Instant::now(), running: HashSet::new() }
    }
}

impl Pulse {
    fn note(&self, event: &AgentEvent) {
        let ended = |status: &str| matches!(status, "completed" | "failed");
        let mut inner = self.inner.lock();
        inner.last = std::time::Instant::now();
        match event {
            AgentEvent::ToolCall { id, status, .. } => {
                if ended(status) {
                    inner.running.remove(id);
                } else {
                    inner.running.insert(id.clone());
                }
            }
            // An update with no status only names files; it does not end the call.
            AgentEvent::ToolUpdate { id, status, .. } if !status.is_empty() => {
                if ended(status) {
                    inner.running.remove(id);
                } else {
                    inner.running.insert(id.clone());
                }
            }
            _ => {}
        }
    }

    /// How long since the agent last said anything.
    fn quiet_for(&self) -> Duration {
        self.inner.lock().last.elapsed()
    }

    /// Whether every tool call it started has reported an end.
    fn nothing_running(&self) -> bool {
        self.inner.lock().running.is_empty()
    }
}

/// One worker turn: the prompt in flight, its events stored and shown, and a
/// watchdog over both clocks (see [`WORKER_QUIET_LIMIT`]).
async fn drive_worker_turn(app: &AppHandle, track: &str, name: &str, run: &str, session: &AgentSession, text: String, cwd: &str) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let pump_task = tauri::async_runtime::spawn(pump(app.clone(), track.to_string(), name.to_string(), run.to_string(), rx));
    let _ = tx.send(AgentEvent::Started {
        session_id: session.session_id().to_string(),
        cwd: cwd.to_string(),
    });

    // The agent's events go through a relay that keeps the pulse. Everything
    // is forwarded unchanged and in order; the relay ends when the turn
    // drops the agent's sender.
    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::unbounded_channel();
    let pulse = Arc::new(Pulse::default());
    let relay = {
        let (out, pulse) = (tx.clone(), pulse.clone());
        tauri::async_runtime::spawn(async move {
            while let Some(event) = agent_rx.recv().await {
                pulse.note(&event);
                if out.send(event).is_err() {
                    break;
                }
            }
        })
    };

    let started = std::time::Instant::now();
    let outcome = {
        let turn = session.prompt(text, agent_tx);
        tokio::pin!(turn);
        loop {
            tokio::select! {
                done = &mut turn => break Ok(done),
                _ = tokio::time::sleep(WORKER_QUIET_CHECK) => {
                    if started.elapsed() > WORKER_TURN_TIMEOUT {
                        break Err(format!("worker turn timed out after {} hours", WORKER_TURN_TIMEOUT.as_secs() / 3600));
                    }
                    let quiet = pulse.quiet_for();
                    // Quiet is only stuck when there is nothing to be quiet
                    // for: no tool call running (a build, a test suite) and
                    // no permission question it is waiting on an answer to.
                    if quiet > WORKER_QUIET_LIMIT && pulse.nothing_running() && session.waiting_questions().is_empty() {
                        break Err(format!("worker said nothing for {} minutes with nothing running", quiet.as_secs() / 60));
                    }
                }
            }
        }
    };
    // The turn is dropped with the block above, and with it the agent's
    // sender: the relay ends on its own.

    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            // Ended because the human stopped it (its session was killed
            // after the grace): say so, so the record is not read as a crash.
            let stopped = app.state::<AppState>().sessions.was_stopped(run);
            let error = if stopped { "stopped by the human".to_string() } else { err.to_string() };
            let _ = tx.send(AgentEvent::Failed { error });
        }
        Err(why) => {
            // Stuck; a cancel may not reach an agent that has stopped
            // answering, so the session ends and the worker reopens (with
            // its memory) on its next task.
            tracing::warn!(%track, worker = %name, %run, %why, "ending a stuck worker turn");
            session.cancel();
            session.kill();
            let _ = tx.send(AgentEvent::Failed { error: format!("{why}; its session was ended") });
            let st = app.state::<AppState>();
            let mut workers = st.sessions.workers.lock().await;
            if workers.get(&worker_key(track, name)).is_some_and(|l| std::ptr::eq(Arc::as_ptr(&l.session), session)) {
                workers.remove(&worker_key(track, name));
            }
        }
    }
    drop(tx);
    let _ = relay.await;
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

/// Whether a run ended normally (not failed, stopped or timed out). A
/// worker the human stopped is never asked again for its report block.
fn ended_well(app: &AppHandle, run: &str) -> bool {
    let state = app.state::<AppState>();
    if state.sessions.was_stopped(run) {
        return false;
    }
    matches!(state.store.run(run), Ok(Some(s)) if s.status == RunStatus::Done && !s.stop_reason.as_deref().is_some_and(|r| r.eq_ignore_ascii_case("cancelled")))
}

/// The report block of a run's reply, checked.
///
/// Read whatever the turn did. A worker can do the whole job, write its
/// block, and only then lose its session — to the hour cap, to an agent
/// that died, to the human stopping it. The words are already in the
/// record; refusing to look at them because the turn ended badly threw
/// away a finished report and handed the conductor an empty one. (That is
/// exactly what used to happen: parsing sat inside the `Done` arm alone,
/// so every other ending skipped it and fell straight to an error.)
///
/// How the turn ended is not lost either — it rides along on the report as
/// [`Report::interrupted`], because "the worker says done" and "the turn
/// ended by itself" are two different facts.
fn checked_report(app: &AppHandle, run: &str) -> Result<Report, String> {
    let state = app.state::<AppState>();
    let stopped = state.sessions.was_stopped(run);
    let Ok(Some(summary)) = state.store.run(run) else {
        return Err("the run is gone".to_string());
    };
    read_reply(&summary.output, ended_badly(&summary, stopped))
}

/// Take the block out of a reply. `cut` is how the turn ended when it did
/// not end on its own, and it only ever adds to what is known: a block that
/// parses keeps it as a note, and a block that does not is still refused —
/// half a JSON object is not a report, and guessing at one would be worse
/// than saying there is none.
fn read_reply(output: &str, cut: Option<String>) -> Result<Report, String> {
    match report::parse(output) {
        Ok(mut parsed) => {
            parsed.interrupted = cut;
            Ok(parsed)
        }
        // No usable block. When the turn was cut short, that is the better
        // explanation of why — and the fallback report says in so many
        // words that the whole output is in read_report.
        Err(err) => Err(match cut {
            Some(why) => format!("{why}, and what it had said holds no usable report block ({err})"),
            None => err,
        }),
    }
}

/// How a run ended, when it did not end on its own. `None` means it did.
fn ended_badly(summary: &RunSummary, stopped: bool) -> Option<String> {
    if stopped {
        return Some(STOPPED_BY_HUMAN.to_string());
    }
    if summary.status == RunStatus::Done {
        // The agent ended the turn itself; only a cancel makes that abnormal.
        return summary
            .stop_reason
            .as_deref()
            .filter(|reason| reason.eq_ignore_ascii_case("cancelled"))
            .map(|_| STOPPED_BY_HUMAN.to_string());
    }
    Some(format!(
        "the turn {}{}",
        summary.status.as_str(),
        summary.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
    ))
}

/// Why a stopped worker has no report block of its own.
const STOPPED_BY_HUMAN: &str = "the human stopped this worker mid-turn";

/// The report as kept: the worker's, or one made from its reply; with the
/// files the app saw its edit tools touch. Stored for the timeline's card.
fn finish_report(
    app: &AppHandle,
    run: &str,
    reminder: Option<&str>,
    outcome: Result<Report, String>,
    cwd: &str,
    ran_on: Option<RanOn>,
) -> Report {
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
    // What it ran on: the worker has no way to know, so the app says it.
    report.ran_on = ran_on;
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
                // Bounded, and an error rather than a panic if it runs out:
                // this is the one place a worker's folder name comes from
                // the folders that happen to be there, so it is the one
                // that could in principle find none free.
                (1..1000)
                    .map(|n| root.join(if n == 1 { base.clone() } else { format!("{base}-{n}") }))
                    .find(|d| !d.exists())
                    .ok_or_else(|| format!("no free folder name for worker {name} under {track_dir}"))?
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
    let stopped = state.sessions.was_stopped(&run) || summary.stop_reason.as_deref().is_some_and(|r| r.eq_ignore_ascii_case("cancelled"));
    let text = format!(
        "{REPORT_PREFIX} worker={worker} run={run} status={} tools={} duration_ms={}{}\n\n{}{}",
        if stopped { "stopped" } else { summary.status.as_str() },
        summary.tool_count,
        summary.duration_ms.unwrap_or(0),
        summary.error.as_ref().map(|e| format!(" error={e}")).unwrap_or_default(),
        report.for_conductor(&run),
        // A report with a block says this itself, on its NOTE line; this
        // is for when there was no block to carry it.
        if stopped && report.interrupted.is_none() {
            "\n(The human stopped this worker by hand. Nothing of it is running now. Tell them what it had got done and ask what they want next; do not start it again on your own.)"
        } else {
            ""
        },
    );
    state.sessions.forget_stopped(&run);

    let hand = Hand {
        track,
        text,
        open: false,
        what: format!("report of {worker} run {run}"),
        // An hour of work: it waits for the conductor rather than going.
        keep: true,
        worker: Some(worker),
        run: Some(run),
    };
    deliver(app.clone(), hand).await;
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
        // The conductor's own model not taking is the same failure as a
        // worker's, and nobody above it would notice on its behalf. Kept for
        // the next turn to carry rather than handed on here: handing it on
        // would mean opening the conductor from inside opening the conductor.
        let said: Vec<String> = session
            .refused_options()
            .iter()
            .map(|r| format!("{}={}: {}", r.option, r.value, r.error))
            .collect();
        if !said.is_empty() {
            tracing::warn!(%track, agent = %agent, refused = ?said, "conductor session did not take the options it was opened with");
            if let Err(err) = st.store.set_meta(&refused_key(track), &said.join("; ")) {
                tracing::warn!(%err, "could not keep what the conductor's session refused");
            }
        }
        if let Err(err) = st.store.set_meta(&key, session.session_id()) {
            tracing::warn!(%err, "could not remember conductor session id");
        }
        st.sessions.conductors.lock().await.insert(
            track.to_string(),
            Conductor {
                live: Live {
                    agent: agent.clone(),
                    config: info.conductor_config.clone(),
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

fn refused_key(track: &str) -> String {
    format!("refused:{track}")
}

/// What this track's conductor session refused when it opened, taken (it is
/// said once, on the next turn, and then it is said).
fn take_refused(st: &AppState, track: &str) -> Option<String> {
    let said = st.store.get_meta(&refused_key(track)).ok().flatten()?;
    let _ = st.store.delete_meta(&refused_key(track));
    Some(said).filter(|s| !s.trim().is_empty())
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
    let state = conductor_state(&app, &track).await;
    // Reports that waited for this conductor go in now.
    tauri::async_runtime::spawn(flush_parked(app.clone(), track));
    Ok(state)
}

/// Stop the conductor's turn in flight (Ctrl+C). The session stays open;
/// the run ends with the agent's `cancelled` stop reason.
///
/// Workers keep working. Each agent session is its own process in its own
/// job object (see `orchestra_acp::process`), and a worker's turn runs in a
/// task of its own, detached from the conductor turn that opened it: only
/// the conductor's session is touched here, never `sessions.workers`. Their
/// reports reach the conductor as usual once it is free. (Ending every
/// session of a track is `Sessions::close_track`, a different thing.)
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

/// Stop a worker's turn in flight, as [`conductor_cancel`] does for the
/// conductor. The agent is asked to stop as Ctrl+C would; its session stays
/// open, the run ends and stays in the record marked stopped (nothing is
/// erased), and its report — whatever it had got to — still reaches the
/// conductor.
///
/// An agent that does not stop is not left running behind a screen that
/// says it stopped: after the same grace the conductor gets, its session is
/// ended, which kills the agent process with its tree (each session is its
/// own job object, see [`orchestra_acp::process`]). The worker reopens with
/// its memory on its next task.
pub async fn worker_cancel(app: AppHandle, track: String, worker: String) -> Result<(), String> {
    let key = worker_key(&track, &worker);
    let found = {
        let st = app.state::<AppState>();
        let workers = st.sessions.workers.lock().await;
        let live = workers.get(&key).ok_or_else(|| format!("no open worker {worker}"))?;
        live.running.clone().map(|run| (run, live.session.clone()))
    };
    // Not in a turn: nothing to stop, and nothing to report as stopped.
    let Some((run, session)) = found else { return Ok(()) };
    app.state::<AppState>().sessions.mark_stopped(&run);
    tracing::info!(%track, %worker, %run, "stopping a worker turn");
    session.cancel();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CANCEL_GRACE).await;
        let st = app.state::<AppState>();
        let mut workers = st.sessions.workers.lock().await;
        let stuck = workers
            .get(&key)
            .is_some_and(|l| l.running.as_deref() == Some(run.as_str()) && Arc::ptr_eq(&l.session, &session));
        if stuck {
            tracing::warn!(%track, %worker, %run, "the worker did not stop after a cancel; ending its session");
            if let Some(old) = workers.remove(&key) {
                old.session.kill();
            }
        }
    });
    Ok(())
}

/// Close a worker's agent session. Its runs and its memory stay in the
/// record, so the same name reopens with its conversation. Refused while it
/// is in a turn: stop it first. `false` when it had no open session.
pub async fn worker_close(app: AppHandle, track: String, worker: String) -> Result<bool, String> {
    let st = app.state::<AppState>();
    let mut workers = st.sessions.workers.lock().await;
    let key = worker_key(&track, &worker);
    match workers.get(&key) {
        Some(live) if live.running.is_some() => Err(format!(
            "{worker} is still working on run {}; stop it or wait for its report first",
            live.running.as_deref().unwrap_or_default()
        )),
        Some(_) => {
            tracing::info!(%track, %worker, "closing a worker session (asked)");
            workers.remove(&key);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Close every worker session of a track that has nothing to do, in one go:
/// the list stops being a pile of idle agent processes. Workers in a turn
/// are left alone, and no record is touched. Returns the names closed.
pub async fn workers_tidy(app: AppHandle, track: String) -> Result<Vec<String>, String> {
    let st = app.state::<AppState>();
    let mut workers = st.sessions.workers.lock().await;
    let prefix = worker_key(&track, "");
    let idle: Vec<String> = workers
        .iter()
        .filter(|(key, live)| key.starts_with(&prefix) && live.running.is_none())
        .map(|(key, _)| key.clone())
        .collect();
    let mut closed = Vec::new();
    for key in idle {
        if workers.remove(&key).is_some() {
            closed.push(key.strip_prefix(&prefix).unwrap_or(&key).to_string());
        }
    }
    closed.sort();
    tracing::info!(%track, closed = closed.len(), "closed the worker sessions that had nothing to do");
    Ok(closed)
}

/// Take a deleted worker's reports off its track's waiting list.
///
/// Its runs and the reports kept against them are gone, so an entry still
/// pointing at one is worse than nothing: the conversation would draw a
/// card with no report behind it, and if the entry ever went through, the
/// conductor would be handed a report and told to call `read_report` on a
/// run that no longer exists. The emitted event is what tells the UI to
/// read the list again.
async fn forget_parked_of(app: &AppHandle, track: &str, worker: &str) {
    let _one = PARKED.lock().await;
    let state = app.state::<AppState>();
    let mut list = parked_list(&state.store, track);
    let had = list.len();
    without_worker(&mut list, worker);
    if list.len() == had {
        return;
    }
    keep_parked(&state.store, track, &list);
    tracing::info!(%track, %worker, gone = had - list.len(), "took a deleted worker's reports off the waiting list");
    let _ = app.emit(
        "parked",
        Waiting { track: track.to_string(), pending: list.len(), added: None, worker: None, run: None, why: String::new() },
    );
}

/// Drop a worker's entries from a waiting list. Everything else stays,
/// the conductor's own answers included.
fn without_worker(list: &mut Vec<Parked>, worker: &str) {
    list.retain(|item| item.worker.as_deref() != Some(worker));
}

/// Delete a worker's record: its runs with their events, their kept
/// reports, any decision hanging off them, the agent session the app would
/// have resumed, and its place in the conductor's waiting list. Returns how
/// many runs went.
///
/// What this never touches: the worker's folder and every file in it, and
/// its checkout. Those hold what the human asked for; the app does not
/// delete their results.
///
/// Refused while the worker is in a turn, and while it holds changes nobody
/// has merged — deleting the record would leave no way back to them.
///
/// The name is held in [`Sessions::deleting`] for the whole of it. Closing
/// a session does not stop the conductor reopening the worker with
/// `ask_worker`, and the unmerged-work check runs git, which takes long
/// enough for that to happen: a turn started in the gap would have its run
/// deleted under it, its events would fail the foreign key, and the work
/// would go unrecorded. With the name held, that turn is refused instead.
pub async fn worker_delete(app: AppHandle, track: String, worker: String) -> Result<u32, String> {
    /// The mark goes however the delete ends.
    struct Deleting<'a>(&'a parking_lot::Mutex<HashSet<String>>, String);
    impl Drop for Deleting<'_> {
        fn drop(&mut self) {
            self.0.lock().remove(&self.1);
        }
    }

    let key = worker_key(&track, &worker);
    let state = app.state::<AppState>();
    {
        // Under the `workers` lock, which is the one [`start_worker_turn`]
        // makes its own checks under: marking and checking have to be one
        // step, or a turn starting right now passes its check just before
        // the mark goes down and is deleted out from under itself.
        let workers = state.sessions.workers.lock().await;
        if workers.get(&key).is_some_and(|live| live.running.is_some()) {
            return Err(format!("{worker} is still working; stop it or wait for its report"));
        }
        // A session part-way through opening has no run yet, but it will:
        // its `begin_run` lands after the slow checks below would have finished.
        if state.sessions.opening.lock().contains(&key) {
            return Err(format!("{worker} is starting a turn; wait for that before deleting it"));
        }
        if !state.sessions.deleting.lock().insert(key.clone()) {
            return Err(format!("worker {worker} is already being deleted"));
        }
    }
    // Nothing awaits between leaving that block and taking the guard, so
    // the mark cannot be left behind.
    let _deleting = Deleting(&state.sessions.deleting, key);

    worker_close(app.clone(), track.clone(), worker.clone()).await?;
    worktree::check_nothing_pending(&state.store, &track, std::slice::from_ref(&worker))?;
    let gone = state.store.delete_worker(&track, &worker).map_err(|e| e.to_string())?;
    forget_parked_of(&app, &track, &worker).await;
    tracing::info!(%track, %worker, runs = gone, "deleted a worker's record; its folder and files stay");
    Ok(gone)
}

/// What the UI shows about one worker's agent session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerState {
    pub name: String,
    /// Its agent session is open (an agent process is alive).
    pub open: bool,
    /// A turn is in flight.
    pub running: bool,
    /// The run of that turn.
    pub run: Option<String>,
    pub agent: String,
    /// The model it is running on, when its agent offers a choice and one
    /// was made. Empty means the agent's own.
    pub model: String,
}

/// Every open worker session, by track (as [`conductor_states`] does for
/// conductors). Workers with no open session are not listed: the UI has
/// them from the record, which outlives sessions.
pub async fn worker_states(app: &AppHandle) -> Vec<(String, Vec<WorkerState>)> {
    let st = app.state::<AppState>();
    let mut by_track: BTreeMap<String, Vec<WorkerState>> = BTreeMap::new();
    for (key, live) in st.sessions.workers.lock().await.iter() {
        let Some((track, name)) = key.split_once('/') else { continue };
        by_track.entry(track.to_string()).or_default().push(WorkerState {
            name: name.to_string(),
            open: true,
            running: live.running.is_some(),
            run: live.running.clone(),
            agent: live.agent.clone(),
            model: model_of(&st, &live.agent, &live.config),
        });
    }
    for list in by_track.values_mut() {
        list.sort_by(|a, b| a.name.cmp(&b.name));
    }
    by_track.into_iter().collect()
}

/// Close conductor and worker sessions nobody has used for a while (the
/// `sessions.idle_minutes` setting, 30 by default, 0 for never): each is an
/// agent process holding memory. Their conversations stay in the store and
/// the agent's own record, so the next message reopens them with memory.
///
/// A conductor with a worker mid-turn is never closed, however long it has
/// been quiet. Waiting is not idling, and the wait is usually longer than the
/// limit: a worker that takes an hour is ordinary.
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
                // Tracks with a worker mid-turn. Their conductors are not
                // idle: they are waiting for something they started, and the
                // report is the only thing that crosses the membrane. Closing
                // one means the report arrives with nowhere to go, parks, and
                // then sits there -- a parked report deliberately does not
                // reopen a closed conductor (see `flush_parked`), so it waits
                // for the human to say something unrelated. A worker that runs
                // longer than the idle limit made that certain rather than
                // unlucky.
                let awaited = {
                    let workers = state.sessions.workers.lock().await;
                    tracks_of(workers.iter().filter(|(_, l)| l.running.is_some()).map(|(key, _)| key.as_str()))
                };
                let mut conductors = state.sessions.conductors.lock().await;
                let idle: Vec<String> = conductors
                    .iter()
                    .filter(|(track, c)| {
                        !state.sessions.is_busy(track)
                            && c.live.running.is_none()
                            && c.live.used.elapsed() > limit
                            && !awaited.contains(track)
                    })
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
        // Deliberately not worded like [`BUSY`]: that one means "not yet,
        // it will go on its own" and the UI treats it that way. This one
        // means the close did not happen.
        return Err("the conductor is mid-turn; stop it or wait for it before closing".to_string());
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
        let text = if first {
            let agents = agent_facts(&st, &lang, &info);
            format!("{}\n\n---\n\n{text}", preamble(&lang, &info, &agents))
        } else {
            text
        };
        // Said on the first turn after the session opened, whether or not
        // that was a fresh session: the human chose a model and did not get
        // it, and only the conductor can tell them.
        let text = match take_refused(&st, &track) {
            Some(said) => format!(
                "[divixi] {} would not take an option this session was opened with, so you are NOT running on what the human chose: {said}. Tell them this before anything else, then answer as usual.\n\n---\n\n{text}",
                info.agent
            ),
            None => text,
        };
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
            // Free again: whatever could not reach it earlier goes in now.
            tauri::async_runtime::spawn(flush_parked(app_for_turn.clone(), track_for_turn));
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
mod parked_list_tests {
    use super::*;

    fn report(worker: &str, run: &str, at: i64) -> Parked {
        Parked {
            id: park_id(at),
            text: format!("{REPORT_PREFIX} worker={worker} run={run} status=done\n\n{}", "x".repeat(4_000)),
            open: false,
            what: format!("report of {worker} run {run}"),
            worker: Some(worker.to_string()),
            run: Some(run.to_string()),
            at,
            why: "the conductor session is closed".to_string(),
            tries: 1,
        }
    }

    fn answer(at: i64, which: i64) -> Parked {
        Parked {
            id: park_id(at),
            text: format!("{DECISION_PREFIX} #{which}"),
            open: true,
            what: format!("decision #{which}"),
            worker: None,
            run: None,
            at,
            why: "the conductor stayed busy".to_string(),
            tries: 1,
        }
    }

    // ----- telling one waiting turn from another -----

    #[test]
    fn entries_are_told_apart_by_id() {
        let a = report("ui", "t001", 100);
        let b = report("ui", "t002", 100);
        assert!(a.is(&a.clone()));
        assert!(!a.is(&b), "two reports parked in the same millisecond are still two");

        // Two answers to one card, which is how a duplicate `what` arises.
        let one = answer(100, 3);
        let two = answer(100, 3);
        assert_eq!(one.what, two.what);
        assert_eq!(one.at, two.at);
        assert!(!one.is(&two), "same text, same instant, different entries");
    }

    #[test]
    fn entries_written_before_ids_fall_back_to_what_identified_them() {
        let mut old = report("ui", "t001", 100);
        let mut same = old.clone();
        old.id = String::new();
        same.id = String::new();
        assert!(old.is(&same), "no id either side: matched on run, text and instant");

        let mut other = report("ui", "t002", 100);
        other.id = String::new();
        assert!(!old.is(&other));

        // One side has an id and the other does not: the fallback still decides.
        let fresh = report("ui", "t001", 100);
        assert!(old.is(&fresh), "an entry rewritten with an id is still the same entry");
    }

    // ----- what a flush removes -----

    #[test]
    fn a_flush_removes_what_it_handed_on_not_whatever_is_first() {
        // The interleaving this guards: a flush copies the front entry,
        // hands it on, and by the time it comes back the list has moved.
        // Removing by position would drop a report nobody delivered.
        let delivered = report("ui", "t001", 100);
        let mut list = vec![report("api", "t002", 90), delivered.clone(), report("db", "t003", 110)];

        assert!(list.iter().any(|item| item.is(&delivered)));
        list.retain(|item| !item.is(&delivered));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].run.as_deref(), Some("t002"), "the one in front of it is untouched");
        assert_eq!(list[1].run.as_deref(), Some("t003"));

        // Handed on twice (two flushes raced): the second finds nothing to
        // remove and leaves the list alone rather than taking the next one.
        assert!(!list.iter().any(|item| item.is(&delivered)));
    }

    // ----- the list does not grow without bound -----

    #[test]
    fn the_newest_reports_keep_their_text_and_the_older_ones_are_folded() {
        let mut list: Vec<Parked> = (0..PARKED_FULL_TEXT + 5).map(|i| report("ui", &format!("t{i:03}"), i as i64)).collect();
        let full = list[0].text.len();
        fold_old(&mut list);

        for (i, item) in list.iter().enumerate() {
            if i < 5 {
                assert!(item.text.len() < full, "an older report is folded to a stub");
                assert!(item.text.starts_with(REPORT_PREFIX), "the header stays, so the timeline still finds its card");
                assert!(item.text.contains(&format!("read_report(\"t{i:03}\")")), "and it says where the report is");
                assert!(item.text.contains("worker=ui"));
            } else {
                assert_eq!(item.text.len(), full, "the newest keep their text");
            }
        }
        assert_eq!(list.len(), PARKED_FULL_TEXT + 5, "folding never drops an entry");
    }

    #[test]
    fn folding_is_idempotent_and_leaves_alone_what_it_cannot_fold() {
        let mut list: Vec<Parked> = (0..PARKED_FULL_TEXT + 2).map(|i| report("ui", &format!("t{i:03}"), i as i64)).collect();
        // A decision answer has no run, so there is nothing in the record
        // to fold it back to: its text is all there is of it.
        list.insert(0, answer(1, 7));
        let kept = list[0].text.clone();

        fold_old(&mut list);
        let once = list[1].text.clone();
        fold_old(&mut list);

        assert_eq!(list[1].text, once, "folding twice writes the same stub");
        assert_eq!(list[0].text, kept, "an answer with no run keeps its text");
    }

    #[test]
    fn a_short_list_is_left_entirely_alone() {
        let mut list: Vec<Parked> = (0..PARKED_FULL_TEXT).map(|i| report("ui", &format!("t{i:03}"), i as i64)).collect();
        let before: Vec<String> = list.iter().map(|item| item.text.clone()).collect();
        fold_old(&mut list);
        assert_eq!(list.iter().map(|item| item.text.clone()).collect::<Vec<_>>(), before);
    }

    // ----- a deleted worker leaves nothing behind -----

    #[test]
    fn deleting_a_worker_takes_its_waiting_reports_and_no_others() {
        // Its runs and its kept reports have just gone. An entry left
        // pointing at one would draw a card with nothing behind it, and if
        // it ever went through the conductor would be told to read a
        // report that is not there.
        let mut list = vec![report("ui", "t001", 100), answer(105, 3), report("api", "t002", 110), report("ui", "t003", 120)];
        without_worker(&mut list, "ui");

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].what, "decision #3", "the conductor's own answer stays");
        assert_eq!(list[1].worker.as_deref(), Some("api"), "so do other workers' reports");
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;
    use orchestra_core::report::{Outcome, Standing};

    /// A finished report, as a worker writes it.
    const FULL: &str = r#"I fixed the parser and checked it.

```divixi-report
{
  "status": "done",
  "summary": "Empty input no longer panics.",
  "changes": [{ "path": "src/parser.rs", "what": "handles empty input" }],
  "checks": [{ "what": "cargo test -p parser", "result": "pass", "detail": "12 tests" }],
  "risks": [],
  "questions": [],
  "next": []
}
```
"#;

    /// The same reply with the session cut mid-block: no closing fence.
    const TRUNCATED: &str = r#"I fixed the parser and checked it.

```divixi-report
{
  "status": "done",
  "summary": "Empty input no long"#;

    fn summary(status: RunStatus, error: Option<&str>, stop_reason: Option<&str>) -> RunSummary {
        RunSummary {
            id: "t042".to_string(),
            track: "tr001".to_string(),
            session: "fix-parser".to_string(),
            agent: "claude_code".to_string(),
            prompt: "fix the parser".to_string(),
            cwd: ".".to_string(),
            status,
            started_at: 0,
            duration_ms: Some(3_600_000),
            session_id: Some("s1".to_string()),
            stop_reason: stop_reason.map(str::to_string),
            error: error.map(str::to_string),
            output: String::new(),
            plan: Vec::new(),
            tool_count: 40,
        }
    }

    // ----- what the record says about how a turn ended -----

    #[test]
    fn a_turn_that_ended_on_its_own_was_not_cut_short() {
        assert_eq!(ended_badly(&summary(RunStatus::Done, None, Some("end_turn")), false), None);
    }

    #[test]
    fn a_timeout_a_death_and_a_stop_are_all_cut_short() {
        let timed_out = ended_badly(&summary(RunStatus::Failed, Some("worker turn timed out after 4 hours; its session was ended"), None), false);
        assert!(timed_out.unwrap().contains("timed out"), "the reason the turn ended is carried, not just that it did");

        let died = ended_badly(&summary(RunStatus::Failed, Some("agent session ended during the turn"), None), false);
        assert!(died.unwrap().contains("agent session ended"));

        // Stopped by the human, both ways it can land: the agent took the
        // cancel and ended the turn, or its session was killed after the grace.
        assert_eq!(ended_badly(&summary(RunStatus::Done, None, Some("cancelled")), false).as_deref(), Some(STOPPED_BY_HUMAN));
        assert_eq!(ended_badly(&summary(RunStatus::Failed, Some("stopped by the human"), None), true).as_deref(), Some(STOPPED_BY_HUMAN));
    }

    // ----- reading the reply -----

    #[test]
    fn a_finished_report_survives_a_turn_that_was_cut_short() {
        // The bug this is here for: a worker did the whole job and wrote its
        // block, then hit the turn cap. The app read none of it and handed
        // the conductor status=failed with every list empty.
        let cut = Some("the turn failed: worker turn timed out after 4 hours; its session was ended".to_string());
        let read = read_reply(FULL, cut.clone()).expect("a complete block is read whatever the turn did");

        assert_eq!(read.status, Standing::Done, "the worker's own word on its work stands");
        assert!(read.structured);
        assert_eq!(read.changes.len(), 1);
        assert_eq!(read.changes[0].path, "src/parser.rs");
        assert_eq!(read.checks.len(), 1);
        assert_eq!(read.checks[0].result, Outcome::Pass);

        // …and the other fact is not lost with it.
        assert_eq!(read.interrupted, cut, "the turn did not end on its own, and the report says so");
        let told = read.for_conductor("t042");
        assert!(told.contains("did not end on its own"), "the conductor is told, not left to infer it from the header");
        assert!(told.contains("timed out"));
        assert!(told.contains("read_report(\"t042\")"));
    }

    #[test]
    fn a_report_from_a_turn_that_ended_well_carries_no_such_note() {
        let read = read_reply(FULL, None).expect("a complete block is read");
        assert_eq!(read.interrupted, None);
        assert!(!read.for_conductor("t042").contains("did not end on its own"));
    }

    #[test]
    fn a_truncated_block_is_refused_rather_than_guessed_at() {
        // Half a JSON object is not a report. Refusing sends it down the
        // unstructured path, which keeps the reply as the summary and points
        // at read_report — better than inventing fields nobody wrote.
        let cut = Some("the turn failed: agent session ended during the turn".to_string());
        let err = read_reply(TRUNCATED, cut).expect_err("a block that was cut off is not a report");
        assert!(err.contains("agent session ended"), "why the turn ended explains why the block is half there");
        assert!(err.contains("no usable report block"));

        // What the conductor ends up with still reaches the whole output.
        let made = Report::unstructured(false, TRUNCATED, &err);
        assert!(!made.structured);
        assert_eq!(made.status, Standing::Failed);
        let told = made.for_conductor("t042");
        assert!(told.contains("read_report(\"t042\")"), "the way to the full output is always given");
        assert!(told.contains("agent session ended"));
    }

    #[test]
    fn a_reply_with_no_block_at_all_says_so() {
        let plain = "I had a look but ran out of time.";

        let cut = read_reply(plain, Some("the turn failed: killed".to_string())).expect_err("no block, no report");
        assert!(cut.contains("killed"));
        assert!(cut.contains("no usable report block"));

        // A turn that ended on its own gets the parser's own words, which say
        // what is missing; that is what the reminder run is written from.
        let clean = read_reply(plain, None).expect_err("no block, no report");
        assert!(!clean.contains("no usable report block"), "nothing was cut short, so nothing about the turn is added: {clean}");
    }

    #[test]
    fn a_stopped_worker_that_managed_a_block_keeps_it() {
        // Stopping usually lands before the block is written, but when it
        // does not there is nothing to gain by throwing the block away.
        let read = read_reply(FULL, Some(STOPPED_BY_HUMAN.to_string())).expect("its own words, written before the stop");
        assert_eq!(read.status, Standing::Done);
        assert_eq!(read.interrupted.as_deref(), Some(STOPPED_BY_HUMAN));
    }
}

#[cfg(test)]
mod pulse_tests {
    use super::*;

    fn call(id: &str, status: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            id: id.to_string(),
            title: "cargo test".to_string(),
            tool_kind: "execute".to_string(),
            status: status.to_string(),
            paths: Vec::new(),
        }
    }

    fn update(id: &str, status: &str) -> AgentEvent {
        AgentEvent::ToolUpdate { id: id.to_string(), status: status.to_string(), title: String::new(), paths: Vec::new() }
    }

    #[test]
    fn a_long_running_tool_call_is_not_silence() {
        let pulse = Pulse::default();
        assert!(pulse.nothing_running(), "a turn starts with nothing outstanding");

        // The agent starts a test run and then says nothing for as long as
        // it takes. That is the case the watchdog must not kill.
        pulse.note(&call("c1", "pending"));
        assert!(!pulse.nothing_running());
        pulse.note(&update("c1", "inprogress"));
        assert!(!pulse.nothing_running());
        // Files named mid-call, no status: the call has not ended.
        pulse.note(&AgentEvent::ToolUpdate { id: "c1".to_string(), status: String::new(), title: String::new(), paths: vec!["a.rs".to_string()] });
        assert!(!pulse.nothing_running());

        pulse.note(&update("c1", "completed"));
        assert!(pulse.nothing_running(), "once it ends, quiet is quiet again");
    }

    #[test]
    fn calls_are_counted_one_by_one_and_a_failure_ends_one_too() {
        let pulse = Pulse::default();
        pulse.note(&call("c1", "pending"));
        pulse.note(&call("c2", "pending"));
        pulse.note(&update("c1", "completed"));
        assert!(!pulse.nothing_running(), "the other one is still going");
        pulse.note(&update("c2", "failed"));
        assert!(pulse.nothing_running());
    }

    #[test]
    fn a_call_that_arrives_already_finished_leaves_nothing_outstanding() {
        let pulse = Pulse::default();
        pulse.note(&call("c1", "completed"));
        assert!(pulse.nothing_running());
    }

    #[test]
    fn prose_and_thinking_keep_the_pulse_without_touching_what_runs() {
        let pulse = Pulse::default();
        pulse.note(&call("c1", "pending"));
        pulse.note(&AgentEvent::Message { text: "working on it".to_string() });
        pulse.note(&AgentEvent::Thought { text: "hmm".to_string() });
        assert!(!pulse.nothing_running());
        assert!(pulse.quiet_for() < Duration::from_secs(5), "it just said something");
    }
}

#[cfg(test)]
mod parked_tests {
    use super::*;

    #[test]
    fn what_the_conductor_could_not_take_is_kept_in_order_and_cleared() {
        let store = orchestra_store::Store::in_memory().unwrap();
        assert!(parked_list(&store, "tr001").is_empty(), "nothing waits to begin with");

        let report = Parked {
            id: park_id(1),
            text: format!("{REPORT_PREFIX} worker=ui run=t002 status=done"),
            open: false,
            what: "report of ui run t002".to_string(),
            worker: Some("ui".to_string()),
            run: Some("t002".to_string()),
            at: 1,
            why: "the conductor session is closed".to_string(),
            tries: 1,
        };
        let answer = Parked {
            // Written before ids existed: it reads back with none, and
            // `Parked::is` falls back for it.
            id: String::new(),
            text: format!("{DECISION_PREFIX} #3"),
            open: true,
            what: "decision #3".to_string(),
            worker: None,
            run: None,
            at: 2,
            why: "the conductor stayed busy".to_string(),
            tries: 1,
        };
        keep_parked(&store, "tr001", &[report, answer]);

        let waiting = parked_list(&store, "tr001");
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0].run.as_deref(), Some("t002"), "oldest first: the report goes in before what came after it");
        assert_eq!(waiting[1].what, "decision #3");
        assert!(!waiting[0].open, "a report still does not open a closed conductor");
        assert!(!waiting[0].id.is_empty(), "the id comes back with the entry");
        assert!(waiting[1].id.is_empty(), "and an older entry reads back without one");

        // One handed on: the rest stays.
        keep_parked(&store, "tr001", &waiting[1..]);
        assert_eq!(parked_list(&store, "tr001").len(), 1);

        keep_parked(&store, "tr001", &[]);
        assert!(parked_list(&store, "tr001").is_empty());
        assert_eq!(store.get_meta(&parked_key("tr001")).unwrap(), None, "an emptied list leaves nothing behind");
    }

    #[test]
    fn a_waiting_list_that_cannot_be_read_is_not_taken_for_an_empty_one_being_written() {
        let store = orchestra_store::Store::in_memory().unwrap();
        store.set_meta(&parked_key("tr001"), "not json").unwrap();
        assert!(parked_list(&store, "tr001").is_empty(), "unreadable reads as nothing to hand on");
        // and writing over it works, so the track is not stuck.
        keep_parked(&store, "tr001", &[Parked {
            id: park_id(3),
            text: "x".to_string(),
            open: false,
            what: "report of ui run t009".to_string(),
            worker: Some("ui".to_string()),
            run: Some("t009".to_string()),
            at: 3,
            why: String::new(),
            tries: 1,
        }]);
        assert_eq!(parked_list(&store, "tr001").len(), 1);
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;

    fn card(ids: &[&str]) -> Decision {
        Decision {
            id: 1,
            track: "tr001".to_string(),
            run: None,
            question: "?".to_string(),
            context: String::new(),
            options: ids
                .iter()
                .map(|id| DecisionOption { label: "x".to_string(), detail: String::new(), id: id.to_string() })
                .collect(),
            recommended: None,
            allow_other: false,
            status: DecisionStatus::Open,
            choice: None,
            answer: None,
            note: String::new(),
            created_at: 0,
            decided_at: None,
            permission: None,
        }
    }

    /// The two kinds of card are answered in completely different places —
    /// one wakes a spawn, the other becomes a conductor turn — so telling
    /// them apart has to be exact, not nearly right.
    #[test]
    fn proposal_cards_are_told_apart_from_the_conductors_own_questions() {
        assert!(is_proposal(&card(&["agent:codex", "agent:claude_code"])));
        // What `request_decision` makes: options carry no id at all.
        assert!(!is_proposal(&card(&["", ""])));
        // A permission card's ids are the agent's own answers.
        assert!(!is_proposal(&card(&["allow_once", "reject_once"])));
        // Half-and-half is not a proposal either: all of them or none.
        assert!(!is_proposal(&card(&["agent:codex", ""])));
        assert!(!is_proposal(&card(&[])));
    }

    /// Naming an agent marks it as the conductor's suggestion on the card;
    /// naming nothing still raises one. The card does not depend on this.
    #[test]
    fn an_ask_knows_whether_it_names_anything() {
        assert!(!Ask::default().names_something());
        assert!(Ask { agent: Some("codex".into()), ..Ask::default() }.names_something());
        assert!(Ask { model: Some("gpt-5.2".into()), ..Ask::default() }.names_something());
    }

    /// The way past a card is spent by using it. Without that the conductor
    /// could carry one answer back twice, or answer its own card forever:
    /// every spawn on such a track raises a card, so a `decided` that kept
    /// working would be a loop with the human's name on it.
    #[test]
    fn the_way_past_a_card_works_once_and_only_for_its_own_worker() {
        let s = Sessions::default();
        s.mark_settled(7, "scan-acp");
        assert_eq!(s.settled_worker(7).as_deref(), Some("scan-acp"), "peeking does not spend it");
        assert_eq!(s.settled_worker(7).as_deref(), Some("scan-acp"));
        assert_eq!(s.take_settled(7).as_deref(), Some("scan-acp"));
        assert_eq!(s.take_settled(7), None, "a card lets exactly one spawn through");
        assert_eq!(s.take_settled(9), None, "an id nobody handed out lets nothing through");
    }

    /// Words instead of a button are not a button that happens to be absent:
    /// they go to the conductor to interpret, and must never be read as an
    /// option index.
    #[test]
    fn an_own_answer_is_not_an_option_index() {
        let s = Sessions::default();
        let mut rx = s.await_proposal(3);
        assert!(s.resolve_proposal(3, Some(OWN_ANSWER)));
        // `usize::MAX`, so it is past any index a card's options could have
        // and can never be mistaken for one of them.
        assert_eq!(rx.try_recv().unwrap(), Some(OWN_ANSWER));
    }
}

#[cfg(test)]
mod waiting_tests {
    use super::{tracks_of, worker_key};

    /// A conductor is spared the idle sweep while a worker of its own is
    /// mid-turn, and the sweep learns which track that is from the session
    /// key. Round-tripped against `worker_key` so the two cannot drift.
    #[test]
    fn a_worker_key_says_which_track_is_waiting() {
        let keys = [worker_key("tr002", "routine-dag"), worker_key("tr007", "repo-clean")];
        assert_eq!(tracks_of(keys.iter().map(String::as_str)), vec!["tr002".to_string(), "tr007".to_string()]);

        // A worker is named by a human, so the name can hold a slash. Only
        // the first one divides, or the track would come back truncated and
        // its conductor would be closed out from under a running worker.
        let odd = worker_key("tr002", "fix/parser");
        assert_eq!(odd, "tr002/fix/parser");
        assert_eq!(tracks_of([odd.as_str()]), vec!["tr002".to_string()]);

        // Nothing running, nothing spared.
        assert!(tracks_of(Vec::<&str>::new()).is_empty());
        // A key with no slash names no track rather than naming itself.
        assert!(tracks_of(["malformed"]).is_empty());
    }
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
