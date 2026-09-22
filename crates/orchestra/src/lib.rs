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

/// One thing that happened inside a lane.
///
/// Serialized to the UI as `{"kind": "...", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            LaneEvent::Finished { .. } => "done",
            LaneEvent::Failed { .. } => "error",
        }
    }
}

/// An event tagged with the lane and run it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneEnvelope {
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
    fn events_serialize_tagged() {
        let json = serde_json::to_string(&LaneEvent::Message { text: "x".into() }).unwrap();
        assert_eq!(json, r#"{"kind":"message","text":"x"}"#);
    }
}
