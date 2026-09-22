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

/// Where the locally installed adapter's entry point lives, relative to a
/// directory that has a `node_modules`.
const CLAUDE_ADAPTER_SCRIPT: &str =
    "node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";

/// Overrides adapter discovery with an explicit path to the entry script.
const ADAPTER_ENV: &str = "ORCHESTRA_ACP_ADAPTER";

impl AgentSpec {
    /// The Claude Code ACP adapter.
    ///
    /// Claude Code has no native ACP mode as of CLI 2.1.x, so the adapter
    /// bridges it. A local install (`npm i -D @agentclientprotocol/claude-agent-acp`)
    /// is preferred and run directly under `node`. Going through `npx` with
    /// nothing installed costs ten seconds per lane on a warm cache and twenty
    /// on a cold one, all of it before `initialize`. Without a local install
    /// this falls back to `npx`, which on Windows is a shim and so goes
    /// through `cmd /C`.
    pub fn claude_code() -> Self {
        if let Some(script) = find_local_adapter() {
            tracing::info!(script = %script.display(), "using locally installed adapter");
            return Self {
                program: "node".to_string(),
                args: vec![script.to_string_lossy().into_owned()],
                env: Vec::new(),
            };
        }

        tracing::info!(package = CLAUDE_ADAPTER, "no local adapter install; falling back to npx");
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

/// Locate a locally installed adapter entry script.
///
/// Checks `ORCHESTRA_ACP_ADAPTER` first, then walks up from the working
/// directory and from the executable's directory looking for `node_modules`.
/// The walk covers both `cargo run` from the repo root and the Tauri dev
/// binary, which lives under `target/`.
fn find_local_adapter() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(ADAPTER_ENV) {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
        tracing::warn!(path = %path.display(), "{ADAPTER_ENV} is set but is not a file");
    }

    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        roots.push(dir);
    }
    roots
        .iter()
        .flat_map(|root| root.ancestors())
        .map(|dir| dir.join(CLAUDE_ADAPTER_SCRIPT))
        .find(|candidate| candidate.is_file())
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
    let started = std::time::Instant::now();
    tracing::info!(
        program = %spec.agent.program,
        cwd = %spec.cwd.display(),
        prompt_chars = spec.prompt.chars().count(),
        "lane run starting"
    );
    let result = run_lane_inner(spec, tx).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match &result {
        Ok(()) => tracing::info!(elapsed_ms, "lane run finished"),
        Err(err) => tracing::warn!(elapsed_ms, %err, "lane run failed"),
    }
    result
}

async fn run_lane_inner(spec: LaneSpec, tx: UnboundedSender<LaneEvent>) -> anyhow::Result<()> {
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
