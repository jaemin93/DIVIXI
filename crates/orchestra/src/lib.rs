//! Orchestra domain types.
//!
//! The membrane lives here: [`AgentEvent`] is everything a session emits, and
//! [`AgentEvent::above_membrane`] decides what is allowed to reach a Track's
//! timeline. Everything else stays in the session and is fetched on demand.

use serde::{Deserialize, Serialize};

/// Which session a run belongs to within its track: `conductor`, a
/// worker's name, or `artifact`. Durable across runs.
pub type SessionName = String;
/// Stable identifier for a single run inside a session.
pub type RunId = String;

/// Lifecycle of a single run, as projected from its events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The agent subprocess is starting; no session yet.
    Connecting,
    /// A session exists and the prompt is in flight.
    Running,
    /// The turn ended normally.
    Done,
    /// The run failed, or was interrupted by the app closing.
    Failed,
}

impl RunStatus {
    /// Whether the run can still change.
    pub fn is_live(self) -> bool {
        matches!(self, RunStatus::Connecting | RunStatus::Running)
    }

    /// The `snake_case` name serde uses, for storage keys and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Connecting => "connecting",
            RunStatus::Running => "running",
            RunStatus::Done => "done",
            RunStatus::Failed => "failed",
        }
    }

    /// Inverse of [`RunStatus::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "connecting" => RunStatus::Connecting,
            "running" => RunStatus::Running,
            "done" => RunStatus::Done,
            "failed" => RunStatus::Failed,
            _ => return None,
        })
    }
}

/// One thing that happened inside a session.
///
/// Serialized to the UI as `{"kind": "...", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The agent subprocess answered `initialize`.
    Connected {
        /// Protocol version the agent negotiated.
        protocol: String,
        /// Whether the agent can restore sessions (`session/load`).
        load_session: bool,
    },
    /// A session was created; the run is now live.
    Started {
        /// The agent's session id.
        session_id: String,
        /// Working directory the agent was given.
        cwd: String,
    },
    /// A chunk of the agent's visible response.
    Message {
        /// Text fragment.
        text: String,
    },
    /// A chunk of the agent's internal reasoning.
    Thought {
        /// Text fragment.
        text: String,
    },
    /// A tool call started.
    ToolCall {
        /// Agent-assigned tool call id.
        id: String,
        /// Human-readable title, e.g. the command or path.
        title: String,
        /// Tool category as reported by the agent (`read`, `execute`, …).
        ///
        /// Not named `kind`: that is the enum's serde tag.
        tool_kind: String,
        /// Lifecycle status.
        status: String,
    },
    /// A tool call changed status.
    ToolUpdate {
        /// Agent-assigned tool call id.
        id: String,
        /// New lifecycle status.
        status: String,
    },
    /// The agent published or revised its plan.
    Plan {
        /// One line per plan entry.
        entries: Vec<String>,
    },
    /// Token and cost accounting for the run so far.
    Usage {
        /// Raw usage payload from the agent.
        raw: serde_json::Value,
    },
    /// The slash commands the agent offers in this session (`/compact`,
    /// `/review`, …). Sent when a session opens and whenever the set
    /// changes; the composer completes them.
    Commands {
        commands: Vec<SlashCommand>,
    },
    /// The agent asks before it acts (`session/request_permission`): what
    /// it wants to do and the answers it offers. It waits until someone
    /// answers through the session (`AgentSession::answer_permission`).
    Permission {
        /// Divixi's id for this question, to answer it by.
        request: String,
        /// What the agent wants to do, e.g. the command.
        title: String,
        /// Tool category (`execute`, `edit`, …).
        tool_kind: String,
        /// The tool's input as the agent gave it, shortened.
        input: String,
        options: Vec<PermissionChoice>,
    },
    /// The turn ended.
    Finished {
        /// Why the turn stopped, e.g. `end_turn`.
        stop_reason: String,
    },
    /// The run could not continue.
    Failed {
        /// Human-readable failure.
        error: String,
    },
}

impl AgentEvent {
    /// Whether this event is allowed to cross into the Track timeline.
    ///
    /// Only lifecycle boundaries cross. Message chunks, thoughts, tool calls
    /// and plans stay in the session and are read through the inspector, which is
    /// the whole point of the membrane: the Track stays readable by a human.
    pub fn above_membrane(&self) -> bool {
        matches!(
            self,
            AgentEvent::Started { .. } | AgentEvent::Finished { .. } | AgentEvent::Failed { .. }
        )
    }

    /// The serde tag (`kind`) this variant serializes with.
    ///
    /// Stable storage key: the event store indexes on it, so it must not
    /// drift from the `#[serde(tag = "kind")]` names.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentEvent::Connected { .. } => "connected",
            AgentEvent::Started { .. } => "started",
            AgentEvent::Message { .. } => "message",
            AgentEvent::Thought { .. } => "thought",
            AgentEvent::ToolCall { .. } => "tool_call",
            AgentEvent::ToolUpdate { .. } => "tool_update",
            AgentEvent::Plan { .. } => "plan",
            AgentEvent::Usage { .. } => "usage",
            AgentEvent::Commands { .. } => "commands",
            AgentEvent::Permission { .. } => "permission",
            AgentEvent::Finished { .. } => "finished",
            AgentEvent::Failed { .. } => "failed",
        }
    }

    /// Whether this event ends the run.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentEvent::Finished { .. } | AgentEvent::Failed { .. })
    }

    /// Short mono label used by the inspector's transcript view.
    pub fn label(&self) -> &'static str {
        match self {
            AgentEvent::Connected { .. } => "connect",
            AgentEvent::Started { .. } => "session",
            AgentEvent::Message { .. } => "message",
            AgentEvent::Thought { .. } => "thought",
            AgentEvent::ToolCall { .. } => "tool",
            AgentEvent::ToolUpdate { .. } => "tool",
            AgentEvent::Plan { .. } => "plan",
            AgentEvent::Usage { .. } => "usage",
            AgentEvent::Commands { .. } => "commands",
            AgentEvent::Permission { .. } => "asks",
            AgentEvent::Finished { .. } => "done",
            AgentEvent::Failed { .. } => "error",
        }
    }
}

/// One answer an agent offers to a permission question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionChoice {
    /// The agent's id for it, sent back as the answer.
    pub id: String,
    /// What the agent calls it, e.g. "Allow".
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// A slash command the agent offers, as the composer completes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    /// Name without the slash, e.g. `compact`.
    pub name: String,
    pub description: String,
    /// What to type after the name, when the command takes input.
    pub hint: Option<String>,
}

/// An event tagged with the session and run it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvelope {
    /// Which track the session belongs to.
    pub track: String,
    /// Which session produced it.
    pub session: SessionName,
    /// Which run produced it.
    pub run: RunId,
    /// Milliseconds since the run started.
    pub at_ms: u64,
    /// The event itself.
    pub event: AgentEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lifecycle_crosses_the_membrane() {
        assert!(AgentEvent::Started {
            session_id: "s".into(),
            cwd: ".".into()
        }
        .above_membrane());
        assert!(AgentEvent::Finished {
            stop_reason: "end_turn".into()
        }
        .above_membrane());
        assert!(!AgentEvent::Message { text: "hi".into() }.above_membrane());
        assert!(!AgentEvent::ToolCall {
            id: "1".into(),
            title: "cargo test".into(),
            tool_kind: "execute".into(),
            status: "pending".into()
        }
        .above_membrane());
    }

    #[test]
    fn kind_matches_serde_tag() {
        let samples = [
            AgentEvent::Connected { protocol: "v1".into(), load_session: false },
            AgentEvent::Started { session_id: "s".into(), cwd: ".".into() },
            AgentEvent::Message { text: "m".into() },
            AgentEvent::Thought { text: "t".into() },
            AgentEvent::ToolCall {
                id: "1".into(),
                title: "ls".into(),
                tool_kind: "execute".into(),
                status: "pending".into(),
            },
            AgentEvent::ToolUpdate { id: "1".into(), status: "completed".into() },
            AgentEvent::Plan { entries: vec![] },
            AgentEvent::Usage { raw: serde_json::Value::Null },
            AgentEvent::Commands { commands: vec![] },
            AgentEvent::Permission {
                request: "p1".into(),
                title: "rm x".into(),
                tool_kind: "execute".into(),
                input: String::new(),
                options: vec![],
            },
            AgentEvent::Finished { stop_reason: "end_turn".into() },
            AgentEvent::Failed { error: "boom".into() },
        ];
        for ev in samples {
            let json = serde_json::to_value(&ev).unwrap();
            assert_eq!(json["kind"], ev.kind(), "{ev:?}");
        }
    }

    #[test]
    fn run_status_round_trips() {
        for s in [RunStatus::Connecting, RunStatus::Running, RunStatus::Done, RunStatus::Failed] {
            assert_eq!(RunStatus::parse(s.as_str()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
    }

    #[test]
    fn events_serialize_tagged() {
        let json = serde_json::to_string(&AgentEvent::Message { text: "x".into() }).unwrap();
        assert_eq!(json, r#"{"kind":"message","text":"x"}"#);
    }
}
