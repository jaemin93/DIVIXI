//! A long-lived agent session: one process, one ACP session, many prompts.
//!
//! [`run_lane`](crate::run_lane) is the one-shot form: open, prompt, close.
//! The conductor needs the other form, because its whole point is context
//! that survives across turns, and so do lanes once the conductor can ask
//! them follow-up questions. [`AgentSession`] owns the process and the
//! protocol task; each [`AgentSession::prompt`] runs one turn and streams
//! that turn's [`LaneEvent`]s to the channel given for it, ending with
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
            NewSessionRequest, SessionConfigOptionValue, SessionNotification,
            SetSessionConfigOptionRequest, SetSessionModeRequest,
        },
        ProtocolVersion,
    },
    util::MatchDispatch,
    ActiveSession, Agent, Client, SessionMessage,
};
use orchestra_core::LaneEvent;
use tokio::sync::{mpsc, oneshot};

use crate::{pick_autonomous_mode, process, translate, AgentSpec};

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
}

/// How long to wait for more replayed history after `session/load`.
const REPLAY_QUIET: Duration = Duration::from_millis(400);

/// One turn's request: the text and where its events go.
struct Turn {
    text: String,
    tx: mpsc::UnboundedSender<LaneEvent>,
    done: oneshot::Sender<anyhow::Result<()>>,
}

/// What the session task reports once the session exists.
struct Ready {
    session_id: String,
    resumed: bool,
}

/// A live session. Dropping it ends the session and kills the agent.
pub struct AgentSession {
    turns: mpsc::Sender<Turn>,
    session_id: String,
    resumed: bool,
    task: tokio::task::JoinHandle<()>,
}

type ReadyTx = oneshot::Sender<anyhow::Result<Ready>>;

impl AgentSession {
    /// Launch the agent, initialize, create (or load) a session and apply
    /// mode and options. Resolves once the session exists, so a failure to
    /// start (auth, bad adapter) is reported here, not on the first prompt.
    pub async fn open(agent: &AgentSpec, opts: SessionOptions) -> anyhow::Result<Self> {
        let (process, transport) = process::spawn(agent)?;
        let (turn_tx, mut turn_rx) = mpsc::channel::<Turn>(1);
        let (ready_tx, ready_rx) = oneshot::channel::<anyhow::Result<Ready>>();
        let opts_for_task = opts.clone();

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
                    let mut carried: Vec<LaneEvent> = Vec::new();
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
                                tracing::warn!(session = %id, %err, "session/load failed; opening a fresh session");
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
                    if let Some(wanted) = opts_for_task.mode.clone().or_else(|| pick_autonomous_mode(&available)) {
                        if current.as_deref() != Some(wanted.as_str()) {
                            tracing::info!(mode = %wanted, "setting session mode");
                            session
                                .connection()
                                .send_request_to(Agent, SetSessionModeRequest::new(session_id.clone(), wanted))
                                .block_task()
                                .await?;
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
                            tracing::warn!(option, value, %err, "agent rejected session option");
                        }
                    }

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
                        let result = run_turn(&mut session, &turn.text, &turn.tx).await;
                        if let Err(err) = &result {
                            let _ = turn.tx.send(LaneEvent::Failed { error: err.to_string() });
                        }
                        let fatal = result.is_err();
                        let _ = turn.done.send(result);
                        if fatal {
                            // The connection is in an unknown state; end the session.
                            return Err(agent_client_protocol::Error::internal_error());
                        }
                    }
                    Ok(())
                })
                .await;

            if let Err(err) = &outcome {
                tracing::warn!(%err, "agent session ended with error");
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
                session_id: ready.session_id,
                resumed: ready.resumed,
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

    /// Run one turn. Events stream to `tx` and end with `Finished` or
    /// `Failed`; the future resolves when the turn is over.
    pub async fn prompt(&self, text: String, tx: mpsc::UnboundedSender<LaneEvent>) -> anyhow::Result<()> {
        let (done_tx, done_rx) = oneshot::channel();
        self.turns
            .send(Turn { text, tx, done: done_tx })
            .await
            .map_err(|_| anyhow::anyhow!("agent session is closed"))?;
        done_rx
            .await
            .map_err(|_| anyhow::anyhow!("agent session ended during the turn"))?
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
async fn drain_replay(session: &mut ActiveSession<'_, Agent>) -> (usize, Vec<LaneEvent>) {
    let mut n = 0;
    let mut kept = Vec::new();
    loop {
        match tokio::time::timeout(REPLAY_QUIET, session.read_update()).await {
            Ok(Ok(SessionMessage::SessionMessage(dispatch))) => {
                n += 1;
                let _ = MatchDispatch::new(dispatch)
                    .if_notification(async |notif: SessionNotification| {
                        for ev in translate(notif.update) {
                            if matches!(ev, LaneEvent::Commands { .. }) {
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

async fn run_turn(
    session: &mut ActiveSession<'_, Agent>,
    text: &str,
    tx: &mpsc::UnboundedSender<LaneEvent>,
) -> anyhow::Result<()> {
    session.send_prompt(text)?;
    loop {
        match session.read_update().await? {
            SessionMessage::SessionMessage(dispatch) => {
                let tx = tx.clone();
                MatchDispatch::new(dispatch)
                    .if_notification(async move |notif: SessionNotification| {
                        for ev in translate(notif.update) {
                            let _ = tx.send(ev);
                        }
                        Ok(())
                    })
                    .await
                    .otherwise_ignore()?;
            }
            SessionMessage::StopReason(reason) => {
                let _ = tx.send(LaneEvent::Finished {
                    stop_reason: format!("{reason:?}"),
                });
                return Ok(());
            }
            _ => {}
        }
    }
}
