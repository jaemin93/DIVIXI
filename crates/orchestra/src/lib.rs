//! Orchestra domain types.
//!
//! The membrane lives here: [`AgentEvent`] is everything a session emits, and
//! [`AgentEvent::above_membrane`] decides what is allowed to reach a Track's
//! timeline. Everything else stays in the session and is fetched on demand.

pub mod path;
pub mod report;

use serde::{Deserialize, Serialize};

/// Which session a run belongs to within its track: `conductor`, a
/// worker's name, or `artifact`. Durable across runs.
pub type SessionName = String;
/// Stable identifier for a single run inside a session.
pub type RunId = String;

/// Lifecycle of a single run, as projected from its events.
///
/// The four ways a run can end are kept apart because they call for
/// different things from whoever reads them. `Failed` is the agent's own
/// doing and is over. `Interrupted` is the app's: a reboot or the app
/// closing under a turn that was going fine, so the work may be half done
/// and is worth starting again with its memory. `TimedOut` is the app
/// giving up on a turn that stopped answering. `Detached` is the one case
/// where nothing has ended: the app closed, but the agent process it had
/// started is still alive and working on its own, so it must not be
/// started again until it is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The agent subprocess is starting; no session yet.
    Connecting,
    /// A session exists and the prompt is in flight.
    Running,
    /// The turn ended normally.
    Done,
    /// The run failed: the agent said so, or its session died.
    Failed,
    /// The app closed, or the machine rebooted, while the turn was live.
    Interrupted,
    /// The app ended the turn because the agent stopped answering.
    TimedOut,
    /// The app closed under the turn, but the agent process is still alive
    /// and cannot be reached; it ends as `Interrupted` once that process
    /// is gone.
    Detached,
}

impl RunStatus {
    /// Whether the run can still change.
    pub fn is_live(self) -> bool {
        matches!(self, RunStatus::Connecting | RunStatus::Running)
    }

    /// Whether the run ended without the agent finishing its turn, for a
    /// reason of the app's rather than the agent's: it may be started
    /// again with its memory.
    pub fn is_cut_short(self) -> bool {
        matches!(self, RunStatus::Interrupted | RunStatus::TimedOut)
    }

    /// The `snake_case` name serde uses, for storage keys and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Connecting => "connecting",
            RunStatus::Running => "running",
            RunStatus::Done => "done",
            RunStatus::Failed => "failed",
            RunStatus::Interrupted => "interrupted",
            RunStatus::TimedOut => "timed_out",
            RunStatus::Detached => "detached",
        }
    }

    /// Inverse of [`RunStatus::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "connecting" => RunStatus::Connecting,
            "running" => RunStatus::Running,
            "done" => RunStatus::Done,
            "failed" => RunStatus::Failed,
            "interrupted" => RunStatus::Interrupted,
            "timed_out" => RunStatus::TimedOut,
            "detached" => RunStatus::Detached,
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
        /// Files the call touches, when the agent says (ACP `locations`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
    },
    /// A tool call changed status, was named, or said which files it touches.
    ToolUpdate {
        /// Agent-assigned tool call id.
        id: String,
        /// New lifecycle status; empty when only `paths` or `title` came.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        status: String,
        /// A better title than the call started with; empty when it did not
        /// change. The Claude adapter starts a call while its input is still
        /// streaming — "Terminal", "Read File" — and names the command or the
        /// file in an update once the input is whole.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        /// Files the call touches, when the update says (some agents, e.g.
        /// Codex, name an edit's files only after it starts).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
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
    /// The app closed, or the machine rebooted, while the turn was live.
    /// Written when the app comes back, not when it went: nothing was
    /// running to write it then.
    Interrupted {
        /// What happened and when, for a human: "the machine rebooted at
        /// 23:34; the run was last seen alive at 23:31".
        why: String,
    },
    /// The app ended the turn because the agent stopped answering.
    TimedOut {
        why: String,
    },
    /// The app closed under the turn but found the agent process still
    /// alive when it came back. Not the end of the run: `Interrupted`
    /// follows once that process is gone.
    Detached {
        why: String,
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
            AgentEvent::Started { .. }
                | AgentEvent::Finished { .. }
                | AgentEvent::Failed { .. }
                | AgentEvent::Interrupted { .. }
                | AgentEvent::TimedOut { .. }
                | AgentEvent::Detached { .. }
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
            AgentEvent::Interrupted { .. } => "interrupted",
            AgentEvent::TimedOut { .. } => "timed_out",
            AgentEvent::Detached { .. } => "detached",
        }
    }

    /// Whether this event ends the run. A detached run has not ended: its
    /// agent is still going, out of reach.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentEvent::Finished { .. } | AgentEvent::Failed { .. } | AgentEvent::Interrupted { .. } | AgentEvent::TimedOut { .. })
    }

    /// The status a terminal event (or a detachment) moves the run to.
    pub fn ends_as(&self) -> Option<RunStatus> {
        Some(match self {
            AgentEvent::Finished { .. } => RunStatus::Done,
            AgentEvent::Failed { .. } => RunStatus::Failed,
            AgentEvent::Interrupted { .. } => RunStatus::Interrupted,
            AgentEvent::TimedOut { .. } => RunStatus::TimedOut,
            AgentEvent::Detached { .. } => RunStatus::Detached,
            _ => return None,
        })
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
            AgentEvent::Interrupted { .. } => "cut",
            AgentEvent::TimedOut { .. } => "timeout",
            AgentEvent::Detached { .. } => "detached",
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

/// A reply's text, put together from the chunks an agent streams.
///
/// Text an agent writes on either side of a tool call is two passages, not
/// one: "It's an npm install, not Homebrew." then, after the call,
/// "`check-claude-app` is gone". Joined as they came they read as one run-on
/// line. A run's stored output (crates/store) and a routine's history
/// (src-tauri/src/routine.rs) both put a reply together this way, so the two
/// read the same.
#[derive(Debug, Default)]
pub struct Reply {
    text: String,
    after_tool: bool,
}

impl Reply {
    /// Add a chunk of what the agent said.
    pub fn say(&mut self, chunk: &str) {
        if self.after_tool && !self.text.is_empty() {
            // A blank line between them -- and exactly one, however many
            // newlines the two sides already bring to the seam.
            let have = self.text.chars().rev().take_while(|c| *c == '\n').count() + chunk.chars().take_while(|c| *c == '\n').count();
            for _ in have..2 {
                self.text.push('\n');
            }
        }
        self.after_tool = false;
        self.text.push_str(chunk);
    }

    /// A tool call came between what was said and what comes next.
    pub fn tool(&mut self) {
        self.after_tool = true;
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_reply_keeps_passages_on_either_side_of_a_tool_apart() {
        let mut r = super::Reply::default();
        r.say("It's an npm global ");
        r.say("install, not Homebrew.");
        r.tool();
        r.say("`check-claude-app` is gone.");
        assert_eq!(r.as_str(), "It's an npm global install, not Homebrew.\n\n`check-claude-app` is gone.");

        let mut list = super::Reply::default();
        list.say("- reading the file\n");
        list.tool();
        list.say("The file is empty.");
        assert_eq!(list.into_string(), "- reading the file\n\nThe file is empty.", "one newline is not a paragraph break");
    }


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
            status: "pending".into(),
            paths: vec![],
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
                paths: vec![],
            },
            AgentEvent::ToolUpdate { id: "1".into(), status: "completed".into(), title: String::new(), paths: vec![] },
            AgentEvent::ToolUpdate { id: "1".into(), status: String::new(), title: String::new(), paths: vec!["a.rs".into()] },
            AgentEvent::ToolUpdate { id: "1".into(), status: String::new(), title: "cargo build".into(), paths: vec![] },
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
            AgentEvent::Interrupted { why: "rebooted".into() },
            AgentEvent::TimedOut { why: "quiet".into() },
            AgentEvent::Detached { why: "alive".into() },
        ];
        for ev in samples {
            let json = serde_json::to_value(&ev).unwrap();
            assert_eq!(json["kind"], ev.kind(), "{ev:?}");
        }
    }

    #[test]
    fn run_status_round_trips() {
        for s in [
            RunStatus::Connecting,
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Interrupted,
            RunStatus::TimedOut,
            RunStatus::Detached,
        ] {
            assert_eq!(RunStatus::parse(s.as_str()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
    }

    /// The ways a run ends are told apart, and a detachment is not an end.
    #[test]
    fn the_ways_a_run_ends_are_kept_apart() {
        let cut = AgentEvent::Interrupted { why: "the machine rebooted".into() };
        let late = AgentEvent::TimedOut { why: "said nothing for 20 minutes".into() };
        let loose = AgentEvent::Detached { why: "its agent is still alive".into() };
        assert!(cut.is_terminal() && late.is_terminal(), "both end the run");
        assert!(!loose.is_terminal(), "a detached run is still going, out of reach");
        assert!(cut.above_membrane() && late.above_membrane() && loose.above_membrane(), "the timeline shows all three");
        assert_eq!(cut.ends_as(), Some(RunStatus::Interrupted));
        assert_eq!(late.ends_as(), Some(RunStatus::TimedOut));
        assert_eq!(loose.ends_as(), Some(RunStatus::Detached));
        assert_eq!(AgentEvent::Message { text: "x".into() }.ends_as(), None);
        assert!(RunStatus::Interrupted.is_cut_short() && RunStatus::TimedOut.is_cut_short());
        assert!(!RunStatus::Failed.is_cut_short(), "a failure is the agent's own and is not started again");
        assert!(!RunStatus::Detached.is_cut_short(), "nothing is started again while the agent is alive");
        assert!(!RunStatus::Detached.is_live(), "but it is not a turn the app can drive either");
    }

    #[test]
    fn events_serialize_tagged() {
        let json = serde_json::to_string(&AgentEvent::Message { text: "x".into() }).unwrap();
        assert_eq!(json, r#"{"kind":"message","text":"x"}"#);
    }
}
