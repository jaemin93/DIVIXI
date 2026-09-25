//! A long-lived agent session: one process, one ACP session, many prompts.
//!
//! [`run_prompt`](crate::run_prompt) is the one-shot form: open, prompt, close.
//! The conductor needs the other form, because its whole point is context
//! that survives across turns, and so do sessions once the conductor can ask
//! them follow-up questions. [`AgentSession`] owns the process and the
//! protocol task; each [`AgentSession::prompt`] runs one turn and streams
//! that turn's [`AgentEvent`]s to the channel given for it, ending with
//! `Finished` or `Failed`.
//!
//! A session can also be **resumed** across app restarts: given the id of a
//! session the same agent created earlier, `session/load` brings it back
//! with its conversation (Claude Code's `--resume`, in ACP terms). The
//! agent replays the old history as notifications; those are drained here
//! so they never surface as part of a new turn.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::{
    schema::{
        v1::{
            HttpHeader, InitializeRequest, LoadSessionRequest, McpServer, McpServerHttp,
            CancelNotification, NewSessionRequest, SessionConfigOptionValue, SessionNotification,
            PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
            SelectedPermissionOutcome, SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason,
        },
        ProtocolVersion,
    },
    util::MatchDispatch,
    ActiveSession, Agent, Client, Responder, SessionMessage,
};
use orchestra_core::{AgentEvent, PermissionChoice, SlashCommand};
use tokio::sync::{mpsc, oneshot};

use crate::{is_autonomous_mode, pick_autonomous_mode, pick_restricted_mode, process, translate, AgentSpec};

/// An MCP server the agent should connect to over HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttp {
    pub name: String,
    pub url: String,
    /// `(name, value)` headers, e.g. a bearer token.
    pub headers: Vec<(String, String)>,
}

/// How to open a session.
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub cwd: PathBuf,
    /// Explicit mode id; `None` picks the most autonomous mode advertised.
    pub mode: Option<String>,
    /// `(option id, value id)` pairs, e.g. the model.
    pub config: Vec<(String, String)>,
    /// MCP servers to hand the agent in `session/new` or `session/load`.
    pub mcp_servers: Vec<McpHttp>,
    /// A session id from an earlier run of the same agent to bring back with
    /// its history. When loading fails, a fresh session is opened instead and
    /// [`AgentSession::resumed`] says so.
    pub resume: Option<String>,
    /// Never act unasked: run in the most restrictive mode the agent offers
    /// (plan, read-only, ask, default), never fall back to an autonomous
    /// one, and refuse to open when only autonomous modes are left. For
    /// sessions fed untrusted text. `mode` is ignored.
    pub restricted: bool,
}

/// How long to wait for more replayed history after `session/load`.
const REPLAY_QUIET: Duration = Duration::from_millis(400);

/// One turn's request: the text, the files with it, and where its events go.
struct Turn {
    text: String,
    files: Vec<PathBuf>,
    tx: mpsc::UnboundedSender<AgentEvent>,
    done: oneshot::Sender<anyhow::Result<()>>,
}

/// What the session task reports once the session exists.
struct Ready {
    session_id: String,
    resumed: bool,
}

/// Permission questions the agent is waiting on, by Divixi's request id.
type Pending = Arc<Mutex<PendingQuestions>>;

#[derive(Default)]
struct PendingQuestions {
    /// Stamped on every id this session gives out, so an id held from an
    /// earlier session (a card left open, a stale message) can never name
    /// a question this one asks.
    prefix: String,
    next: u64,
    waiting: std::collections::HashMap<String, Responder<RequestPermissionResponse>>,
}

impl PendingQuestions {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self { prefix: format!("q{:x}", (nanos as u64) ^ ((std::process::id() as u64) << 40)), ..Default::default() }
    }
}

/// Answer every question still waiting with "cancelled".
fn refuse_all(pending: &Pending) {
    let waiting: Vec<_> = pending.lock().map(|mut p| p.waiting.drain().collect()).unwrap_or_default();
    for (_, responder) in waiting {
        let _ = responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled));
    }
}

/// A permission question as an event, and the question kept to answer.
fn hold_question(pending: &Pending, req: RequestPermissionRequest, responder: Responder<RequestPermissionResponse>) -> AgentEvent {
    let fields = &req.tool_call.fields;
    let kind = |k: &PermissionOptionKind| {
        match k {
            PermissionOptionKind::AllowOnce => "allow_once",
            PermissionOptionKind::AllowAlways => "allow_always",
            PermissionOptionKind::RejectOnce => "reject_once",
            PermissionOptionKind::RejectAlways => "reject_always",
            _ => "other",
        }
        .to_string()
    };
    let mut input = fields.raw_input.as_ref().map(|v| v.to_string()).unwrap_or_default();
    if input.chars().count() > 2000 {
        input = format!("{}…", input.chars().take(2000).collect::<String>());
    }
    let options = req
        .options
        .iter()
        .map(|o| PermissionChoice { id: o.option_id.to_string(), name: o.name.clone(), kind: kind(&o.kind) })
        .collect();
    let request = match pending.lock() {
        Ok(mut p) => {
            p.next += 1;
            let id = format!("{}-{}", p.prefix, p.next);
            p.waiting.insert(id.clone(), responder);
            id
        }
        Err(_) => String::new(),
    };
    AgentEvent::Permission {
        request,
        title: fields.title.clone().unwrap_or_else(|| req.tool_call.tool_call_id.to_string()),
        tool_kind: fields.kind.as_ref().map(|k| format!("{k:?}").to_lowercase()).unwrap_or_default(),
        input,
        options,
    }
}

/// A live session. Dropping it ends the session and kills the agent.
pub struct AgentSession {
    turns: mpsc::Sender<Turn>,
    /// One message per cancel request; the task forwards it as `session/cancel`.
    cancel: mpsc::UnboundedSender<()>,
    session_id: String,
    resumed: bool,
    /// The latest slash-command list the agent sent.
    commands: Arc<Mutex<Vec<SlashCommand>>>,
    /// Permission questions waiting for an answer.
    pending: Pending,
    task: tokio::task::JoinHandle<()>,
}

/// Keep the newest command list out of a batch of events.
fn remember_commands(slot: &Arc<Mutex<Vec<SlashCommand>>>, events: &[AgentEvent]) {
    if let Some(AgentEvent::Commands { commands }) = events.iter().rev().find(|e| matches!(e, AgentEvent::Commands { .. })) {
        if let Ok(mut c) = slot.lock() {
            *c = commands.clone();
        }
    }
}

type ReadyTx = oneshot::Sender<anyhow::Result<Ready>>;

impl AgentSession {
    /// Launch the agent, initialize, create (or load) a session and apply
    /// mode and options. Resolves once the session exists, so a failure to
    /// start (auth, bad adapter) is reported here, not on the first prompt.
    pub async fn open(agent: &AgentSpec, opts: SessionOptions) -> anyhow::Result<Self> {
        let (process, transport) = process::spawn(agent)?;
        let (turn_tx, mut turn_rx) = mpsc::channel::<Turn>(1);
        let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel::<()>();
        let (ready_tx, ready_rx) = oneshot::channel::<anyhow::Result<Ready>>();
        let opts_for_task = opts.clone();
        let commands: Arc<Mutex<Vec<SlashCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let commands_task = commands.clone();
        let pending: Pending = Arc::new(Mutex::new(PendingQuestions::new()));
        let pending_task = pending.clone();

        let task = tokio::spawn(async move {
            let ready: Arc<Mutex<Option<ReadyTx>>> = Arc::new(Mutex::new(Some(ready_tx)));
            let ready_inner = ready.clone();
            let outcome = Client
                .builder()
                .name("orchestra-session")
                .connect_with(transport, async move |cx| {
                    let init = cx
                        .send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    let can_load = init.agent_capabilities.load_session;
                    // Whether pictures can go inline rather than as links.
                    let images = init.agent_capabilities.prompt_capabilities.image;

                    let mcp = |o: &SessionOptions| -> Vec<McpServer> {
                        o.mcp_servers
                            .iter()
                            .map(|m| {
                                let mut http = McpServerHttp::new(m.name.clone(), m.url.clone());
                                http.headers = m
                                    .headers
                                    .iter()
                                    .map(|(n, v)| HttpHeader::new(n.clone(), v.clone()))
                                    .collect();
                                McpServer::Http(http)
                            })
                            .collect()
                    };

                    // Bring an earlier session back, or open a fresh one.
                    let mut resumed = false;
                    // Command lists seen during a replay, handed to the first turn.
                    let mut carried: Vec<AgentEvent> = Vec::new();
                    let mut session: Option<ActiveSession<'static, Agent>> = None;
                    if let Some(id) = opts_for_task.resume.clone().filter(|_| can_load) {
                        let request = LoadSessionRequest::new(id.clone(), opts_for_task.cwd.clone())
                            .mcp_servers(mcp(&opts_for_task));
                        match cx.load_session_from(request).block_task().start_session().await {
                            Ok(restored) => {
                                let mut s = restored.into_session();
                                let (replayed, kept) = drain_replay(&mut s).await;
                                tracing::info!(session = %id, replayed, kept = kept.len(), "resumed agent session");
                                resumed = true;
                                carried = kept;
                                session = Some(s);
                            }
                            Err(err) => {
                                tracing::info!(session = %id, %err, "session/load failed; opening a fresh session");
                            }
                        }
                    }
                    let mut session = match session {
                        Some(s) => s,
                        None => {
                            let request = NewSessionRequest::new(opts_for_task.cwd.clone())
                                .mcp_servers(mcp(&opts_for_task));
                            cx.build_session_from(request).block_task().start_session().await?
                        }
                    };

                    let session_id = session.session_id().clone();
                    let available: Vec<(String, String)> = session
                        .modes()
                        .map(|m| {
                            m.available_modes
                                .iter()
                                .map(|x| (x.id.to_string(), x.name.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let current = session.modes().map(|m| m.current_mode_id.to_string());
                    tracing::info!(?available, ?current, "session modes");
                    let restricted = opts_for_task.restricted;
                    let fallback = if restricted { None } else { pick_autonomous_mode(&available) };
                    let chosen = if restricted { pick_restricted_mode(&available) } else { opts_for_task.mode.clone() };
                    if restricted && chosen.is_none() && current.as_deref().is_some_and(is_autonomous_mode) {
                        return Err(agent_client_protocol::util::internal_error(
                            "the agent offers no mode that asks before acting",
                        ));
                    }
                    if let Some(wanted) = chosen.or_else(|| fallback.clone()) {
                        if current.as_deref() != Some(wanted.as_str()) {
                            tracing::info!(mode = %wanted, "setting session mode");
                            let set = session
                                .connection()
                                .send_request_to(Agent, SetSessionModeRequest::new(session_id.clone(), wanted.clone()))
                                .block_task()
                                .await;
                            match (set, fallback.filter(|f| *f != wanted)) {
                                (Ok(_), _) => {}
                                // A stored mode id the agent no longer knows: take
                                // its autonomous mode rather than failing the open.
                                (Err(err), Some(other)) => {
                                    tracing::warn!(mode = %wanted, %err, fallback = %other, "agent rejected the session mode");
                                    session
                                        .connection()
                                        .send_request_to(Agent, SetSessionModeRequest::new(session_id.clone(), other))
                                        .block_task()
                                        .await?;
                                }
                                (Err(err), None) => return Err(err),
                            }
                        }
                    }
                    for (option, value) in &opts_for_task.config {
                        tracing::info!(option, value, "setting session option");
                        let request = SetSessionConfigOptionRequest::new(
                            session_id.clone(),
                            option.clone(),
                            SessionConfigOptionValue::ValueId { value: value.clone().into() },
                        );
                        if let Err(err) = session.connection().send_request_to(Agent, request).block_task().await {
                            tracing::info!(option, value, %err, "agent rejected session option");
                        }
                    }

                    // A fresh session's first notifications (the slash-command
                    // list, sent right after session/new) arrive before any
                    // prompt; take them now so a client can ask for them.
                    if !resumed {
                        let (_, kept) = drain_replay(&mut session).await;
                        carried.extend(kept);
                    }
                    remember_commands(&commands_task, &carried);

                    if let Some(ready) = ready_inner.lock().ok().and_then(|mut r| r.take()) {
                        let _ = ready.send(Ok(Ready {
                            session_id: format!("{session_id}"),
                            resumed,
                        }));
                    }

                    // Turn loop: one prompt at a time, until the handle is dropped.
                    while let Some(turn) = turn_rx.recv().await {
                        for ev in carried.drain(..) {
                            let _ = turn.tx.send(ev);
                        }
                        // A cancel asked for before this turn is not for this turn.
                        while cancel_rx.try_recv().is_ok() {}
                        let result = run_turn(&mut session, &turn.text, &turn.files, images, &turn.tx, &commands_task, &pending_task, &mut cancel_rx).await;
                        // A question the turn ended without is moot; nobody may answer it later.
                        refuse_all(&pending_task);
                        if let Err(err) = &result {
                            let _ = turn.tx.send(AgentEvent::Failed { error: err.to_string() });
                        }
                        let fatal = result.as_ref().err().map(|e| e.to_string());
                        let _ = turn.done.send(result);
                        if let Some(cause) = fatal {
                            // The connection is in an unknown state; end the session.
                            tracing::warn!(%cause, "turn failed; ending the agent session");
                            return Err(agent_client_protocol::Error::internal_error());
                        }
                    }
                    Ok(())
                })
                .await;

            if let Err(err) = &outcome {
                tracing::info!(%err, "agent session ended with error");
            }
            if let Some(ready) = ready.lock().ok().and_then(|mut r| r.take()) {
                let msg = match &outcome {
                    Err(err) => format!("session failed to start: {err}"),
                    Ok(()) => "session ended before it was ready".to_string(),
                };
                let _ = ready.send(Err(anyhow::anyhow!(super::with_stderr(msg, &process))));
            }
            process.shutdown().await;
        });

        match ready_rx.await {
            Ok(Ok(ready)) => Ok(Self {
                turns: turn_tx,
                cancel: cancel_tx,
                session_id: ready.session_id,
                resumed: ready.resumed,
                commands,
                pending,
                task,
            }),
            Ok(Err(err)) => {
                task.abort();
                Err(err)
            }
            Err(_) => {
                task.abort();
                anyhow::bail!("agent session task ended before the session was ready")
            }
        }
    }

    /// The agent's session id. Persist it to resume later.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Whether this session was brought back from an earlier one.
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    /// The slash commands the agent last advertised for this session.
    pub fn commands(&self) -> Vec<SlashCommand> {
        self.commands.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Run one turn. Events stream to `tx` and end with `Finished` or
    /// `Failed`; the future resolves when the turn is over.
    pub async fn prompt(&self, text: String, tx: mpsc::UnboundedSender<AgentEvent>) -> anyhow::Result<()> {
        self.prompt_with(text, Vec::new(), tx).await
    }

    /// Run one turn with files attached: each goes as a link the agent can
    /// read, or inline when it is a picture and the agent takes pictures.
    pub async fn prompt_with(&self, text: String, files: Vec<PathBuf>, tx: mpsc::UnboundedSender<AgentEvent>) -> anyhow::Result<()> {
        let (done_tx, done_rx) = oneshot::channel();
        self.turns
            .send(Turn { text, files, tx, done: done_tx })
            .await
            .map_err(|_| anyhow::anyhow!("agent session is closed"))?;
        done_rx
            .await
            .map_err(|_| anyhow::anyhow!("agent session ended during the turn"))?
    }

    /// Stop the turn in flight, as Ctrl+C would in the agent's own terminal.
    /// The agent ends the turn with a `cancelled` stop reason; the session
    /// stays open. Nothing happens when no turn is running.
    pub fn cancel(&self) {
        // Questions still waiting are answered "cancelled", as ACP asks of a
        // client that cancels a turn.
        refuse_all(&self.pending);
        let _ = self.cancel.send(());
    }

    /// Answer a permission question the agent is waiting on: one of the
    /// options it offered, or `None` to cancel. Errors when no question has
    /// that id (answered already, or the session was reopened since).
    pub fn answer_permission(&self, request: &str, option: Option<&str>) -> anyhow::Result<()> {
        let responder = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("permission questions are unavailable"))?
            .waiting
            .remove(request)
            .ok_or_else(|| anyhow::anyhow!("no permission question {request} is waiting"))?;
        let outcome = match option {
            Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id.to_string())),
            None => RequestPermissionOutcome::Cancelled,
        };
        responder
            .respond(RequestPermissionResponse::new(outcome))
            .map_err(|e| anyhow::anyhow!("could not answer the agent: {e}"))
    }

    /// Permission questions the agent is waiting on now.
    pub fn waiting_questions(&self) -> Vec<String> {
        self.pending.lock().map(|p| p.waiting.keys().cloned().collect()).unwrap_or_default()
    }

    /// End the session and wait for the agent to go away.
    pub async fn close(self) {
        drop(self.turns);
        let _ = self.task.await;
    }
}

/// Consume the history an agent replays after `session/load`. Returns how
/// many messages were dropped and the command lists among them, which are
/// not history and belong to the session ahead. The replay has no end
/// marker; a quiet gap is taken as the end.
async fn drain_replay(session: &mut ActiveSession<'_, Agent>) -> (usize, Vec<AgentEvent>) {
    let mut n = 0;
    let mut kept = Vec::new();
    loop {
        match tokio::time::timeout(REPLAY_QUIET, session.read_update()).await {
            Ok(Ok(SessionMessage::SessionMessage(dispatch))) => {
                n += 1;
                let _ = MatchDispatch::new(dispatch)
                    .if_notification(async |notif: SessionNotification| {
                        for ev in translate(notif.update) {
                            if matches!(ev, AgentEvent::Commands { .. }) {
                                kept.push(ev);
                            }
                        }
                        Ok(())
                    })
                    .await
                    .otherwise_ignore();
            }
            Ok(Ok(_)) => n += 1,
            Ok(Err(_)) | Err(_) => return (n, kept),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    session: &mut ActiveSession<'_, Agent>,
    text: &str,
    files: &[PathBuf],
    images: bool,
    tx: &mpsc::UnboundedSender<AgentEvent>,
    commands: &Arc<Mutex<Vec<SlashCommand>>>,
    pending: &Pending,
    cancel: &mut mpsc::UnboundedReceiver<()>,
) -> anyhow::Result<()> {
    // The prompt goes out as blocks (text, then the files), which the
    // crate's text-only `send_prompt` cannot do. Taking the result in a
    // callback attached at once keeps the ordering it gives: every update
    // the agent sent before answering is queued before the answer lands.
    // The callback never fails, so an agent's error on the prompt ends the
    // turn with its own message instead of closing the connection.
    let (stop_tx, mut stop_rx) = oneshot::channel::<Result<StopReason, agent_client_protocol::Error>>();
    let request = PromptRequest::new(session.session_id().clone(), crate::attach::prompt_blocks(text, files, images));
    session
        .connection()
        .send_request_to(Agent, request)
        .on_receiving_result(async move |result| {
            let _ = stop_tx.send(result.map(|r| r.stop_reason));
            Ok(())
        })?;
    loop {
        // Read the next update, forward a cancel, or finish on the answer:
        // the agent answers a cancel with its stop reason like any other end.
        let update = tokio::select! {
            biased;
            update = session.read_update() => update?,
            stop = &mut stop_rx => {
                // Whatever is already queued belongs to this turn.
                while let Some(Ok(update)) = futures::FutureExt::now_or_never(session.read_update()) {
                    forward(update, tx, commands, pending).await?;
                }
                match stop {
                    Ok(Ok(reason)) => {
                        let _ = tx.send(AgentEvent::Finished { stop_reason: format!("{reason:?}") });
                    }
                    Ok(Err(err)) => {
                        let detail = err.data.as_ref().map(|d| format!(" ({d})")).unwrap_or_default();
                        let _ = tx.send(AgentEvent::Failed { error: format!("{}{detail}", err.message) });
                    }
                    Err(_) => anyhow::bail!("the agent connection closed during the turn"),
                }
                return Ok(());
            }
            Some(()) = cancel.recv() => {
                tracing::info!("cancelling the turn");
                let id = session.session_id().clone();
                if let Err(err) = session.connection().send_notification_to(Agent, CancelNotification::new(id)) {
                    tracing::warn!(%err, "could not send session/cancel");
                }
                continue;
            }
        };
        forward(update, tx, commands, pending).await?;
    }
}

/// Turn one session message into session events.
async fn forward(
    update: SessionMessage,
    tx: &mpsc::UnboundedSender<AgentEvent>,
    commands: &Arc<Mutex<Vec<SlashCommand>>>,
    pending: &Pending,
) -> anyhow::Result<()> {
    if let SessionMessage::SessionMessage(dispatch) = update {
        let tx = tx.clone();
        let tx_ask = tx.clone();
        let commands = commands.clone();
        let pending = pending.clone();
        MatchDispatch::new(dispatch)
            .if_notification(async move |notif: SessionNotification| {
                let events = translate(notif.update);
                remember_commands(&commands, &events);
                for ev in events {
                    let _ = tx.send(ev);
                }
                Ok(())
            })
            .await
            // The agent asks before acting: keep the question, say it was
            // asked, and let whoever decides answer it through the session.
            .if_request(async move |req: RequestPermissionRequest, responder: Responder<RequestPermissionResponse>| {
                let _ = tx_ask.send(hold_question(&pending, req, responder));
                Ok(())
            })
            .await
            .otherwise_ignore()?;
    }
    Ok(())
}
