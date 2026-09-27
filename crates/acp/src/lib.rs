//! ACP client: spawns an agent subprocess, runs one prompt, streams
//! [`AgentEvent`]s back.
//!
//! This is the only place that knows about the wire protocol. Everything above
//! it sees session events and nothing else, which is what lets a non-ACP backend
//! (a PTY-driven CLI) be added later behind the same channel.

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_client_protocol::{
    schema::{
        v1::{
            AuthMethod, AuthenticateRequest, InitializeRequest, NewSessionRequest, NewSessionResponse,
            SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
            SessionConfigSelectOptions,
        },
        ProtocolVersion,
    },
    Client, ErrorCode,
};
use orchestra_core::AgentEvent;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

mod attach;
mod process;
mod session;
pub use attach::{file_uri, mime_of};
pub use process::{spawn as spawn_agent, AgentProcess};
pub use session::{AgentSession, McpHttp, SessionOptions};

/// How to launch an agent subprocess.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// Executable to run.
    pub program: String,
    /// Arguments passed to it.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// The environment's names but none of its values: a spec is printed in a
/// log line and by everything holding one ([`PromptSpec`]), and an agent's
/// API key or token is exactly what [`AgentSpec::env`] is for.
///
/// Destructured so that a field added to `AgentSpec` stops here with a
/// compile error, rather than being left out of every log line by accident
/// or printed in full by a derive somebody reached for.
impl std::fmt::Debug for AgentSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { program, args, env } = self;
        f.debug_struct("AgentSpec")
            .field("program", program)
            .field("args", args)
            .field("env", &env.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>())
            .finish()
    }
}

/// The maintained Claude Code ACP adapter.
///
/// This is the same adapter Zed drives its Claude Code integration with, which
/// is why that integration feels native: the protocol carries diffs, terminal
/// output, plans, slash commands and mode switching, not just text.
///
/// Note the package name: `@zed-industries/claude-code-acp` was renamed and
/// its last release (0.16.2, Feb 2026) is stale. Pinning that one silently
/// freezes the session on a months-old agent SDK.
pub const CLAUDE_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp@0.79.0";

/// Where the locally installed Claude adapter's entry point lives, relative to
/// a directory that has a `node_modules`.
pub const CLAUDE_ADAPTER_SCRIPT: &str =
    "node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";

/// Overrides Claude adapter discovery with an explicit path to the entry script.
const ADAPTER_ENV: &str = "DIVIXI_ACP_ADAPTER";

impl AgentSpec {
    /// Run a JavaScript entry script directly under `node`.
    pub fn node_script(script: impl AsRef<Path>) -> Self {
        Self {
            program: "node".to_string(),
            args: vec![script.as_ref().to_string_lossy().into_owned()],
            env: Vec::new(),
        }
    }

    /// Run an npm package through `npx`. Slow (ten seconds or more per
    /// launch), so callers should prefer a local install when one exists.
    ///
    /// On Windows `npx` is a shim, so it goes through `cmd /C`.
    pub fn npx(package: &str, args: &[&str]) -> Self {
        let (program, mut argv) = if cfg!(windows) {
            ("cmd".to_string(), vec!["/C".to_string(), "npx".to_string()])
        } else {
            ("npx".to_string(), Vec::new())
        };
        argv.extend(["-y".to_string(), package.to_string()]);
        argv.extend(args.iter().map(|s| s.to_string()));
        Self {
            program,
            args: argv,
            env: Vec::new(),
        }
    }

    /// Run a native executable.
    pub fn binary(program: impl AsRef<Path>, args: &[&str]) -> Self {
        Self {
            program: program.as_ref().to_string_lossy().into_owned(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
        }
    }

    /// The Claude Code ACP adapter.
    ///
    /// Claude Code has no native ACP mode as of CLI 2.1.x, so the adapter
    /// bridges it. A local install (`npm i -D @agentclientprotocol/claude-agent-acp`)
    /// is preferred and run directly under `node`. Going through `npx` with
    /// nothing installed costs ten seconds per session on a warm cache and twenty
    /// on a cold one, all of it before `initialize`. Without a local install
    /// this falls back to `npx`.
    pub fn claude_code() -> Self {
        if let Some(explicit) = std::env::var_os(ADAPTER_ENV) {
            let path = PathBuf::from(explicit);
            if path.is_file() {
                tracing::info!(script = %path.display(), "using adapter from {ADAPTER_ENV}");
                return Self::node_script(path);
            }
            tracing::warn!(path = %path.display(), "{ADAPTER_ENV} is set but is not a file");
        }
        if let Some(script) = find_local_script(CLAUDE_ADAPTER_SCRIPT) {
            tracing::info!(script = %script.display(), "using locally installed adapter");
            return Self::node_script(script);
        }
        tracing::info!(package = CLAUDE_ADAPTER, "no local adapter install; falling back to npx");
        Self::npx(CLAUDE_ADAPTER, &[])
    }
}

/// Locate a script inside a locally installed npm package.
///
/// `relative` is a path like `node_modules/<pkg>/dist/index.js`. The search
/// walks up from the executable's directory looking for a directory that
/// contains it, which covers `cargo run`, the examples and the Tauri dev
/// binary (all under the repo's `target/`).
pub fn find_local_script(relative: &str) -> Option<PathBuf> {
    // Only beside the executable: a development build (under the repo's
    // target/) walks up to the repo; a release build looks in its own
    // folder and the one above, never in whatever folder it was started
    // from or at a drive's root, where anyone could have put a script.
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let reach = if cfg!(debug_assertions) { usize::MAX } else { 2 };
    exe_dir
        .ancestors()
        .take(reach)
        .map(|dir| dir.join(relative))
        .find(|candidate| candidate.is_file())
}

/// Everything needed to execute one run in one session.
#[derive(Debug, Clone)]
pub struct PromptSpec {
    /// Which agent to launch.
    pub agent: AgentSpec,
    /// Working directory handed to the agent's session.
    pub cwd: PathBuf,
    /// The task text.
    pub prompt: String,
    /// Session mode to request before prompting, e.g. `bypassPermissions`.
    ///
    /// Orchestra surfaces deliberate escalations, not tool-approval prompts,
    /// so sessions run without per-tool permission round-trips. `None` picks the
    /// most autonomous mode the agent advertises.
    pub mode: Option<String>,
    /// Session options to set before prompting: `(option id, value id)`, as
    /// the agent advertised them (see [`ConfigOptionInfo`]). The model
    /// selector goes here.
    pub config: Vec<(String, String)>,
}

/// Environment variables that mark "you are inside a Claude Code session".
///
/// A session's agent must not inherit them. Claude Code refuses to start a nested
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

/// "…" plus the agent's last stderr lines, for error messages.
fn with_stderr(msg: String, process: &AgentProcess) -> String {
    let tail = process.stderr_tail();
    if tail.is_empty() {
        msg
    } else {
        format!("{msg}\nagent stderr:\n  {}", tail.join("\n  "))
    }
}

/// One way an agent lets the user log in, as advertised in `initialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// For terminal methods: the command line to run, joined with spaces.
    pub terminal_command: Option<String>,
}

/// What `session/new` said during a probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionProbe {
    /// A session was created: the agent is ready to run.
    Ok,
    /// The agent answered `auth_required`: installed, not logged in.
    ///
    /// `detail` is whatever the agent attached to the error, which is often
    /// the most useful text available (Antigravity, for one, lists the
    /// accepted auth methods and a settings file there).
    AuthRequired { detail: Option<String> },
    /// Some other failure.
    Failed { error: String, detail: Option<String> },
}

impl SessionProbe {
    fn from_error(err: &agent_client_protocol::Error) -> Self {
        let detail = err.data.as_ref().map(|d| {
            d.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| d.to_string())
        });
        // The code is authoritative, but an agent that wraps the error loses
        // it and keeps only the standard message, so accept that too.
        if err.code == ErrorCode::AuthRequired || err.message.starts_with("Authentication required") {
            SessionProbe::AuthRequired { detail }
        } else {
            SessionProbe::Failed {
                error: err.message.clone(),
                detail,
            }
        }
    }
}

/// One value a select-type session option can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChoice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Group name when the agent groups its choices (e.g. by provider).
    pub group: Option<String>,
}

/// A session option the agent exposes (`session/new` → `configOptions`):
/// the model selector, thought level, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigOptionInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `model`, `mode`, `model_config`, `thought_level`, or the agent's own word.
    pub category: String,
    /// Current value id (select) or `true`/`false` (boolean).
    pub current: String,
    /// Empty for boolean options.
    pub choices: Vec<ConfigChoice>,
}

/// A session mode the agent offers (`session/new` → `modes`): how much it
/// asks before acting. Ids are the agent's own (`bypassPermissions`,
/// `agent-full-access`, `yolo`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// What a probe learned about an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub protocol: String,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub load_session: bool,
    /// Whether the agent accepts HTTP MCP servers in `session/new`. The
    /// conductor's tools arrive that way.
    #[serde(default)]
    pub mcp_http: bool,
    pub auth_methods: Vec<AuthMethodInfo>,
    pub session: SessionProbe,
    /// Session options the agent offered, when a session was created.
    #[serde(default)]
    pub config_options: Vec<ConfigOptionInfo>,
    /// Session modes the agent offered, when a session was created.
    #[serde(default)]
    pub modes: Vec<ModeInfo>,
    /// The mode a fresh session starts in, when the agent has modes.
    #[serde(default)]
    pub default_mode: Option<String>,
    /// The mode Orchestra picks when none is chosen: the most autonomous
    /// one offered (see [`pick_autonomous_mode`]), else the default.
    #[serde(default)]
    pub autonomous_mode: Option<String>,
}

/// What `session/new` said about the session: its options and modes.
type SessionShape = (Vec<ConfigOptionInfo>, Vec<ModeInfo>, Option<String>);

fn session_shape_of(resp: &NewSessionResponse) -> SessionShape {
    let options = config_options_of(resp.config_options.as_ref());
    let modes = resp
        .modes
        .as_ref()
        .map(|m| {
            m.available_modes
                .iter()
                .map(|x| ModeInfo {
                    id: x.id.to_string(),
                    name: x.name.clone(),
                    description: x.description.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let current = resp.modes.as_ref().map(|m| m.current_mode_id.to_string());
    (options, modes, current)
}

fn config_options_of(options: Option<&Vec<SessionConfigOption>>) -> Vec<ConfigOptionInfo> {
    let Some(options) = options else { return Vec::new() };
    options
        .iter()
        .map(|o| {
            let category = match &o.category {
                Some(SessionConfigOptionCategory::Mode) => "mode".to_string(),
                Some(SessionConfigOptionCategory::Model) => "model".to_string(),
                Some(SessionConfigOptionCategory::ModelConfig) => "model_config".to_string(),
                Some(SessionConfigOptionCategory::ThoughtLevel) => "thought_level".to_string(),
                Some(SessionConfigOptionCategory::Other(s)) => s.clone(),
                Some(_) | None => "other".to_string(),
            };
            let (current, choices) = match &o.kind {
                SessionConfigKind::Select(sel) => {
                    let flat: Vec<ConfigChoice> = match &sel.options {
                        SessionConfigSelectOptions::Ungrouped(list) => list
                            .iter()
                            .map(|c| ConfigChoice {
                                id: c.value.to_string(),
                                name: c.name.clone(),
                                description: c.description.clone(),
                                group: None,
                            })
                            .collect(),
                        SessionConfigSelectOptions::Grouped(groups) => groups
                            .iter()
                            .flat_map(|g| {
                                g.options.iter().map(move |c| ConfigChoice {
                                    id: c.value.to_string(),
                                    name: c.name.clone(),
                                    description: c.description.clone(),
                                    group: Some(g.name.clone()),
                                })
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    (sel.current_value.to_string(), flat)
                }
                SessionConfigKind::Boolean(b) => (b.current_value.to_string(), Vec::new()),
                _ => (String::new(), Vec::new()),
            };
            ConfigOptionInfo {
                id: o.id.to_string(),
                name: o.name.clone(),
                description: o.description.clone(),
                category,
                current,
                choices,
            }
        })
        .collect()
}

/// Launch an agent, run the `initialize` handshake, try to open a session,
/// and shut it down again.
///
/// This is the protocol-native way to detect an agent: `initialize` reports
/// what it is and how to log in, and `session/new` fails with
/// `auth_required` when nobody has. Nothing is prompted. The whole probe is
/// bounded by `timeout`; an agent that hangs is reported as a failure, not
/// waited on.
pub async fn probe(agent: &AgentSpec, cwd: &Path, timeout: Duration) -> anyhow::Result<ProbeReport> {
    probe_inner(agent, cwd, timeout, None).await
}

/// Like [`probe`], but when `session/new` asks for authentication, send
/// `authenticate` with `method_id` and try the session once more.
///
/// The agent owns the login flow. Depending on the method it may reuse
/// cached credentials silently or open a browser; either way the result is
/// visible in the returned report's `session`.
pub async fn authenticate(
    agent: &AgentSpec,
    cwd: &Path,
    method_id: &str,
    timeout: Duration,
) -> anyhow::Result<ProbeReport> {
    probe_inner(agent, cwd, timeout, Some(method_id.to_string())).await
}

async fn probe_inner(
    agent: &AgentSpec,
    cwd: &Path,
    timeout: Duration,
    auth: Option<String>,
) -> anyhow::Result<ProbeReport> {
    let (process, transport) = process::spawn(agent)?;
    let cwd = cwd.to_path_buf();

    let probe = Client
        .builder()
        .name("orchestra-probe")
        .connect_with(transport, async move |cx| {
            let init = cx
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let auth_methods = init
                .auth_methods
                .iter()
                .map(|m| AuthMethodInfo {
                    id: m.id().to_string(),
                    name: m.name().to_string(),
                    description: m.description().map(str::to_owned),
                    terminal_command: match m {
                        AuthMethod::Terminal(t) => Some(t.args.join(" ")),
                        _ => None,
                    },
                })
                .collect();

            // The raw request, not `build_session`: that one spawns session
            // runners whose failure tears the whole connection down, which
            // would hide the error code we are here to read.
            let new_session = || async {
                match cx
                    .send_request(NewSessionRequest::new(cwd.clone()))
                    .block_task()
                    .await
                {
                    Ok(resp) => (SessionProbe::Ok, session_shape_of(&resp)),
                    Err(err) => (SessionProbe::from_error(&err), SessionShape::default()),
                }
            };

            let (mut session, mut shape) = new_session().await;
            if let (SessionProbe::AuthRequired { .. }, Some(method)) = (&session, auth.clone()) {
                tracing::info!(method, "session requires auth; authenticating");
                match cx
                    .send_request(AuthenticateRequest::new(method.clone()))
                    .block_task()
                    .await
                {
                    Ok(_) => (session, shape) = new_session().await,
                    Err(err) => {
                        session = SessionProbe::Failed {
                            error: format!("authenticate({method}) failed: {}", err.message),
                            detail: err.data.as_ref().map(|d| d.to_string()),
                        }
                    }
                }
            }

            let (config_options, modes, default_mode) = shape;
            let pairs: Vec<(String, String)> = modes.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
            let autonomous_mode = pick_autonomous_mode(&pairs).or_else(|| default_mode.clone());
            Ok(ProbeReport {
                protocol: format!("{:?}", init.protocol_version),
                agent_name: init.agent_info.as_ref().map(|i| i.name.clone()),
                agent_version: init.agent_info.as_ref().map(|i| i.version.clone()),
                load_session: init.agent_capabilities.load_session,
                mcp_http: init.agent_capabilities.mcp_capabilities.http,
                auth_methods,
                session,
                config_options,
                modes,
                default_mode,
                autonomous_mode,
            })
        });

    let outcome = tokio::time::timeout(timeout, probe).await;
    let result = match outcome {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(err)) => Err(anyhow::anyhow!(with_stderr(format!("acp probe failed: {err}"), &process))),
        Err(_) => Err(anyhow::anyhow!(with_stderr(
            format!("acp probe timed out after {}s", timeout.as_secs()),
            &process
        ))),
    };
    process.shutdown().await;
    result
}

/// Run one prompt to completion, emitting events as they arrive.
///
/// Returns once the agent reports a stop reason or the connection fails. The
/// subprocess is torn down when this future resolves.
pub async fn run_prompt(spec: PromptSpec, tx: UnboundedSender<AgentEvent>) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    tracing::info!(
        program = %spec.agent.program,
        cwd = %spec.cwd.display(),
        prompt_chars = spec.prompt.chars().count(),
        "one-shot run starting"
    );
    let result = run_prompt_inner(spec, tx).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match &result {
        Ok(()) => tracing::info!(elapsed_ms, "one-shot run finished"),
        Err(err) => tracing::warn!(elapsed_ms, %err, "one-shot run failed"),
    }
    result
}

async fn run_prompt_inner(spec: PromptSpec, tx: UnboundedSender<AgentEvent>) -> anyhow::Result<()> {
    let session = AgentSession::open(
        &spec.agent,
        SessionOptions {
            cwd: spec.cwd.clone(),
            mode: spec.mode.clone(),
            config: spec.config.clone(),
            mcp_servers: Vec::new(),
            resume: None,
            restricted: false,
        },
    )
    .await?;
    let _ = tx.send(AgentEvent::Connected {
        protocol: "ProtocolVersion(1)".to_string(),
        load_session: false,
    });
    let _ = tx.send(AgentEvent::Started {
        session_id: session.session_id().to_string(),
        cwd: spec.cwd.display().to_string(),
    });
    let result = session.prompt(spec.prompt.clone(), tx).await;
    session.close().await;
    result
}

/// Mode ids that mean "do not stop to ask about tools", most autonomous
/// first, as observed per agent:
///
/// - Claude Code: `bypassPermissions`
/// - Codex: `agent-full-access` (default `agent` still asks)
/// - Copilot: `https://agentclientprotocol.com/protocol/session-modes#autopilot`
/// - Antigravity: `yolo` (then `auto_edit`, `default`)
///
/// Ids are compared by their last `#`/`/` segment, case-insensitively,
/// and names are compared too.
const AUTONOMOUS_MODE_IDS: &[&str] = &[
    "bypasspermissions",
    "bypass-permissions",
    "yolo",
    "agent-full-access",
    "full-access",
    "full_access",
    "fullaccess",
    "autopilot",
    "auto-approve",
    "autoapprove",
    "auto_edit",
    "auto-edit",
    "autoedit",
    "acceptedits",
    "accept-edits",
];

/// Mode ids that ask before acting, most restrictive first.
const RESTRICTED_MODE_IDS: &[&str] = &["plan", "read-only", "readonly", "read_only", "ask", "manual", "default"];

fn mode_key(s: &str) -> String {
    s.rsplit(['#', '/']).next().unwrap_or(s).to_ascii_lowercase()
}

/// Whether a mode id is one that acts without asking.
pub fn is_autonomous_mode(id: &str) -> bool {
    AUTONOMOUS_MODE_IDS.contains(&mode_key(id).as_str())
}

/// The most restrictive mode among `(id, name)` pairs the agent offers.
pub fn pick_restricted_mode(available: &[(String, String)]) -> Option<String> {
    RESTRICTED_MODE_IDS.iter().find_map(|want| {
        available
            .iter()
            .find(|(id, name)| mode_key(id) == *want || mode_key(name) == *want)
            .map(|(id, _)| id.clone())
    })
}

/// The most autonomous mode among `(id, name)` pairs the agent offers.
///
/// Sessions run without per-tool approval by design: Orchestra surfaces the
/// escalations a session raises on purpose, not permission prompts. When an
/// agent offers nothing recognizable, its default mode is kept.
pub fn pick_autonomous_mode(available: &[(String, String)]) -> Option<String> {
    fn key(s: &str) -> String {
        s.rsplit(['#', '/']).next().unwrap_or(s).to_ascii_lowercase()
    }
    AUTONOMOUS_MODE_IDS.iter().find_map(|want| {
        available
            .iter()
            .find(|(id, name)| key(id) == *want || key(name) == *want)
            .map(|(id, _)| id.clone())
    })
}

use agent_client_protocol::schema::v1::{AvailableCommandInput, ContentBlock, SessionUpdate, ToolCallContent, ToolCallLocation};

/// Files a tool call touches: its locations and the paths of its diffs
/// (Codex names an edit's file only in the diff it sends).
fn touched<'a>(locations: impl IntoIterator<Item = &'a ToolCallLocation>, content: impl IntoIterator<Item = &'a ToolCallContent>) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let found = locations.into_iter().map(|l| &l.path).chain(content.into_iter().filter_map(|c| match c {
        ToolCallContent::Diff(d) => Some(&d.path),
        _ => None,
    }));
    for p in found {
        let p = p.display().to_string();
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}
use orchestra_core::SlashCommand;

/// Map one protocol update onto zero or more session events.
pub(crate) fn translate(update: SessionUpdate) -> Vec<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content)
            .map(|text| vec![AgentEvent::Message { text }])
            .unwrap_or_default(),
        SessionUpdate::AgentThoughtChunk(chunk) => text_of(&chunk.content)
            .map(|text| vec![AgentEvent::Thought { text }])
            .unwrap_or_default(),
        SessionUpdate::ToolCall(call) => vec![AgentEvent::ToolCall {
            id: format!("{}", call.tool_call_id),
            title: call.title.clone(),
            tool_kind: format!("{:?}", call.kind).to_lowercase(),
            status: format!("{:?}", call.status).to_lowercase(),
            paths: touched(&call.locations, &call.content),
        }],
        // Most tool-call updates carry content (terminal output, diffs), not a
        // status change. Emitting those as `status: unknown` buries the real
        // transitions, so only forward updates that move the state or name
        // the files the call touches.
        SessionUpdate::ToolCallUpdate(update) => {
            let status = update.fields.status.map(|s| format!("{s:?}").to_lowercase()).unwrap_or_default();
            let paths = touched(update.fields.locations.iter().flatten(), update.fields.content.iter().flatten());
            if status.is_empty() && paths.is_empty() {
                Vec::new()
            } else {
                vec![AgentEvent::ToolUpdate {
                    id: format!("{}", update.tool_call_id),
                    status,
                    paths,
                }]
            }
        }
        SessionUpdate::Plan(plan) => vec![AgentEvent::Plan {
            entries: plan.entries.iter().map(|e| e.content.clone()).collect(),
        }],
        SessionUpdate::UsageUpdate(usage) => vec![AgentEvent::Usage {
            raw: serde_json::to_value(&usage).unwrap_or(serde_json::Value::Null),
        }],
        SessionUpdate::AvailableCommandsUpdate(update) => vec![AgentEvent::Commands {
            commands: update
                .available_commands
                .iter()
                .map(|c| SlashCommand {
                    name: c.name.clone(),
                    description: c.description.clone(),
                    hint: c.input.as_ref().and_then(|i| match i {
                        AvailableCommandInput::Unstructured(u) => Some(u.hint.clone()),
                        _ => None,
                    }),
                })
                .collect(),
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

#[cfg(test)]
mod mode_tests {
    use super::pick_autonomous_mode;

    fn modes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(i, n)| (i.to_string(), n.to_string())).collect()
    }

    #[test]
    fn picks_each_agents_autonomous_mode() {
        assert_eq!(
            pick_autonomous_mode(&modes(&[("default", "Default"), ("bypassPermissions", "Bypass")])),
            Some("bypassPermissions".into())
        );
        assert_eq!(
            pick_autonomous_mode(&modes(&[
                ("read-only", "Ask for approval"),
                ("agent", "Approve for me"),
                ("agent-full-access", "Full access")
            ])),
            Some("agent-full-access".into())
        );
        let cp = "https://agentclientprotocol.com/protocol/session-modes#autopilot";
        assert_eq!(
            pick_autonomous_mode(&modes(&[
                ("https://agentclientprotocol.com/protocol/session-modes#agent", "Agent"),
                (cp, "Autopilot")
            ])),
            Some(cp.into())
        );
        assert_eq!(
            pick_autonomous_mode(&modes(&[("default", "Default"), ("auto_edit", "Auto Edit"), ("yolo", "YOLO")])),
            Some("yolo".into())
        );
        assert_eq!(pick_autonomous_mode(&modes(&[("plan", "Plan")])), None);
    }
}

#[cfg(test)]
mod spec_tests {
    use super::AgentSpec;

    /// The log keeps a file now, so a spec's environment must not print its
    /// values: that is where an agent's API key or token would be.
    #[test]
    fn a_spec_prints_its_environments_names_only() {
        let mut spec = AgentSpec::binary("claude", &["--acp"]);
        spec.env.push(("ANTHROPIC_API_KEY".into(), "sk-secret".into()));
        let shown = format!("{spec:?}");
        assert!(shown.contains("ANTHROPIC_API_KEY"), "the name says enough to debug with: {shown}");
        assert!(!shown.contains("sk-secret"), "the value never goes in a log: {shown}");
        assert!(shown.contains("claude") && shown.contains("--acp"), "{shown}");
    }
}
