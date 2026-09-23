//! Orchestra domain types.
//!
//! The membrane lives here: [`LaneEvent`] is everything a lane emits, and
//! [`LaneEvent::above_membrane`] decides what is allowed to reach a Track's
//! timeline. Everything else stays in the lane and is fetched on demand.

use serde::{Deserialize, Serialize};

/// Stable identifier for a lane (durable across runs).
pub type LaneId = String;
/// Stable identifier for a single run inside a lane.
pub type RunId = String;

/// A lane's execution status, as shown on the lane strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneStatus {
    /// No run in flight.
    Idle,
    /// A run is in flight.
    Running,
    /// A run is blocked on a human decision.
    AwaitingDecision,
    /// The last run failed.
    Failed,
}

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

/// One thing that happened inside a lane.
///
/// Serialized to the UI as `{"kind": "...", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaneEvent {
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

impl LaneEvent {
    /// Whether this event is allowed to cross into the Track timeline.
    ///
    /// Only lifecycle boundaries cross. Message chunks, thoughts, tool calls
    /// and plans stay in the lane and are read through the inspector, which is
    /// the whole point of the membrane: the Track stays readable by a human.
    pub fn above_membrane(&self) -> bool {
        matches!(
            self,
            LaneEvent::Started { .. } | LaneEvent::Finished { .. } | LaneEvent::Failed { .. }
        )
    }

    /// The serde tag (`kind`) this variant serializes with.
    ///
    /// Stable storage key: the event store indexes on it, so it must not
    /// drift from the `#[serde(tag = "kind")]` names.
    pub fn kind(&self) -> &'static str {
        match self {
            LaneEvent::Connected { .. } => "connected",
            LaneEvent::Started { .. } => "started",
            LaneEvent::Message { .. } => "message",
            LaneEvent::Thought { .. } => "thought",
            LaneEvent::ToolCall { .. } => "tool_call",
            LaneEvent::ToolUpdate { .. } => "tool_update",
            LaneEvent::Plan { .. } => "plan",
            LaneEvent::Usage { .. } => "usage",
            LaneEvent::Commands { .. } => "commands",
            LaneEvent::Finished { .. } => "finished",
            LaneEvent::Failed { .. } => "failed",
        }
    }

    /// Whether this event ends the run.
    pub fn is_terminal(&self) -> bool {
        matches!(self, LaneEvent::Finished { .. } | LaneEvent::Failed { .. })
    }

    /// Short mono label used by the inspector's transcript view.
    pub fn label(&self) -> &'static str {
        match self {
            LaneEvent::Connected { .. } => "connect",
            LaneEvent::Started { .. } => "session",
            LaneEvent::Message { .. } => "message",
            LaneEvent::Thought { .. } => "thought",
            LaneEvent::ToolCall { .. } => "tool",
            LaneEvent::ToolUpdate { .. } => "tool",
            LaneEvent::Plan { .. } => "plan",
            LaneEvent::Usage { .. } => "usage",
            LaneEvent::Commands { .. } => "commands",
            LaneEvent::Finished { .. } => "done",
            LaneEvent::Failed { .. } => "error",
        }
    }
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

/// An event tagged with the lane and run it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneEnvelope {
    /// Which track the lane belongs to.
    pub track: String,
    /// Which lane produced it.
    pub lane: LaneId,
    /// Which run produced it.
    pub run: RunId,
    /// Milliseconds since the run started.
    pub at_ms: u64,
    /// The event itself.
    pub event: LaneEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lifecycle_crosses_the_membrane() {
        assert!(LaneEvent::Started {
            session_id: "s".into(),
            cwd: ".".into()
        }
        .above_membrane());
        assert!(LaneEvent::Finished {
            stop_reason: "end_turn".into()
        }
        .above_membrane());
        assert!(!LaneEvent::Message { text: "hi".into() }.above_membrane());
        assert!(!LaneEvent::ToolCall {
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
            LaneEvent::Connected { protocol: "v1".into(), load_session: false },
            LaneEvent::Started { session_id: "s".into(), cwd: ".".into() },
            LaneEvent::Message { text: "m".into() },
            LaneEvent::Thought { text: "t".into() },
            LaneEvent::ToolCall {
                id: "1".into(),
                title: "ls".into(),
                tool_kind: "execute".into(),
                status: "pending".into(),
            },
            LaneEvent::ToolUpdate { id: "1".into(), status: "completed".into() },
            LaneEvent::Plan { entries: vec![] },
            LaneEvent::Usage { raw: serde_json::Value::Null },
            LaneEvent::Commands { commands: vec![] },
            LaneEvent::Finished { stop_reason: "end_turn".into() },
            LaneEvent::Failed { error: "boom".into() },
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
        let json = serde_json::to_string(&LaneEvent::Message { text: "x".into() }).unwrap();
        assert_eq!(json, r#"{"kind":"message","text":"x"}"#);
    }
}
