//! ACP client: spawns an agent subprocess, runs one prompt, streams
//! [`LaneEvent`]s back.
//!
//! This is the only place that knows about the wire protocol. Everything above
//! it sees lane events and nothing else, which is what lets a non-ACP backend
//! (a PTY-driven CLI) be added later behind the same channel.

use std::path::PathBuf;

use agent_client_protocol::{
    schema::{
        v1::{InitializeRequest, SetSessionModeRequest},
        ProtocolVersion,
    },
    util::MatchDispatch,
    AcpAgent, AcpAgentConfig, Agent, Client, SessionMessage,
};
use orchestra_core::LaneEvent;
use tokio::sync::mpsc::UnboundedSender;

/// How to launch an agent subprocess.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    /// Executable to run.
    pub program: String,
    /// Arguments passed to it.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// The maintained Claude Code ACP adapter.
///
/// This is the same adapter Zed drives its Claude Code integration with, which
/// is why that integration feels native: the protocol carries diffs, terminal
/// output, plans, slash commands and mode switching, not just text.
///
/// Note the package name: `@zed-industries/claude-code-acp` was renamed and
/// its last release (0.16.2, Feb 2026) is stale. Pinning that one silently
/// freezes the lane on a months-old agent SDK.
const CLAUDE_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp@0.79.0";

impl AgentSpec {
    /// The Claude Code ACP adapter, run through `npx`.
    ///
    /// Claude Code has no native ACP mode as of CLI 2.1.x, so the adapter
    /// bridges it. On Windows `npx` is a shim, so it goes through `cmd /C`.
    pub fn claude_code() -> Self {
        let (program, mut args) = if cfg!(windows) {
            ("cmd".to_string(), vec!["/C".to_string(), "npx".to_string()])
        } else {
            ("npx".to_string(), Vec::new())
        };
        args.extend(["-y".to_string(), CLAUDE_ADAPTER.to_string()]);
        Self {
            program,
            args,
            env: Vec::new(),
        }
    }
}

/// Everything needed to execute one run in one lane.
#[derive(Debug, Clone)]
pub struct LaneSpec {
    /// Which agent to launch.
    pub agent: AgentSpec,
    /// Working directory handed to the agent's session.
    pub cwd: PathBuf,
    /// The task text.
    pub prompt: String,
    /// Session mode to request before prompting, e.g. `bypassPermissions`.
    ///
    /// Orchestra surfaces deliberate escalations, not tool-approval prompts,
    /// so lanes run without per-tool permission round-trips. `None` leaves the
    /// agent's default mode alone.
    pub mode: Option<String>,
}

/// Environment variables that mark "you are inside a Claude Code session".
///
/// A lane's agent must not inherit them. Claude Code refuses to start a nested
/// session, so an agent spawned from inside one dies during `session/new` with
/// `Query closed before response received` — an error that names nothing
/// useful. Orchestra is frequently launched from exactly such a terminal.
const INHERITED_SESSION_VARS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// Drop inherited session markers from *this* process.
///
/// Call once, at the very start of `main`, before any thread or async runtime
/// exists: the agent SDK reads these from the spawned child's inherited
/// environment, and [`AcpAgentConfig`] can only add variables, never remove
/// them, so the removal has to happen here.
///
/// # Safety
///
/// Mutating the environment races with concurrent reads in other threads.
/// Calling this before the runtime starts keeps it single-threaded.
pub fn scrub_inherited_session_env() {
    for key in INHERITED_SESSION_VARS {
        if std::env::var_os(key).is_some() {
            tracing::debug!(key, "removing inherited Claude Code session variable");
            std::env::remove_var(key);
        }
    }
}

/// Run one prompt to completion, emitting events as they arrive.
///
/// Returns once the agent reports a stop reason or the connection fails. The
/// subprocess is torn down when this future resolves.
pub async fn run_lane(spec: LaneSpec, tx: UnboundedSender<LaneEvent>) -> anyhow::Result<()> {
    let mut cfg = AcpAgentConfig::new(&spec.agent.program).args(spec.agent.args.clone());
    for (k, v) in &spec.agent.env {
        cfg = cfg.env(k, v);
    }
    let transport = AcpAgent::new(cfg);

    let cwd = spec.cwd.clone();
    let prompt = spec.prompt.clone();
    let mode = spec.mode.clone();

    let outcome = Client
        .builder()
        .name("orchestra")
        .connect_with(transport, async move |cx| {
            let init = cx
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let _ = tx.send(LaneEvent::Connected {
                protocol: format!("{:?}", init.protocol_version),
                load_session: init.agent_capabilities.load_session,
            });

            let cwd_label = cwd.display().to_string();
            cx.build_session(&cwd)
                .block_task()
                .run_until(async move |mut session| {
                    let session_id = session.session_id().clone();
                    let _ = tx.send(LaneEvent::Started {
                        session_id: format!("{session_id}"),
                        cwd: cwd_label,
                    });

                    if let Some(mode) = mode {
                        session
                            .connection()
                            .send_request_to(Agent, SetSessionModeRequest::new(session_id, mode))
                            .block_task()
                            .await?;
                    }

                    session.send_prompt(&prompt)?;

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
                                break;
                            }
                            // `SessionMessage` is `#[non_exhaustive]`.
                            _ => {}
                        }
                    }
                    Ok(())
                })
                .await
        })
        .await;

    if let Err(err) = outcome {
        anyhow::bail!("acp connection failed: {err}");
    }
    Ok(())
}

use agent_client_protocol::schema::v1::{ContentBlock, SessionNotification, SessionUpdate};

/// Map one protocol update onto zero or more lane events.
fn translate(update: SessionUpdate) -> Vec<LaneEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content)
            .map(|text| vec![LaneEvent::Message { text }])
            .unwrap_or_default(),
        SessionUpdate::AgentThoughtChunk(chunk) => text_of(&chunk.content)
            .map(|text| vec![LaneEvent::Thought { text }])
            .unwrap_or_default(),
        SessionUpdate::ToolCall(call) => vec![LaneEvent::ToolCall {
            id: format!("{}", call.tool_call_id),
            title: call.title.clone(),
            tool_kind: format!("{:?}", call.kind).to_lowercase(),
            status: format!("{:?}", call.status).to_lowercase(),
        }],
        // Most tool-call updates carry content (terminal output, diffs), not a
        // status change. Emitting those as `status: unknown` buries the real
        // transitions, so only forward updates that actually move the state.
        SessionUpdate::ToolCallUpdate(update) => update
            .fields
            .status
            .map(|status| {
                vec![LaneEvent::ToolUpdate {
                    id: format!("{}", update.tool_call_id),
                    status: format!("{status:?}").to_lowercase(),
                }]
            })
            .unwrap_or_default(),
        SessionUpdate::Plan(plan) => vec![LaneEvent::Plan {
            entries: plan.entries.iter().map(|e| e.content.clone()).collect(),
        }],
        SessionUpdate::UsageUpdate(usage) => vec![LaneEvent::Usage {
            raw: serde_json::to_value(&usage).unwrap_or(serde_json::Value::Null),
        }],
        _ => Vec::new(),
    }
}

/// Pull plain text out of a content block, ignoring images and resources.
fn text_of(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}
