//! Plans: a routine's instruction broken into steps, worked out with an
//! agent of its own.
//!
//! A plan is the "how" beside a routine's "what". Its steps say what to do
//! in order; a step's `after` names the steps that must be done before it,
//! so the plan is a graph without cycles, read as waves: everything whose
//! steps before it are done can go together. When the routine runs, its one
//! session is given the instruction and the plan, wave by wave. The plan
//! does not drive a session per step.
//!
//! A plan is kept as an artifact of kind `plan`, linked from its routine
//! (`routines.plan`), so its conversation runs the way a design's does: one
//! long-lived session in the artifact's own folder, on an agent and options
//! of its own — never the routine's, which do the work. The agent changes
//! the plan through MCP tools. Like a design's board, what it changes lands
//! at once and stays a suggestion until the human keeps or reverts it, a
//! human edit to a suggested step keeps it, and undo takes back only the
//! human's own edits.
//!
//! Edits, the human's and the agent's, gather in the working plan. A run is
//! given the plan as it was last saved: saving makes a numbered version
//! (when, and a note if the human wrote one), a run keeps the number of the
//! version it ran, and any saved version can be brought back into the
//! working plan. A plan never saved is not given to runs.
//!
//! Plans are reached through their routine and nowhere else: they are left
//! out of the artifact list that the designs page, the composer's
//! attachments and other devices read (see `artifact::listed`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orchestra_mcp::Tool;
use orchestra_store::Routine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::{AppHandle, AppState};

/// The artifact kind this module serves.
pub const KIND: &str = "plan";

/// Bounds on one plan. Every step goes to the routine's agent with every
/// run, and every edit saves and sends the whole plan.
pub const MAX_STEPS: usize = 40;
pub const MAX_AFTER: usize = 8;
const MAX_TITLE: usize = 120;
const MAX_INSTRUCTION: usize = 4_000;
const MAX_DONE_WHEN: usize = 1_000;
const MAX_IO: usize = 12;
const MAX_IO_TEXT: usize = 200;
const MAX_AGENT: usize = 64;
const MAX_PROPOSAL: usize = 20_000;
const MAX_OPS: usize = 100;
/// Saved versions kept per plan; the oldest go first.
const MAX_VERSIONS: usize = 50;
const MAX_NOTE: usize = 200;

// ----- the plan -----

/// What the steps after a failed one do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    #[default]
    Stop,
    Continue,
}

/// One step of a plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// `s` and a number, never reused within a plan.
    pub id: String,
    pub title: String,
    /// What to do in this step, written to be read on its own.
    #[serde(default)]
    pub instruction: String,
    /// The steps that must be done before this one.
    #[serde(default)]
    pub after: Vec<String>,
    /// What it uses: an earlier step's output, a file, a list.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// What it leaves behind.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// How to tell it is done.
    #[serde(default)]
    pub done_when: String,
    /// The agent meant for it; empty is the routine's. Kept, not shown:
    /// a routine runs its whole plan in one session (see the module note).
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub on_fail: OnFail,
    /// Where a person put it on the graph; none is laid out automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// `human` or `agent`: who made it.
    #[serde(default)]
    pub by: String,
}

/// An agent change waiting for the human: the step as it was (none when the
/// agent made it) and as it is now (none when the agent removed it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: u64,
    /// The step it is about.
    pub target: String,
    /// The turn that made it.
    #[serde(default)]
    pub run: Option<String>,
    pub before: Option<Step>,
    pub after: Option<Step>,
}

/// The instruction a plan was made against, to tell when it has moved on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub instruction: String,
    /// Unix milliseconds.
    pub at: i64,
}

/// An instruction the plan's agent proposed for the routine. The agent
/// never writes the routine itself; the human takes the words or not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub text: String,
    #[serde(default)]
    pub run: Option<String>,
    pub at: i64,
}

/// A saved version of the plan: what runs are given, until the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    /// 1, 2, 3...: never reused, even when old versions are let go.
    pub n: u32,
    /// Unix milliseconds.
    pub at: i64,
    /// What the human said of it, if anything.
    #[serde(default)]
    pub note: String,
    pub steps: Vec<Step>,
    /// When it was last saved over, if it was (Unix milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
}

/// The steps as they bear on a run: where they sit on the graph does not.
fn content(steps: &[Step]) -> Vec<Step> {
    steps.iter().map(|s| Step { x: None, y: None, ..s.clone() }).collect()
}

/// The whole plan: the artifact's body.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    #[serde(default)]
    pub version: u64,
    /// The routine it belongs to.
    #[serde(default)]
    pub routine: String,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Agent changes the human has not kept or reverted yet.
    #[serde(default)]
    pub changes: Vec<Change>,
    /// Counter for step and change ids.
    #[serde(default)]
    pub next: u64,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
    /// Saved versions, oldest first.
    #[serde(default)]
    pub saved: Vec<Version>,
    /// The version the working plan was last opened from or saved as: what
    /// "save over" writes into. None before any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<u32>,
    /// The version runs are given: the one last saved or saved over. None
    /// (a plan saved before this was kept) reads as the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<u32>,
    /// What the human's edit in progress touched, each step as it was
    /// before; turned into an undo step when the edit is kept. Not saved.
    #[serde(skip)]
    journal: Vec<(String, Option<Step>)>,
}

/// What pressing Run does with a plan (see [`Doc::gate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Straight away: no plan, or the working plan is the version runs are given.
    Run,
    /// Steps never saved: save and run, or not.
    AskSave,
    /// The working plan differs from the version runs are given.
    AskUnsaved,
    /// Suggestions wait, so it cannot be saved: settle them first.
    Settle,
}

/// Who is editing.
#[derive(Clone, Debug)]
pub enum Actor {
    Human,
    Agent { run: Option<String> },
}

impl Actor {
    fn name(&self) -> &'static str {
        match self {
            Actor::Human => "human",
            Actor::Agent { .. } => "agent",
        }
    }
}

/// One edit to a plan. The window and the agent speak the same set, less
/// the few that are the human's alone (marked).
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddStep {
        title: String,
        #[serde(default)]
        instruction: String,
        #[serde(default)]
        after: Vec<String>,
        #[serde(default)]
        inputs: Vec<String>,
        #[serde(default)]
        outputs: Vec<String>,
        #[serde(default)]
        done_when: String,
        #[serde(default)]
        agent: String,
        #[serde(default)]
        on_fail: OnFail,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    UpdateStep {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        instruction: Option<String>,
        #[serde(default)]
        inputs: Option<Vec<String>>,
        #[serde(default)]
        outputs: Option<Vec<String>>,
        #[serde(default)]
        done_when: Option<String>,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        on_fail: Option<OnFail>,
    },
    /// `to` waits for `from`.
    Link { from: String, to: String },
    Unlink { from: String, to: String },
    /// The whole list of steps `id` waits for.
    SetAfter { id: String, after: Vec<String> },
    Delete { ids: Vec<String> },
    /// Put a step somewhere by hand; it stays there through layouts.
    Move { id: String, x: f64, y: f64 },
    /// Let every step be laid out automatically again.
    AutoLayout {},
    /// The human's: the plan now answers to this instruction.
    Restamp { instruction: String },
    /// The human's: put the agent's proposed instruction away.
    DismissProposal {},
    /// The human's: save the working plan as a new version, the one runs
    /// are given. Refused while agent suggestions wait to be settled.
    Save {
        #[serde(default)]
        note: String,
        /// Save over this version instead of making a new one. Runs that
        /// already went keep the plan they were given (their own copy).
        #[serde(default)]
        over: Option<u32>,
    },
    /// The human's: bring a saved version back into the working plan (an
    /// edit like any other: it can be undone, and saved again).
    Open { n: u32 },
}

impl Op {
    fn humans_only(&self) -> Option<&'static str> {
        match self {
            Op::Restamp { .. } => Some("restamp"),
            Op::DismissProposal {} => Some("dismiss_proposal"),
            Op::Save { .. } => Some("save"),
            Op::Open { .. } => Some("open"),
            _ => None,
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn check_len(text: &str, max: usize, what: &str) -> Result<(), String> {
    if text.chars().count() > max {
        Err(format!("{what} is longer than {max} characters"))
    } else {
        Ok(())
    }
}

fn check_list(items: &[String], what: &str) -> Result<Vec<String>, String> {
    if items.len() > MAX_IO {
        return Err(format!("at most {MAX_IO} {what}"));
    }
    let mut out = Vec::new();
    for item in items {
        let item = item.trim();
        check_len(item, MAX_IO_TEXT, &format!("one of the {what}"))?;
        if !item.is_empty() && !out.iter().any(|x| x == item) {
            out.push(item.to_string());
        }
    }
    Ok(out)
}

fn check_agent(agent: &str) -> Result<String, String> {
    let agent = agent.trim();
    if agent.len() > MAX_AGENT || !agent.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(format!("{agent:?} is not an agent id"));
    }
    Ok(agent.to_string())
}

fn finite(v: f64, what: &str) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{what} must be a number"))
    }
}

impl Doc {
    fn fresh_id(&mut self) -> String {
        self.next += 1;
        format!("s{}", self.next)
    }

    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    fn has(&self, id: &str) -> bool {
        self.steps.iter().any(|s| s.id == id)
    }

    /// The steps `from` waits for, all the way back, as a path from `from`
    /// to `to` when `to` is among them. Making `to` wait for `from` would
    /// then close a cycle, and the path is the cycle the human is shown.
    pub fn path_back(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let mut prev: HashMap<String, String> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
        let mut queue = std::collections::VecDeque::from([from.to_string()]);
        while let Some(v) = queue.pop_front() {
            if v == to {
                let mut path = vec![v.clone()];
                let mut at = v;
                while let Some(p) = prev.get(&at) {
                    path.push(p.clone());
                    at = p.clone();
                }
                path.reverse();
                return Some(path);
            }
            for d in self.step(&v).map(|s| s.after.as_slice()).unwrap_or_default() {
                if seen.insert(d.clone()) {
                    prev.insert(d.clone(), v.clone());
                    queue.push_back(d.clone());
                }
            }
        }
        None
    }

    /// Check a list of steps for `id` to wait for: existing, not itself,
    /// no repeats, within bounds, and no cycle.
    fn check_after(&self, id: Option<&str>, after: &[String]) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        for d in after {
            let d = d.trim();
            if out.iter().any(|x| x == d) {
                continue;
            }
            if Some(d) == id {
                return Err(format!("{d} cannot wait for itself"));
            }
            if !self.has(d) {
                return Err(format!("no step {d}"));
            }
            if let Some(id) = id {
                if let Some(path) = self.path_back(d, id) {
                    // `path` runs from d back to id; with id waiting for d it closes.
                    let mut cycle: Vec<String> = path.into_iter().rev().collect();
                    cycle.push(id.to_string());
                    return Err(format!("{id} cannot wait for {d}: that makes a cycle ({})", cycle.join(" -> ")));
                }
            }
            out.push(d.to_string());
        }
        if out.len() > MAX_AFTER {
            return Err(format!("a step waits for at most {MAX_AFTER} others"));
        }
        Ok(out)
    }

    /// The waves: steps grouped by how many steps stand before them on the
    /// longest way back. Everything in a wave can go together once the waves
    /// before it are done. Steps keep their plan order within a wave.
    pub fn waves(&self) -> Vec<Vec<String>> {
        let mut depth: HashMap<&str, usize> = HashMap::new();
        // Iterated to a fixed point rather than recursed: a plan from disk is
        // trusted to have no cycle, but a bad one must still end.
        for _ in 0..=self.steps.len() {
            let mut moved = false;
            for s in &self.steps {
                let d = s.after.iter().filter_map(|a| depth.get(a.as_str()).map(|d| d + 1)).max().unwrap_or(0);
                if depth.get(s.id.as_str()) != Some(&d) {
                    depth.insert(&s.id, d);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        let mut out: Vec<Vec<String>> = Vec::new();
        for s in &self.steps {
            let d = depth.get(s.id.as_str()).copied().unwrap_or(0).min(self.steps.len());
            if out.len() <= d {
                out.resize(d + 1, Vec::new());
            }
            out[d].push(s.id.clone());
        }
        out.retain(|w| !w.is_empty());
        out
    }

    /// What a careful reader would point out: things that are allowed but
    /// probably not meant.
    pub fn issues(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.steps {
            if s.instruction.trim().is_empty() {
                out.push(format!("{} has no instruction", s.id));
            }
            if s.done_when.trim().is_empty() {
                out.push(format!("{} does not say how to tell it is done", s.id));
            }
            // An input is met by an output of a step it waits for, directly
            // or not; anything else is a file or a fact the step finds itself,
            // which is fine, but worth a look when an output has that name.
            let mut before: HashSet<&str> = HashSet::new();
            let mut stack: Vec<&str> = s.after.iter().map(String::as_str).collect();
            while let Some(v) = stack.pop() {
                if before.insert(v) {
                    stack.extend(self.step(v).map(|x| x.after.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default());
                }
            }
            for input in &s.inputs {
                let made_by: Vec<&str> = self.steps.iter().filter(|o| o.id != s.id && o.outputs.contains(input)).map(|o| o.id.as_str()).collect();
                if !made_by.is_empty() && !made_by.iter().any(|m| before.contains(m)) {
                    out.push(format!("{} uses {input:?}, which {} makes, but does not wait for it", s.id, made_by.join(", ")));
                }
            }
        }
        out
    }

    /// The version runs are given now, if the plan was ever saved: the one
    /// last saved or saved over (the last, for a plan saved before that was
    /// kept, or when that version was let go).
    pub fn run_version(&self) -> Option<&Version> {
        self.run.and_then(|n| self.saved.iter().find(|v| v.n == n)).or_else(|| self.saved.last())
    }

    /// Whether the working plan differs from the version runs are given, in
    /// what a run would get (where steps sit on the graph does not count).
    /// A plan never saved differs once it has steps.
    pub fn unsaved(&self) -> bool {
        match self.run_version() {
            None => !self.steps.is_empty(),
            Some(v) => content(&v.steps) != content(&self.steps),
        }
    }

    /// What pressing Run does with this plan: the one rule for the window's
    /// Run (`runGate` in ui/src/lib/planGraph.ts, tested on the same cases)
    /// and the conductor's `run_routine`, which can only refuse.
    pub fn gate(&self) -> Gate {
        if !self.changes.is_empty() {
            Gate::Settle
        } else if !self.unsaved() {
            Gate::Run
        } else if self.saved.is_empty() {
            Gate::AskSave
        } else {
            Gate::AskUnsaved
        }
    }

    /// Whether the routine's instruction has moved on since the plan was made.
    pub fn instruction_changed(&self, instruction: &str) -> bool {
        self.source.as_ref().is_some_and(|s| s.instruction.trim() != instruction.trim())
    }

    /// The plan as the agent reads it.
    pub fn outline(&self) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|s| {
                let mut v = json!({
                    "id": s.id, "title": s.title, "instruction": s.instruction, "after": s.after,
                    "inputs": s.inputs, "outputs": s.outputs, "done_when": s.done_when,
                    "on_fail": s.on_fail, "by": s.by,
                });
                if !s.agent.is_empty() {
                    v["agent"] = json!(s.agent);
                }
                v
            })
            .collect();
        let pending: Vec<&str> = self.changes.iter().map(|c| c.target.as_str()).collect();
        let mut out = json!({
            "steps": steps,
            "waves": self.waves(),
            "pending_suggestions": pending,
            "issues": self.issues(),
            "saved_version": self.run_version().map(|v| v.n),
            "unsaved_changes": self.unsaved(),
        });
        if let Some(p) = &self.proposal {
            out["proposed_instruction"] = json!(p.text);
        }
        out
    }

    /// Put a step back as it was, or take it away when it was not there.
    fn restore(&mut self, target: &str, what: Option<&Step>) {
        match what {
            Some(s) => match self.steps.iter().position(|x| x.id == target) {
                Some(i) => self.steps[i] = s.clone(),
                None => self.steps.push(s.clone()),
            },
            None => self.steps.retain(|s| s.id != target),
        }
    }

    /// After steps came back or went away by review or undo: drop waits on
    /// steps that are gone, and any wait that now closes a cycle (possible
    /// when one side of two crossing edits is reverted). Not recorded: it
    /// follows from the change that caused it.
    fn repair(&mut self) {
        let ids: HashSet<String> = self.steps.iter().map(|s| s.id.clone()).collect();
        for s in &mut self.steps {
            s.after.retain(|a| ids.contains(a) && *a != s.id);
        }
        for i in 0..self.steps.len() {
            let id = self.steps[i].id.clone();
            let mut keep = Vec::new();
            for d in self.steps[i].after.clone() {
                // Check against the waits kept so far, so the first of two
                // crossing waits stays and the later one goes.
                self.steps[i].after = keep.clone();
                if self.path_back(&d, &id).is_none() {
                    keep.push(d);
                }
            }
            self.steps[i].after = keep;
        }
    }

    /// Note what an edit did to `target`. A human's settles any suggestion
    /// on it and can be undone; an agent's becomes (or extends) a suggestion.
    fn record(&mut self, actor: &Actor, target: &str, before: Option<Step>) {
        let after = self.step(target).cloned();
        match actor {
            Actor::Human => {
                self.changes.retain(|c| c.target != target);
                if !self.journal.iter().any(|(t, _)| t == target) {
                    self.journal.push((target.to_string(), before));
                }
            }
            Actor::Agent { run } => {
                if let Some(c) = self.changes.iter_mut().find(|c| c.target == target) {
                    c.after = after;
                    c.run = run.clone();
                } else {
                    self.next += 1;
                    let id = self.next;
                    self.changes.push(Change { id, target: target.to_string(), run: run.clone(), before, after });
                }
                // Made and removed in the same breath, or put back as it was:
                // nothing to review.
                self.changes.retain(|c| c.before != c.after);
            }
        }
    }

    /// Change one step's wait list, recorded.
    fn set_after(&mut self, actor: &Actor, id: &str, after: Vec<String>) -> Result<(), String> {
        let before = self.step(id).cloned().ok_or_else(|| format!("no step {id}"))?;
        let after = self.check_after(Some(id), &after)?;
        if before.after == after {
            return Ok(());
        }
        if let Some(s) = self.steps.iter_mut().find(|s| s.id == id) {
            s.after = after;
        }
        self.record(actor, id, Some(before));
        Ok(())
    }

    /// Apply edits in order. Each result is `{ok, id?, ...}` or
    /// `{ok: false, error}`; a failed edit does not stop the rest.
    pub fn apply(&mut self, ops: Vec<Op>, actor: &Actor) -> Vec<Value> {
        if ops.len() > MAX_OPS {
            return vec![json!({ "ok": false, "error": format!("at most {MAX_OPS} commands per call") })];
        }
        let mut out = Vec::new();
        for op in ops {
            let result = (|| {
                if let (Actor::Agent { .. }, Some(name)) = (actor, op.humans_only()) {
                    return Err(format!("{name} is the human's to do"));
                }
                match op {
                    Op::AddStep { title, instruction, after, inputs, outputs, done_when, agent, on_fail, x, y } => {
                        if self.steps.len() >= MAX_STEPS {
                            return Err(format!("a plan holds at most {MAX_STEPS} steps"));
                        }
                        let title = title.trim().to_string();
                        if title.is_empty() {
                            return Err("a step needs a title".to_string());
                        }
                        check_len(&title, MAX_TITLE, "a step's title")?;
                        check_len(&instruction, MAX_INSTRUCTION, "a step's instruction")?;
                        check_len(&done_when, MAX_DONE_WHEN, "a step's done_when")?;
                        let after = self.check_after(None, &after)?;
                        let step = Step {
                            id: self.fresh_id(),
                            title,
                            instruction: instruction.trim().to_string(),
                            after,
                            inputs: check_list(&inputs, "inputs")?,
                            outputs: check_list(&outputs, "outputs")?,
                            done_when: done_when.trim().to_string(),
                            agent: check_agent(&agent)?,
                            on_fail,
                            x: x.map(|v| finite(v, "x")).transpose()?,
                            y: y.map(|v| finite(v, "y")).transpose()?,
                            by: actor.name().to_string(),
                        };
                        let id = step.id.clone();
                        self.steps.push(step);
                        self.record(actor, &id, None);
                        Ok(self.wrote(&id))
                    }
                    Op::UpdateStep { id, title, instruction, inputs, outputs, done_when, agent, on_fail } => {
                        let before = self.step(&id).cloned().ok_or_else(|| format!("no step {id}"))?;
                        let mut s = before.clone();
                        if let Some(t) = title {
                            let t = t.trim().to_string();
                            if t.is_empty() {
                                return Err("a step needs a title".to_string());
                            }
                            check_len(&t, MAX_TITLE, "a step's title")?;
                            s.title = t;
                        }
                        if let Some(v) = instruction {
                            check_len(&v, MAX_INSTRUCTION, "a step's instruction")?;
                            s.instruction = v.trim().to_string();
                        }
                        if let Some(v) = inputs {
                            s.inputs = check_list(&v, "inputs")?;
                        }
                        if let Some(v) = outputs {
                            s.outputs = check_list(&v, "outputs")?;
                        }
                        if let Some(v) = done_when {
                            check_len(&v, MAX_DONE_WHEN, "a step's done_when")?;
                            s.done_when = v.trim().to_string();
                        }
                        if let Some(v) = agent {
                            s.agent = check_agent(&v)?;
                        }
                        if let Some(v) = on_fail {
                            s.on_fail = v;
                        }
                        if s != before {
                            self.restore(&id, Some(&s));
                            self.record(actor, &id, Some(before));
                        }
                        Ok(self.wrote(&id))
                    }
                    Op::Link { from, to } => {
                        let mut after = self.step(&to).ok_or_else(|| format!("no step {to}"))?.after.clone();
                        after.push(from);
                        self.set_after(actor, &to, after)?;
                        Ok(json!({ "ok": true, "id": to, "after": self.step(&to).map(|s| s.after.clone()) }))
                    }
                    Op::Unlink { from, to } => {
                        let after: Vec<String> = self.step(&to).ok_or_else(|| format!("no step {to}"))?.after.iter().filter(|a| **a != from).cloned().collect();
                        self.set_after(actor, &to, after)?;
                        Ok(json!({ "ok": true, "id": to, "after": self.step(&to).map(|s| s.after.clone()) }))
                    }
                    Op::SetAfter { id, after } => {
                        self.set_after(actor, &id, after)?;
                        Ok(json!({ "ok": true, "id": id, "after": self.step(&id).map(|s| s.after.clone()) }))
                    }
                    Op::Delete { ids } => {
                        let mut gone = Vec::new();
                        let mut missing = Vec::new();
                        for id in ids {
                            let Some(before) = self.step(&id).cloned() else {
                                if !gone.contains(&id) {
                                    missing.push(id);
                                }
                                continue;
                            };
                            // Steps that waited for it stop waiting, each recorded.
                            let waiting: Vec<String> = self.steps.iter().filter(|s| s.after.contains(&id)).map(|s| s.id.clone()).collect();
                            for w in waiting {
                                let b = self.step(&w).cloned();
                                if let Some(s) = self.steps.iter_mut().find(|s| s.id == w) {
                                    s.after.retain(|a| *a != id);
                                }
                                self.record(actor, &w, b);
                            }
                            self.steps.retain(|s| s.id != id);
                            self.record(actor, &id, Some(before));
                            gone.push(id);
                        }
                        Ok(json!({ "ok": missing.is_empty(), "deleted": gone, "missing": missing }))
                    }
                    Op::Move { id, x, y } => {
                        let before = self.step(&id).cloned().ok_or_else(|| format!("no step {id}"))?;
                        let (x, y) = (finite(x, "x")?, finite(y, "y")?);
                        if let Some(s) = self.steps.iter_mut().find(|s| s.id == id) {
                            s.x = Some(x);
                            s.y = Some(y);
                        }
                        self.record(actor, &id, Some(before));
                        Ok(json!({ "ok": true, "id": id }))
                    }
                    Op::AutoLayout {} => {
                        let pinned: Vec<String> = self.steps.iter().filter(|s| s.x.is_some() || s.y.is_some()).map(|s| s.id.clone()).collect();
                        for id in &pinned {
                            let before = self.step(id).cloned();
                            if let Some(s) = self.steps.iter_mut().find(|s| &s.id == id) {
                                s.x = None;
                                s.y = None;
                            }
                            self.record(actor, id, before);
                        }
                        Ok(json!({ "ok": true, "unpinned": pinned }))
                    }
                    Op::Restamp { instruction } => {
                        self.source = Some(Source { instruction, at: now_ms() });
                        Ok(json!({ "ok": true }))
                    }
                    Op::DismissProposal {} => {
                        self.proposal = None;
                        Ok(json!({ "ok": true }))
                    }
                    Op::Save { note, over } => {
                        if !self.changes.is_empty() {
                            return Err(format!("keep or revert the {} suggestion(s) first", self.changes.len()));
                        }
                        let note = note.trim().to_string();
                        check_len(&note, MAX_NOTE, "a note")?;
                        let now = content(&self.steps);
                        if let Some(n) = over {
                            let steps = self.steps.clone();
                            let v = self.saved.iter_mut().find(|v| v.n == n).ok_or_else(|| format!("no saved version {n}"))?;
                            if content(&v.steps) == now {
                                return Err(format!("v{n} already has this plan"));
                            }
                            v.steps = steps;
                            v.edited_at = Some(now_ms());
                            // A new note replaces the old one; none keeps it.
                            if !note.is_empty() {
                                v.note = note;
                            }
                            self.opened = Some(n);
                            // Saved over, it is what runs are given from here.
                            self.run = Some(n);
                            return Ok(json!({ "ok": true, "version": n, "over": true }));
                        }
                        if self.saved.last().is_some_and(|v| content(&v.steps) == now) || (self.saved.is_empty() && self.steps.is_empty()) {
                            return Err("nothing has changed since the last saved version".to_string());
                        }
                        let n = self.saved.last().map(|v| v.n + 1).unwrap_or(1);
                        self.saved.push(Version { n, at: now_ms(), note, steps: self.steps.clone(), edited_at: None });
                        let drop = self.saved.len().saturating_sub(MAX_VERSIONS);
                        self.saved.drain(..drop);
                        self.opened = Some(n);
                        self.run = Some(n);
                        Ok(json!({ "ok": true, "version": n }))
                    }
                    Op::Open { n } => {
                        if !self.changes.is_empty() {
                            return Err(format!("keep or revert the {} suggestion(s) first", self.changes.len()));
                        }
                        let back = self.saved.iter().find(|v| v.n == n).ok_or_else(|| format!("no saved version {n}"))?.steps.clone();
                        // Each step that changes is a human edit: undone one by
                        // one if the human thinks better of it.
                        let gone: Vec<Step> = self.steps.iter().filter(|s| !back.iter().any(|b| b.id == s.id)).cloned().collect();
                        for s in gone {
                            let id = s.id.clone();
                            self.steps.retain(|x| x.id != id);
                            self.record(actor, &id, Some(s));
                        }
                        for b in &back {
                            let before = self.step(&b.id).cloned();
                            if before.as_ref() == Some(b) {
                                continue;
                            }
                            self.restore(&b.id, Some(b));
                            self.record(actor, &b.id, before);
                            // Ids are never reused: the counter stays ahead of every one brought back.
                            let num: u64 = b.id.trim_start_matches('s').parse().unwrap_or(0);
                            self.next = self.next.max(num);
                        }
                        // In the version's order.
                        self.steps.sort_by_key(|s| back.iter().position(|b| b.id == s.id).unwrap_or(usize::MAX));
                        self.repair();
                        self.opened = Some(n);
                        Ok(json!({ "ok": true, "opened": n }))
                    }
                }
            })();
            out.push(result.unwrap_or_else(|error| json!({ "ok": false, "error": error })));
        }
        out
    }

    /// The answer to an edit that wrote words: the step's id and the words
    /// the plan now holds for it, to read against what was sent.
    fn wrote(&self, id: &str) -> Value {
        match self.step(id) {
            Some(s) => json!({ "ok": true, "id": id, "title": s.title, "instruction": s.instruction, "done_when": s.done_when, "after": s.after }),
            None => json!({ "ok": true, "id": id }),
        }
    }

    /// Settle agent changes: keep them, or put the steps back.
    pub fn review(&mut self, ids: &[u64], keep: bool) {
        let (settled, rest): (Vec<Change>, Vec<Change>) = std::mem::take(&mut self.changes).into_iter().partition(|c| ids.contains(&c.id));
        self.changes = rest;
        if keep {
            return;
        }
        for c in settled.iter().rev() {
            self.restore(&c.target, c.before.as_ref());
        }
        self.repair();
    }

    /// The human's edit just made, as an undo step.
    fn take_step(&mut self) -> Option<UndoStep> {
        let touched = std::mem::take(&mut self.journal);
        (!touched.is_empty()).then(|| {
            touched
                .into_iter()
                .map(|(t, before)| {
                    let after = self.step(&t).cloned();
                    (t, before, after)
                })
                .collect()
        })
    }

    /// Put the steps of an undo step back as they were before it (or, with
    /// `again`, as it left them). A step that changed since is left alone:
    /// undo never overwrites someone else's later work.
    fn replay(&mut self, step: &UndoStep, again: bool) {
        for (target, before, after) in step.iter().rev() {
            let (expected, wanted) = if again { (before, after) } else { (after, before) };
            if self.step(target) != expected.as_ref() {
                continue;
            }
            self.restore(target, wanted.as_ref());
            self.changes.retain(|c| &c.target != target);
        }
        self.repair();
        self.journal.clear();
    }
}

type UndoStep = Vec<(String, Option<Step>, Option<Step>)>;

/// Undo steps kept per plan.
const UNDO_DEPTH: usize = 100;

/// One plan, loaded on first use, with the human's undo and redo steps (in
/// memory only). Its lock is held for a whole edit (read, change, save), so
/// a human's edit and the agent's tool call never start from the same
/// version and overwrite each other.
#[derive(Default)]
struct Slot {
    doc: Option<Doc>,
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
}

/// Every open plan.
#[derive(Default)]
pub struct Plans {
    docs: parking_lot::Mutex<HashMap<String, Arc<parking_lot::Mutex<Slot>>>>,
}

impl Plans {
    /// Forget a plan (its artifact is gone).
    pub fn forget(&self, id: &str) {
        self.docs.lock().remove(id);
    }
}

/// The plan's slot, loaded. A plan that does not load leaves no slot behind,
/// so asking for ids that are not plans does not grow the map.
fn loaded(state: &AppState, id: &str) -> Result<Arc<parking_lot::Mutex<Slot>>, String> {
    if let Some(slot) = state.plans.docs.lock().get(id).cloned() {
        return Ok(slot);
    }
    let doc = load_doc(state, id)?;
    let mut docs = state.plans.docs.lock();
    let slot = docs.entry(id.to_string()).or_insert_with(|| Arc::new(parking_lot::Mutex::new(Slot { doc: Some(doc), ..Slot::default() })));
    Ok(slot.clone())
}

fn load_doc(state: &AppState, id: &str) -> Result<Doc, String> {
    let (info, raw) = state.store.artifact(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no plan {id}"))?;
    if info.kind != KIND {
        return Err(format!("{id} is not a plan"));
    }
    // Only an empty body is a new plan; anything else that does not read
    // stops here rather than showing an empty plan the next edit would save
    // over the real one.
    if raw.trim().is_empty() || raw.trim() == "{}" {
        return Ok(Doc::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("plan {id} could not be read: {e}"))
}

/// The plan as it is now.
pub fn doc(state: &AppState, id: &str) -> Result<Doc, String> {
    let slot = loaded(state, id)?;
    let mut guard = slot.lock();
    if guard.doc.is_none() {
        guard.doc = Some(load_doc(state, id)?);
    }
    guard.doc.clone().ok_or_else(|| format!("plan {id} vanished"))
}

/// What the window hears after every change: the plan whole (it is small),
/// and the version it was made on, so a window holding another knows it
/// missed one.
#[derive(Clone, Debug, Serialize)]
pub struct PlanEvent {
    pub id: String,
    pub base: u64,
    pub doc: Doc,
}

/// Run `f` on the plan (and its undo steps), save it, and tell the window.
fn change<R>(app: &AppHandle, id: &str, f: impl FnOnce(&mut Doc, &mut Vec<UndoStep>, &mut Vec<UndoStep>) -> R) -> Result<R, String> {
    let state = app.state::<AppState>();
    let slot = loaded(&state, id)?;
    let (out, event) = {
        let mut guard = slot.lock();
        let held = &mut *guard;
        if held.doc.is_none() {
            held.doc = Some(load_doc(&state, id)?);
        }
        let old = held.doc.take().ok_or_else(|| format!("plan {id} vanished"))?;
        let mut current = old.clone();
        current.journal.clear();
        let (kept_undo, kept_redo) = (held.undo.clone(), held.redo.clone());
        let (mut undo, mut redo) = (std::mem::take(&mut held.undo), std::mem::take(&mut held.redo));
        let out = f(&mut current, &mut undo, &mut redo);
        current.version += 1;
        let saved = serde_json::to_string(&current).map_err(|e| e.to_string()).and_then(|json| {
            state
                .store
                .update_artifact(id, &orchestra_store::ArtifactPatch { body: Some(json), ..Default::default() })
                .map_err(|e| e.to_string())
        });
        if let Err(err) = saved {
            held.doc = Some(old);
            held.undo = kept_undo;
            held.redo = kept_redo;
            return Err(err);
        }
        let event = PlanEvent { id: id.to_string(), base: old.version, doc: current.clone() };
        held.doc = Some(current);
        held.undo = undo;
        held.redo = redo;
        (out, event)
    };
    let _ = app.emit("plan", &event);
    Ok(out)
}

/// `change` for an edit: what the human's part of it touched becomes an
/// undo step (and a new edit forgets what was undone).
fn edit<R>(app: &AppHandle, id: &str, f: impl FnOnce(&mut Doc) -> R) -> Result<R, String> {
    change(app, id, |d, undo, redo| {
        let out = f(d);
        if let Some(step) = d.take_step() {
            undo.push(step);
            redo.clear();
            let over = undo.len().saturating_sub(UNDO_DEPTH);
            undo.drain(..over);
        }
        out
    })
}

/// Apply edits to a plan.
pub fn apply(app: &AppHandle, id: &str, ops: Vec<Op>, actor: Actor) -> Result<Vec<Value>, String> {
    edit(app, id, |d| d.apply(ops, &actor))
}

/// Keep or revert agent suggestions.
pub fn review(app: &AppHandle, id: &str, changes: &[u64], keep: bool) -> Result<(), String> {
    edit(app, id, |d| d.review(changes, keep))
}

/// Undo the human's last edit (or redo, with `again`). Returns whether
/// there was one.
pub fn undo(app: &AppHandle, id: &str, again: bool) -> Result<bool, String> {
    change(app, id, |d, undo, redo| {
        let (from, to) = if again { (redo, undo) } else { (undo, redo) };
        let Some(step) = from.pop() else { return false };
        d.replay(&step, again);
        to.push(step);
        true
    })
}

/// The agent's proposed instruction for the routine, kept on the plan for
/// the human to take or put away.
fn propose(app: &AppHandle, id: &str, text: String, run: Option<String>) -> Result<(), String> {
    change(app, id, |d, _, _| d.proposal = Some(Proposal { text, run, at: now_ms() }))
}

/// A fresh plan's body for a routine.
pub fn new_body(routine: &Routine) -> Result<String, String> {
    let doc = Doc {
        routine: routine.id.clone(),
        source: Some(Source { instruction: routine.instruction.clone(), at: now_ms() }),
        ..Doc::default()
    };
    serde_json::to_string(&doc).map_err(|e| e.to_string())
}

/// The routine a plan belongs to.
fn routine_of(state: &AppState, id: &str) -> Result<Routine, String> {
    let d = doc(state, id)?;
    state
        .store
        .routine(&d.routine)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("the routine of plan {id} is gone"))
}

// ----- the plan in a run -----

/// What a run is given beyond its instruction: the plan, wave by wave, as
/// one section of its prompt, and the plan as it read then (kept with the
/// run, every word of each step, so a version saved over later does not
/// change what the run is shown to have been given). None when there is
/// nothing to give: never saved, or a version with no steps. A routine
/// without a plan then runs exactly as before.
pub struct Guidance {
    pub section: String,
    pub snapshot: Value,
    /// The step ids, in plan order: the only ones a result is read for.
    pub ids: Vec<String>,
}

pub fn guidance(doc: &Doc) -> Option<Guidance> {
    let version = doc.run_version()?;
    if version.steps.is_empty() {
        return None;
    }
    // The version as a plan of its own, so its waves are its own.
    let doc = &Doc { steps: version.steps.clone(), ..Doc::default() };
    let waves = doc.waves();
    let mut out = String::from(
        "[plan] Work through these steps wave by wave. The steps of one wave do not wait for each other; a wave starts when the waves before it are done.\n",
    );
    for (i, wave) in waves.iter().enumerate() {
        out.push_str(&format!("\nWave {}{}\n", i + 1, if wave.len() > 1 { " (side by side)" } else { "" }));
        for id in wave {
            let Some(s) = doc.step(id) else { continue };
            out.push_str(&format!("- {} \"{}\"", s.id, s.title));
            if !s.after.is_empty() {
                out.push_str(&format!(" (after {})", s.after.join(", ")));
            }
            out.push('\n');
            if !s.instruction.is_empty() {
                for line in s.instruction.lines() {
                    out.push_str(&format!("  {line}\n"));
                }
            }
            if !s.inputs.is_empty() {
                out.push_str(&format!("  uses: {}\n", s.inputs.join("; ")));
            }
            if !s.outputs.is_empty() {
                out.push_str(&format!("  leaves: {}\n", s.outputs.join("; ")));
            }
            if !s.done_when.is_empty() {
                out.push_str(&format!("  done when: {}\n", s.done_when));
            }
            if s.on_fail == OnFail::Continue {
                out.push_str("  if it fails: carry on with the steps that do not depend on it\n");
            }
        }
    }
    out.push_str(
        "\nAs you begin each step, call the tool `plan_step` with its id and status \"started\"; when it ends, call it again with \"done\", \"failed\" or \"skipped\".\n\
         If a step fails, stop the steps that depend on it unless it says to carry on.\n\
         In the report block at the end, add a \"steps\" list with one entry for every step: {\"id\": \"s1\", \"status\": \"done\" | \"failed\" | \"skipped\", \"note\": \"one short line\"}.",
    );
    let steps = serde_json::to_value(content(&doc.steps)).unwrap_or_default();
    Some(Guidance {
        section: out,
        snapshot: json!({ "version": version.n, "saved_at": version.edited_at.unwrap_or(version.at), "note": version.note, "steps": steps, "waves": waves }),
        ids: doc.steps.iter().map(|s| s.id.clone()).collect(),
    })
}

/// Why a run started where no one can be asked (the conductor's
/// `run_routine`) must not go: the same rule as the window's Run
/// ([`Doc::gate`]), where every answer but "run" is a question the window
/// asks and the conductor cannot. The words are for the conductor to pass
/// on as they are. None when the run may go.
pub fn run_refusal(doc: &Doc, ko: bool) -> Option<String> {
    let n = doc.run_version().map(|v| v.n).unwrap_or(0);
    let pending = doc.changes.len();
    let out = match (doc.gate(), ko) {
        (Gate::Run, _) => return None,
        (Gate::AskSave, true) => "계획이 저장되지 않았습니다. 되풀이 화면에서 [계획 저장]을 한 뒤 다시 실행하세요.".to_string(),
        (Gate::AskSave, false) => "The plan is not saved. Save it on the Routines page ([Save plan]), then run it again.".to_string(),
        (Gate::AskUnsaved, true) => format!("계획에 저장하지 않은 변경이 있습니다(실행 버전 v{n}). 되풀이 화면에서 [계획 저장]을 한 뒤 다시 실행하세요."),
        (Gate::AskUnsaved, false) => format!("The plan has changes not saved (runs use v{n}). Save it on the Routines page ([Save plan]), then run it again."),
        (Gate::Settle, true) => format!("처리하지 않은 제안이 {pending}건 있습니다. 되풀이 화면에서 유지하거나 되돌리고 [계획 저장]을 한 뒤 다시 실행하세요."),
        (Gate::Settle, false) => format!("{pending} suggestion(s) are waiting. Keep or revert them on the Routines page, save the plan, then run it again."),
    };
    Some(out)
}

/// What a finished run's agent said of each step: the `steps` list of its
/// report block, kept for the steps it was given and only in the shape
/// asked for. None when there is no such list to keep.
pub fn step_results(reply: &str, ids: &[String]) -> Option<Value> {
    let raw = orchestra_core::report::block(reply).ok()?;
    let v: Value = serde_json::from_str(raw).ok()?;
    let list = v.get("steps")?.as_array()?;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for item in list {
        let Some(id) = item.get("id").and_then(Value::as_str) else { continue };
        let Some(status) = item.get("status").and_then(Value::as_str) else { continue };
        if !ids.iter().any(|x| x == id) || !matches!(status, "done" | "failed" | "skipped") || !seen.insert(id) {
            continue;
        }
        let note: String = item.get("note").and_then(Value::as_str).unwrap_or_default().chars().take(300).collect();
        out.push(json!({ "id": id, "status": status, "note": note }));
    }
    (!out.is_empty()).then(|| Value::Array(out))
}

// ----- the agent -----

/// What goes with each message: the routine, the plan, and the steps the
/// human picked.
pub fn context(state: &AppState, id: &str, selected: &[String]) -> Result<String, String> {
    let d = doc(state, id)?;
    let r = routine_of(state, id)?;
    let routine = json!({
        "name": r.name,
        "folder": r.cwd,
        "run_agent": r.agent,
        "instruction": r.instruction,
        "instruction_changed_since_plan": d.instruction_changed(&r.instruction),
    });
    let mut out = format!("[routine]\n{routine}\n[plan]\n{}", d.outline());
    if !selected.is_empty() {
        out.push_str(&format!("\nselected: {}", selected.join(", ")));
    }
    Ok(out)
}

/// What the plan's agent is told once, at the start of its session.
pub fn preamble(lang: &str, name: &str) -> String {
    if lang == "ko" {
        return format!(
            r#"당신은 DIVIXI의 계획 파트너입니다. 사람과 함께 되풀이 "{name}"의 계획을 짭니다. 되풀이는 저장해 둔 지시문을 자기 에이전트가 자기 폴더에서 혼자 실행하는 일입니다. 계획은 그 지시문을 단계로 나눈 것이고, 실행할 때 지시문과 함께 한 세션에 묶음(wave) 순서로 전달됩니다. 당신은 계획을 짜는 쪽이지 실행하는 쪽이 아닙니다.

계획:
- 단계마다 title(짧은 이름), instruction(이 단계에서 할 일, 혼자 읽혀도 통하게), after(먼저 끝나야 하는 단계 id 목록), inputs/outputs(쓰는 것과 남기는 것), done_when(끝났는지 알아보는 기준), on_fail(stop 또는 continue)이 있습니다.
- 매 메시지마다 [routine] 아래 되풀이(이름, 폴더, 실행 에이전트, 지시문, 계획 이후 지시문이 바뀌었는지)와 [plan] 아래 계획 개요(단계, 묶음 waves, 대기 중인 제안, 점검 결과 issues)가 JSON으로 옵니다. 사람이 고른 단계는 selected로 옵니다.
- 되풀이 폴더는 읽기 참고용입니다. 계획을 짜는 동안 그 폴더의 파일을 바꾸거나 명령을 실행하지 마세요.

도구:
- `plan_get`: 지금 계획 전체.
- `plan_add_step`: 단계 하나를 만듭니다. 글은 도구의 맨 위 칸에 그대로 씁니다. after로 앞 단계를 바로 정합니다.
- `plan_update_step`: 단계 하나의 글이나 설정을 고칩니다.
- `plan_link`: 한 단계가 다른 단계를 기다리게 합니다. 순환이 생기면 거절되고 그 경로가 돌아옵니다.
- `plan_edit`: 글 없는 명령 묶음(link, unlink, set_after, delete, move, auto_layout). 이 목록에는 한글 같은 비ASCII 글을 넣지 않습니다.
- `plan_check`: 점검 결과와 묶음.
- `plan_propose_instruction`: 계획에 맞춘 되풀이 지시문을 제안합니다. 지시문은 사람이 받아들일 때만 바뀝니다.
- 글(title, instruction, done_when, inputs, outputs)은 글자 그대로 씁니다. `양` 같은 이스케이프로 바꾸지 마세요. 도구는 보드에 담긴 글을 돌려주니, 보낸 것과 맞춰 보고 어긋나면 한 번 다시 씁니다.
- 당신이 바꾼 것은 사람이 유지하거나 되돌리기 전까지 "제안"으로 표시됩니다. 되돌려진 것을 다시 밀어붙이지 마세요.
- 실행에는 사람이 [계획 저장]으로 저장한 마지막 버전이 쓰입니다. 저장은 사람이 합니다.

짜는 법:
- 단계는 작고 확인할 수 있게 나눕니다. 한 단계에 한 가지 일.
- 기본은 순서대로입니다. 서로 기다릴 필요가 정말 없을 때만 나란히 둡니다.
- 마지막 단계는 결과를 확인하고 보고하는 단계로 둡니다.
- 단계는 많아야 {MAX_STEPS}개입니다. 대개 3~8개면 충분합니다.
- 사람에게 단계를 말할 때는 id와 짧은 제목을 함께 씁니다: 예) s3("보안 공지 확인").
- 대화 답은 짧게. 계획에 쓴 것을 대화에서 되풀이하지 않습니다. 한국어로 말합니다.
"#
        );
    }
    format!(
        r#"You are DIVIXI's planning partner. With the human you work out the plan of the routine "{name}". A routine is a saved instruction its own agent runs alone, in its own folder. The plan breaks that instruction into steps; when it runs, its session is given the instruction and the plan together, wave by wave. You make the plan; you do not run it.

The plan:
- Each step has a title (short), an instruction (what to do in it, written to be read on its own), after (ids of the steps that must be done first), inputs/outputs (what it uses and leaves behind), done_when (how to tell it is done) and on_fail (stop or continue).
- Every message brings, as JSON, the routine under [routine] (name, folder, the agent that runs it, its instruction, and whether the instruction changed since the plan was made) and the plan under [plan] (steps, waves, suggestions still pending, issues). Steps the human picked come as selected.
- The routine's folder is there to read for reference. While planning, do not change files in it or run commands there.

Tools:
- `plan_get`: the whole plan as it is.
- `plan_add_step`: make one step. Its words go in the tool's top-level fields as they are; `after` sets the steps it waits for.
- `plan_update_step`: change one step's words or settings.
- `plan_link`: make one step wait for another. A cycle is refused with its path.
- `plan_edit`: wordless commands in a list (link, unlink, set_after, delete, move, auto_layout). No words that are not ASCII in this list.
- `plan_check`: the issues and the waves.
- `plan_propose_instruction`: propose an instruction for the routine that fits the plan. It changes only if the human takes it.
- Write words (title, instruction, done_when, inputs, outputs) as they are, never as escapes such as `양`. The tools answer with the words the plan now holds; read them against what you sent and write once more if they came out wrong.
- What you change shows as a suggestion until the human keeps or reverts it. Do not push back what was reverted.
- Runs are given the version the human last saved with [Save plan]. Saving is the human's.

How to plan:
- Small steps that can be checked; one job per step.
- In order by default. Put steps side by side only when they truly need not wait for each other.
- Make the last step the one that checks the result and reports.
- At most {MAX_STEPS} steps; three to eight is usually enough.
- When you mention a step to the human, give its id with its title, e.g. s3 ("check advisories").
- Keep chat replies short and do not repeat in chat what you wrote in the plan. Speak English.
"#
    )
}

pub const PLAN_GET: &str = "plan_get";
/// The one tool of a run given a plan (routine.rs): where it is in the plan.
pub const PLAN_STEP: &str = "plan_step";
pub const PLAN_ADD_STEP: &str = "plan_add_step";
pub const PLAN_UPDATE_STEP: &str = "plan_update_step";
pub const PLAN_LINK: &str = "plan_link";
pub const PLAN_EDIT: &str = "plan_edit";
pub const PLAN_CHECK: &str = "plan_check";
pub const PLAN_PROPOSE_INSTRUCTION: &str = "plan_propose_instruction";

/// The plan's own tools, the only thing its agent is let do.
const OWN_TOOLS: &[&str] = &[PLAN_GET, PLAN_ADD_STEP, PLAN_UPDATE_STEP, PLAN_LINK, PLAN_EDIT, PLAN_CHECK, PLAN_PROPOSE_INSTRUCTION];

/// Whether a permission question from a plan's agent is about one of its
/// own plan tools (allowed) rather than anything else it could do: write a
/// file, run a command (refused). Read from the question's title and input,
/// which name the tool as each agent writes it: `mcp__divixi__plan_add_step`
/// (Claude Code), `mcp.divixi.plan_add_step` (Codex), the bare name.
pub fn is_own_tool(title: &str, input: &str) -> bool {
    let named = |text: &str| {
        OWN_TOOLS.iter().any(|t| {
            let text = text.trim();
            text == *t || text.contains(&format!("divixi__{t}")) || text == format!("mcp.divixi.{t}") || text.starts_with(&format!("mcp.divixi.{t}("))
        })
    };
    named(title) || named(input)
}

/// The mode a plan's agent works in: one that asks before it acts, so that
/// every act but its plan tools reaches [`is_own_tool`] and is refused.
/// Among `(id, name)` modes, the first that asks: the agent's own default
/// or manual, ask, then read-only. None when it offers none of these.
pub fn asking_mode(modes: &[(String, String)]) -> Option<String> {
    let key = |s: &str| s.rsplit(['#', '/']).next().unwrap_or(s).to_ascii_lowercase();
    const ASKING: [&str; 6] = ["default", "manual", "ask", "read-only", "readonly", "read_only"];
    // By id first: a name is the agent's to word ("Default" may well be a
    // mode that writes unasked).
    ASKING
        .iter()
        .find_map(|want| modes.iter().find(|(id, _)| key(id) == *want).map(|(id, _)| id.clone()))
        .or_else(|| ASKING.iter().find_map(|want| modes.iter().find(|(_, name)| key(name) == *want).map(|(id, _)| id.clone())))
}

const WORDS: &str = "Plain text, every character as itself (Korean, emoji, accents, CJK). Never \\uXXXX escapes.";

fn step_fields(id_required: bool) -> Value {
    let mut props = json!({
        "title": { "type": "string", "description": WORDS },
        "instruction": { "type": "string", "description": WORDS },
        "inputs": { "type": "array", "items": { "type": "string" }, "description": WORDS },
        "outputs": { "type": "array", "items": { "type": "string" }, "description": WORDS },
        "done_when": { "type": "string", "description": WORDS },
        "agent": { "type": "string", "description": "An agent id for this step; leave out for the routine's own." },
        "on_fail": { "type": "string", "enum": ["stop", "continue"] }
    });
    if id_required {
        props["id"] = json!({ "type": "string" });
        json!({ "type": "object", "properties": props, "required": ["id"], "additionalProperties": false })
    } else {
        props["after"] = json!({ "type": "array", "items": { "type": "string" }, "description": "Ids of the steps it waits for." });
        json!({ "type": "object", "properties": props, "required": ["title", "instruction"], "additionalProperties": false })
    }
}

fn edit_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "commands": {
                "type": "array",
                "description": "Applied in order: link {from, to}; unlink {from, to}; set_after {id, after}; delete {ids}; move {id, x, y}; auto_layout {}.",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["link", "unlink", "set_after", "delete", "move", "auto_layout"] },
                        "id": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "after": { "type": "array", "items": { "type": "string" } },
                        "ids": { "type": "array", "items": { "type": "string" } },
                        "x": { "type": "number" },
                        "y": { "type": "number" }
                    },
                    "required": ["op"]
                }
            }
        },
        "required": ["commands"],
        "additionalProperties": false
    })
}

/// Every string anywhere in a value.
fn strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// Refuse words that still hold an escape written out as text (see
/// `design::literal_escape`): such a write would keep other letters.
pub fn check_escapes(args: &Value) -> Result<(), String> {
    let mut all = Vec::new();
    strings(args, &mut all);
    for s in all {
        if let Some(found) = crate::design::literal_escape(&s) {
            return Err(format!(
                "the words hold the escape {found} as text. Nothing was written. Write them again with every character as itself (Korean, emoji, accents), not \\uXXXX escapes."
            ));
        }
    }
    Ok(())
}

/// `plan_edit` takes ids and numbers only: words inside its list are the
/// ones that come out escaped.
pub fn check_ascii(args: &Value) -> Result<(), String> {
    let mut all = Vec::new();
    strings(args, &mut all);
    if all.iter().any(|s| !s.is_ascii()) {
        return Err("plan_edit takes step ids and numbers only, and these commands hold words that are not ASCII. Nothing was written. Change a step's words with plan_update_step, one step per call.".to_string());
    }
    Ok(())
}

/// One op from a tool's top-level fields, as the window would send it.
fn single(op: &str, args: &Value) -> Result<Op, String> {
    let mut v = args.clone();
    if !v.is_object() {
        v = json!({});
    }
    v["op"] = json!(op);
    serde_json::from_value(v).map_err(|e| format!("bad {op}: {e}"))
}

/// What the agent's `plan_edit` may say: the wordless commands only.
pub fn parse_edit(args: &Value) -> Result<Vec<Op>, String> {
    let raw = args.get("commands").cloned().unwrap_or(Value::Null);
    let ops: Vec<Op> = serde_json::from_value(raw).map_err(|e| format!("bad commands: {e}"))?;
    for op in &ops {
        if matches!(op, Op::AddStep { .. } | Op::UpdateStep { .. }) {
            return Err("plan_edit does not write words: make and change steps with plan_add_step and plan_update_step".to_string());
        }
    }
    Ok(ops)
}

/// The plan agent's tools, scoped to one plan.
pub fn tools(app: AppHandle, id: String) -> Vec<Tool> {
    // One handle for every tool, each cloning what it needs.
    let at = (app, id);
    let run = |at: &(AppHandle, String), op: Op| {
        let (app, id) = at.clone();
        async move {
            let turn = crate::artifact::running(&app, &id).await;
            let mut results = apply(&app, &id, vec![op], Actor::Agent { run: turn })?;
            let first = results.pop().unwrap_or(Value::Null);
            if first.get("ok") == Some(&Value::Bool(false)) {
                return Err(first.get("error").and_then(Value::as_str).unwrap_or("refused").to_string());
            }
            Ok(first)
        }
    };

    let get = at.clone();
    let check = at.clone();
    let add = at.clone();
    let update = at.clone();
    let link = at.clone();
    let editing = at.clone();
    let proposing = at;
    vec![
        Tool::new(
            PLAN_GET,
            "Read the plan: every step (id, title, instruction, after, inputs, outputs, done_when, on_fail, who made it), the waves it runs in, the suggestions still waiting on the human, issues worth a look, and any instruction you proposed.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_| {
                let (app, id) = get.clone();
                async move { Ok(doc(&app.state::<AppState>(), &id)?.outline()) }
            },
        ),
        Tool::new(
            PLAN_CHECK,
            "Check the plan: the waves its steps run in, and issues worth a look (a step with no instruction or no way to tell it is done, an input another step makes that this one does not wait for).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_| {
                let (app, id) = check.clone();
                async move {
                    let d = doc(&app.state::<AppState>(), &id)?;
                    Ok(json!({ "waves": d.waves(), "issues": d.issues(), "steps": d.steps.len() }))
                }
            },
        ),
        Tool::new(
            PLAN_ADD_STEP,
            "Make one step. Its words are this tool's own fields, so every character arrives as itself. `after` lists the steps it waits for. Answers with the step's id and the words the plan now holds: read them against what you sent. Shows as a suggestion the human keeps or reverts.",
            step_fields(false),
            move |args| {
                let at = add.clone();
                async move {
                    check_escapes(&args)?;
                    run(&at, single("add_step", &args)?).await
                }
            },
        ),
        Tool::new(
            PLAN_UPDATE_STEP,
            "Change one step's words or settings; fields left out stay as they are. Answers with the words the plan now holds. Shows as a suggestion the human keeps or reverts.",
            step_fields(true),
            move |args| {
                let at = update.clone();
                async move {
                    check_escapes(&args)?;
                    run(&at, single("update_step", &args)?).await
                }
            },
        ),
        Tool::new(
            PLAN_LINK,
            "Make step `to` wait for step `from`. Refused, with the path, when it would make a cycle.",
            json!({
                "type": "object",
                "properties": { "from": { "type": "string" }, "to": { "type": "string" } },
                "required": ["from", "to"],
                "additionalProperties": false
            }),
            move |args| {
                let at = link.clone();
                async move {
                    check_ascii(&args)?;
                    run(&at, single("link", &args)?).await
                }
            },
        ),
        Tool::new(
            PLAN_EDIT,
            "Change the plan with wordless commands, applied in order: link {from, to}; unlink {from, to}; set_after {id, after}; delete {ids}; move {id, x, y}; auto_layout {}. Ids and numbers only: words go through plan_add_step and plan_update_step. Each result says ok, or the error. Your changes show as suggestions.",
            edit_schema(),
            move |args| {
                let (app, id) = editing.clone();
                async move {
                    check_ascii(&args)?;
                    let ops = parse_edit(&args)?;
                    let turn = crate::artifact::running(&app, &id).await;
                    Ok(json!({ "results": apply(&app, &id, ops, Actor::Agent { run: turn })? }))
                }
            },
        ),
        Tool::new(
            PLAN_PROPOSE_INSTRUCTION,
            "Propose a new instruction for the routine, one that fits the plan. The routine's instruction changes only if the human takes it; a later proposal replaces this one.",
            json!({
                "type": "object",
                "properties": { "text": { "type": "string", "description": WORDS } },
                "required": ["text"],
                "additionalProperties": false
            }),
            move |args| {
                let (app, id) = proposing.clone();
                async move {
                    check_escapes(&args)?;
                    let text = args.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        return Err("an instruction needs words".to_string());
                    }
                    check_len(&text, MAX_PROPOSAL, "a proposed instruction")?;
                    let turn = crate::artifact::running(&app, &id).await;
                    propose(&app, &id, text, turn)?;
                    Ok(json!({ "ok": true, "note": "Proposed. The human sees it beside the plan and decides." }))
                }
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(v: Value) -> Vec<Op> {
        serde_json::from_value(v).unwrap()
    }

    fn agent() -> Actor {
        Actor::Agent { run: Some("t001".into()) }
    }

    /// s1 -> s2, s1 -> s3, (s2, s3) -> s4, made by the human.
    fn diamond() -> Doc {
        let mut d = Doc::default();
        let r = d.apply(
            ops(json!([
                { "op": "add_step", "title": "pull", "instruction": "git pull", "done_when": "clean" },
                { "op": "add_step", "title": "deps", "instruction": "list deps", "after": ["s1"], "outputs": ["deps.json"], "done_when": "listed" },
                { "op": "add_step", "title": "audit", "instruction": "audit", "after": ["s1"], "outputs": ["audit.json"], "done_when": "done" },
                { "op": "add_step", "title": "report", "instruction": "merge", "after": ["s2", "s3"], "inputs": ["deps.json", "audit.json"], "done_when": "a table" }
            ])),
            &Actor::Human,
        );
        assert!(r.iter().all(|x| x["ok"] == true), "{r:?}");
        d.journal.clear();
        d
    }

    #[test]
    fn waves_follow_the_longest_way_back() {
        let d = diamond();
        assert_eq!(d.waves(), vec![vec!["s1"], vec!["s2", "s3"], vec!["s4"]]);
        assert!(d.issues().is_empty(), "{:?}", d.issues());
    }

    #[test]
    fn a_cycle_is_refused_with_its_path() {
        let mut d = diamond();
        let r = d.apply(ops(json!([{ "op": "link", "from": "s4", "to": "s1" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
        let err = r[0]["error"].as_str().unwrap();
        assert!(err.contains("cycle") && err.contains("s1") && err.contains("s4"), "{err}");
        assert!(d.step("s1").unwrap().after.is_empty(), "nothing was written");

        let r = d.apply(ops(json!([{ "op": "set_after", "id": "s2", "after": ["s2"] }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "a step cannot wait for itself");
        let r = d.apply(ops(json!([{ "op": "set_after", "id": "s2", "after": ["s9"] }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "nor for a step that is not there");
    }

    #[test]
    fn link_and_unlink_change_one_steps_wait_list() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "unlink", "from": "s2", "to": "s4" }])), &Actor::Human);
        assert_eq!(d.step("s4").unwrap().after, vec!["s3"]);
        d.apply(ops(json!([{ "op": "link", "from": "s2", "to": "s4" }, { "op": "link", "from": "s2", "to": "s4" }])), &Actor::Human);
        assert_eq!(d.step("s4").unwrap().after, vec!["s3", "s2"], "linking twice is once");
    }

    #[test]
    fn deleting_a_step_takes_it_out_of_every_wait_list() {
        let mut d = diamond();
        let r = d.apply(ops(json!([{ "op": "delete", "ids": ["s2", "s9"] }])), &Actor::Human);
        assert_eq!(r[0]["deleted"], json!(["s2"]));
        assert_eq!(r[0]["missing"], json!(["s9"]));
        assert_eq!(d.step("s4").unwrap().after, vec!["s3"]);
        assert_eq!(d.waves(), vec![vec!["s1"], vec!["s3"], vec!["s4"]]);
    }

    #[test]
    fn agent_changes_are_suggestions_the_human_keeps_or_reverts() {
        let mut d = diamond();
        let r = d.apply(
            ops(json!([
                { "op": "add_step", "title": "majors", "instruction": "pick majors", "after": ["s2"] },
                { "op": "set_after", "id": "s4", "after": ["s3", "s5"] }
            ])),
            &agent(),
        );
        assert!(r.iter().all(|x| x["ok"] == true), "{r:?}");
        assert_eq!(d.changes.len(), 2);
        assert_eq!(d.step("s5").unwrap().by, "agent");

        // Reverting both puts s4 back to waiting for s2 and s3, and s5 is gone.
        let all: Vec<u64> = d.changes.iter().map(|c| c.id).collect();
        d.review(&all, false);
        assert!(d.step("s5").is_none());
        assert_eq!(d.step("s4").unwrap().after, vec!["s2", "s3"]);
        assert!(d.changes.is_empty());
    }

    #[test]
    fn reverting_a_new_step_alone_leaves_no_wait_on_it() {
        let mut d = diamond();
        d.apply(
            ops(json!([
                { "op": "add_step", "title": "majors", "instruction": "pick majors", "after": ["s2"] },
                { "op": "link", "from": "s5", "to": "s4" }
            ])),
            &agent(),
        );
        let made = d.changes.iter().find(|c| c.target == "s5").unwrap().id;
        d.review(&[made], false);
        assert!(d.step("s5").is_none());
        assert_eq!(d.step("s4").unwrap().after, vec!["s2", "s3"], "the wait on the step that went is gone too");
    }

    #[test]
    fn a_human_edit_to_a_suggested_step_keeps_it() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "advisories" }])), &agent());
        assert_eq!(d.changes.len(), 1);
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "done_when": "counted by severity" }])), &Actor::Human);
        assert!(d.changes.is_empty(), "touching a suggestion keeps it");
        assert_eq!(d.step("s3").unwrap().title, "advisories");
    }

    #[test]
    fn an_agent_edit_put_back_as_it_was_leaves_nothing_to_review() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "advisories" }])), &agent());
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "audit" }])), &agent());
        assert!(d.changes.is_empty());
    }

    #[test]
    fn the_humans_only_ops_are_refused_to_the_agent() {
        let mut d = diamond();
        for op in [json!({ "op": "save" }), json!({ "op": "open", "n": 1 }), json!({ "op": "dismiss_proposal" })] {
            let r = d.apply(ops(json!([op])), &agent());
            assert_eq!(r[0]["ok"], false, "{r:?}");
        }
        assert_eq!(d.steps.len(), 4);
        assert!(d.saved.is_empty());
    }

    #[test]
    fn undo_takes_back_the_human_edit_and_leaves_the_agents() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "update_step", "id": "s2", "title": "dependencies" }])), &Actor::Human);
        let step = d.take_step().unwrap();
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "advisories" }])), &agent());
        d.replay(&step, false);
        assert_eq!(d.step("s2").unwrap().title, "deps");
        assert_eq!(d.step("s3").unwrap().title, "advisories", "the agent's later work stays");
        d.replay(&step, true);
        assert_eq!(d.step("s2").unwrap().title, "dependencies");
    }

    #[test]
    fn crossing_edits_reverted_leave_no_cycle() {
        // s2 waits for s3 (the human's). The agent takes that wait away; the
        // human then makes s3 wait for s2, which is allowed now. Reverting
        // the agent's suggestion would bring back s2 -> s3 and close a cycle:
        // the repair drops one of the two waits instead.
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "link", "from": "s3", "to": "s2" }])), &Actor::Human);
        d.apply(ops(json!([{ "op": "set_after", "id": "s2", "after": ["s1"] }])), &agent());
        let id = d.changes[0].id;
        let r = d.apply(ops(json!([{ "op": "link", "from": "s2", "to": "s3" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], true, "{r:?}");
        d.review(&[id], false);
        let a2 = d.step("s2").unwrap().after.clone();
        let a3 = d.step("s3").unwrap().after.clone();
        assert!(!(a2.contains(&"s3".to_string()) && a3.contains(&"s2".to_string())), "s2 {a2:?} s3 {a3:?}");
        assert_eq!(d.waves().iter().map(Vec::len).sum::<usize>(), 4, "every step still has a wave");
        assert_eq!(d.waves().len(), 4, "s1, then s2, then s3, then s4");
    }

    #[test]
    fn bounds_hold() {
        let mut d = Doc::default();
        for i in 0..MAX_STEPS {
            let r = d.apply(ops(json!([{ "op": "add_step", "title": format!("step {i}"), "instruction": "x" }])), &Actor::Human);
            assert_eq!(r[0]["ok"], true);
        }
        let r = d.apply(ops(json!([{ "op": "add_step", "title": "one more", "instruction": "x" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
        let too_long = "가".repeat(MAX_TITLE + 1);
        let r = d.apply(ops(json!([{ "op": "update_step", "id": "s1", "title": too_long }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
        let r = d.apply(ops(json!([{ "op": "add_step", "title": " ", "instruction": "x" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "a step needs a title");
        let r = d.apply(ops(json!([{ "op": "update_step", "id": "s1", "agent": "rm -rf /" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "an agent is an id");
    }

    #[test]
    fn saving_makes_versions_and_a_version_comes_back_into_the_working_plan() {
        let mut d = diamond();
        assert!(d.unsaved(), "a plan never saved has unsaved changes");
        let r = d.apply(ops(json!([{ "op": "save", "note": "first" }])), &Actor::Human);
        assert_eq!(r[0]["version"], 1);
        assert!(!d.unsaved());
        let r = d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "nothing to save");

        // Moving a card is not a change a run would see.
        d.apply(ops(json!([{ "op": "move", "id": "s2", "x": 10, "y": 20 }])), &Actor::Human);
        assert!(!d.unsaved());

        d.apply(ops(json!([{ "op": "update_step", "id": "s2", "title": "dependencies" }, { "op": "delete", "ids": ["s3"] }])), &Actor::Human);
        assert!(d.unsaved());
        d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(d.saved.iter().map(|v| v.n).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(d.saved[0].note, "first");

        // Version 1 back into the working plan: s3 returns, s2 has its old title.
        d.journal.clear();
        let r = d.apply(ops(json!([{ "op": "open", "n": 1 }])), &Actor::Human);
        assert_eq!(r[0]["ok"], true, "{r:?}");
        assert_eq!(d.step("s2").unwrap().title, "deps");
        assert!(d.step("s3").is_some());
        assert_eq!(d.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "s2", "s3", "s4"]);
        assert!(d.unsaved(), "the working plan is v1's, but runs are given v2: it differs from what a run would get");
        assert_eq!(d.run_version().map(|v| v.n), Some(2), "opening a version does not change what runs are given");
        assert_eq!(d.opened, Some(1));
        assert_eq!(d.saved.len(), 2, "opening a version saves nothing by itself");
        let step = d.take_step().unwrap();
        assert!(!step.is_empty(), "bringing a version back is an edit that can be undone");
        let r = d.apply(ops(json!([{ "op": "add_step", "title": "new", "instruction": "x" }])), &Actor::Human);
        assert_eq!(r[0]["id"], "s5", "ids are not reused");
        let r = d.apply(ops(json!([{ "op": "open", "n": 9 }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
    }

    #[test]
    fn saving_over_a_version_keeps_its_number_and_runs_keep_what_they_were_given() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "save", "note": "first" }])), &Actor::Human);
        let given = guidance(&d).unwrap().snapshot; // a run that went with v1
        let first_words = d.step("s2").unwrap().instruction.clone();
        assert_eq!(d.opened, Some(1));

        d.apply(ops(json!([{ "op": "update_step", "id": "s2", "instruction": "list direct deps only" }])), &Actor::Human);
        assert!(d.unsaved());
        let r = d.apply(ops(json!([{ "op": "save", "over": 1 }])), &Actor::Human);
        assert_eq!(r[0]["version"], 1, "{r:?}");
        assert_eq!(d.saved.len(), 1, "no new version");
        assert!(d.saved[0].edited_at.is_some(), "the time it was saved over is kept");
        assert_eq!(d.saved[0].note, "first", "no new note keeps the old one");
        assert!(!d.unsaved());
        assert_eq!(given["steps"][1]["title"], "deps", "the earlier run's copy is its own");
        assert_eq!(given["steps"][1]["instruction"], json!(first_words), "every word of it: saving over v1 does not rewrite what that run was given");
        assert!(given["steps"][1].get("x").is_none(), "but not where the card sat");
        assert_eq!(guidance(&d).unwrap().snapshot["version"], 1);
        assert!(guidance(&d).unwrap().section.contains("list direct deps only"), "the next run gets the new words");
        let r = d.apply(ops(json!([{ "op": "save", "over": 1 }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false, "nothing to save over with");

        // A new version beside it; then v1 opened, edited and saved over again.
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "advisories" }, { "op": "save", "note": "second" }])), &Actor::Human);
        assert_eq!((d.saved.len(), d.opened), (2, Some(2)));
        d.apply(ops(json!([{ "op": "open", "n": 1 }])), &Actor::Human);
        assert_eq!(d.opened, Some(1));
        assert!(d.unsaved(), "back at v1, while runs are given v2");
        assert_eq!(d.gate(), Gate::AskUnsaved);
        d.apply(ops(json!([{ "op": "update_step", "id": "s1", "title": "fetch" }, { "op": "save", "over": 1, "note": "redo" }])), &Actor::Human);
        assert_eq!(d.saved[0].note, "redo");
        assert_eq!(d.saved[1].steps[0].title, "pull", "v2 untouched");
        // Saved over, v1 is what runs are given now, though v2 is newer.
        assert_eq!(d.run_version().map(|v| v.n), Some(1));
        assert!(!d.unsaved());
        assert_eq!(d.gate(), Gate::Run);
        assert_eq!(guidance(&d).unwrap().snapshot["version"], 1);
        assert!(guidance(&d).unwrap().section.contains("\"fetch\""));
        assert_eq!(d.outline()["saved_version"], 1);
        // A new version is the run version again.
        d.apply(ops(json!([{ "op": "update_step", "id": "s4", "title": "notes" }, { "op": "save" }])), &Actor::Human);
        assert_eq!(d.run_version().map(|v| v.n), Some(3));
        let r = d.apply(ops(json!([{ "op": "save", "over": 7 }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
    }

    #[test]
    fn a_run_no_one_can_be_asked_about_is_refused_while_the_plan_is_not_saved() {
        // No plan: runs.
        assert_eq!(run_refusal(&Doc::default(), true), None);
        // A plan never saved: refused, saying to save it.
        let mut d = diamond();
        let why = run_refusal(&d, true).unwrap();
        assert!(why.contains("저장되지 않았습니다") && why.contains("[계획 저장]"), "{why}");
        assert!(run_refusal(&d, false).unwrap().contains("not saved"));
        // Saved: runs.
        d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(run_refusal(&d, true), None);
        // A card moved is no change.
        d.apply(ops(json!([{ "op": "move", "id": "s2", "x": 5, "y": 5 }])), &Actor::Human);
        assert_eq!(run_refusal(&d, true), None);
        // Changed since: refused, naming the version runs are given.
        d.apply(ops(json!([{ "op": "update_step", "id": "s2", "title": "dependencies" }])), &Actor::Human);
        let why = run_refusal(&d, true).unwrap();
        assert!(why.contains("v1") && why.contains("저장하지 않은 변경"), "{why}");
        // Suggestions waiting: refused for them, as the window's Run is.
        d.apply(ops(json!([{ "op": "add_step", "title": "x", "instruction": "y" }])), &agent());
        let why = run_refusal(&d, true).unwrap();
        assert!(why.contains("제안이 1건"), "{why}");
        assert!(run_refusal(&d, false).unwrap().contains("1 suggestion(s)"));
    }

    #[test]
    fn a_plans_agent_may_use_its_plan_tools_and_nothing_else() {
        assert!(is_own_tool("mcp__divixi__plan_add_step", ""));
        assert!(is_own_tool("Tool call", r#"{"tool":"mcp__divixi__plan_update_step","id":"s1"}"#));
        assert!(is_own_tool("plan_get", ""));
        assert!(!is_own_tool("Write", r#"{"file_path":"/w/plan_get.txt"}"#), "a file that only mentions a tool's name");
        assert!(!is_own_tool("Bash", "rm -rf build"));
        assert!(!is_own_tool("mcp__divixi__plan_step", ""), "a run's tool is not the plan agent's");
        assert!(!is_own_tool("mcp__other__plan_add_step", ""), "another server's tool of the same name");
        assert!(is_own_tool("mcp.divixi.plan_add_step", ""), "as Codex names it");
        assert!(!is_own_tool("mcp.other.plan_add_step", ""));
        assert!(!is_own_tool("echo mcp.divixi.plan_add_step > x.txt", ""), "a command that only mentions it");
    }

    #[test]
    fn a_plans_agent_works_in_a_mode_that_asks() {
        let claude = [("default", "Manual"), ("acceptEdits", "Accept edits"), ("plan", "Plan"), ("bypassPermissions", "Bypass permissions")]
            .map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(asking_mode(&claude).as_deref(), Some("default"));
        let codex = [("read-only", "Read Only"), ("auto", "Default"), ("full-access", "Full Access")].map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(asking_mode(&codex).as_deref(), Some("read-only"), "by id: a mode named Default may write unasked");
        let named = [("m1", "Ask"), ("m2", "Full")].map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(asking_mode(&named).as_deref(), Some("m1"), "by name when no id says");
        let only_bold = [("yolo", "Yolo")].map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(asking_mode(&only_bold), None);
    }

    /// The window's Run and the conductor's `run_routine` answer by one rule;
    /// these are the cases ui/src/lib/planGraph.test.ts checks `runGate` on.
    #[test]
    fn the_gate_is_one_rule_for_the_window_and_the_conductor() {
        let empty = Doc::default();
        assert_eq!(empty.gate(), Gate::Run, "no plan");
        let mut d = diamond();
        assert_eq!(d.gate(), Gate::AskSave, "steps never saved");
        d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(d.gate(), Gate::Run, "the version runs are given");
        // Every step taken off the working plan, not saved: v1 would still run.
        d.apply(ops(json!([{ "op": "delete", "ids": ["s1", "s2", "s3", "s4"] }])), &Actor::Human);
        assert!(d.steps.is_empty());
        assert_eq!(d.gate(), Gate::AskUnsaved, "an emptied working plan is still not what runs get");
        assert!(run_refusal(&d, true).is_some(), "so the conductor does not run v1 behind it");
        d.journal.clear();
        d.apply(ops(json!([{ "op": "open", "n": 1 }])), &Actor::Human);
        assert_eq!(d.gate(), Gate::Run);
        // A suggestion waiting settles first, whatever else.
        d.apply(ops(json!([{ "op": "update_step", "id": "s1", "title": "from the agent" }])), &agent());
        assert_eq!(d.gate(), Gate::Settle);
        assert!(run_refusal(&d, true).is_some());
    }

    #[test]
    fn suggestions_waiting_hold_back_saving_and_opening() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "title": "advisories" }])), &agent());
        let r = d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
        assert!(r[0]["error"].as_str().unwrap().contains("suggestion"));
        let r = d.apply(ops(json!([{ "op": "open", "n": 1 }])), &Actor::Human);
        assert_eq!(r[0]["ok"], false);
        // And neither is the agent's to do.
        let all: Vec<u64> = d.changes.iter().map(|c| c.id).collect();
        d.review(&all, true);
        let r = d.apply(ops(json!([{ "op": "save" }])), &agent());
        assert_eq!(r[0]["ok"], false);
        let r = d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        assert_eq!(r[0]["version"], 2);
    }

    #[test]
    fn auto_layout_lets_go_of_every_pin_after_the_order_changed() {
        // The reported case: a card placed by hand, then its steps before
        // changed, then "lay out": no card may stay where the hand put it.
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "move", "id": "s4", "x": 10, "y": 10 }, { "op": "move", "id": "s2", "x": 900, "y": 0 }])), &Actor::Human);
        d.apply(ops(json!([{ "op": "set_after", "id": "s4", "after": ["s3"] }, { "op": "update_step", "id": "s4", "inputs": ["audit.json"] }])), &Actor::Human);
        let r = d.apply(ops(json!([{ "op": "auto_layout" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], true);
        assert!(d.steps.iter().all(|s| s.x.is_none() && s.y.is_none()), "{:?}", d.steps);
        assert_eq!(d.step("s4").unwrap().after, vec!["s3"], "the order it was given stays");
        assert_eq!(d.step("s4").unwrap().inputs, vec!["audit.json"]);
        // The window sees the steps without positions: the serialized step has none.
        let wire = serde_json::to_value(d.step("s4").unwrap()).unwrap();
        assert!(wire.get("x").is_none() && wire.get("y").is_none(), "{wire}");
    }

    #[test]
    fn issues_point_at_what_is_probably_not_meant() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "unlink", "from": "s2", "to": "s4" }, { "op": "update_step", "id": "s3", "done_when": "" }])), &Actor::Human);
        let issues = d.issues();
        assert!(issues.iter().any(|i| i.contains("s4") && i.contains("deps.json") && i.contains("s2")), "{issues:?}");
        assert!(issues.iter().any(|i| i.starts_with("s3 does not say")), "{issues:?}");
    }

    #[test]
    fn the_instruction_moving_on_is_noticed() {
        let mut d = Doc { source: Some(Source { instruction: "check deps".into(), at: 1 }), ..Doc::default() };
        assert!(!d.instruction_changed(" check deps\n"));
        assert!(d.instruction_changed("check deps and advisories"));
        d.apply(ops(json!([{ "op": "restamp", "instruction": "check deps and advisories" }])), &Actor::Human);
        assert!(!d.instruction_changed("check deps and advisories"));
    }

    /// Korean, emoji, a combining mark: through a step, its outline, the
    /// stored JSON and back, unchanged.
    #[test]
    fn words_go_round_the_plan_unchanged() {
        let words = ["저장소 갱신: 양쪽 곳 비교", "졸업생·뷁·똠얌꿍 🎨 e\u{301} 漢字"];
        let call: Value = serde_json::from_str(
            &json!({ "title": words[0], "instruction": words[1], "outputs": [words[0]] }).to_string(),
        )
        .unwrap();
        check_escapes(&call).unwrap();
        let mut d = Doc::default();
        let r = d.apply(vec![single("add_step", &call).unwrap()], &agent());
        assert_eq!(r[0]["title"], words[0]);
        assert_eq!(r[0]["instruction"], words[1]);
        let saved = serde_json::to_string(&d).unwrap();
        assert!(!saved.contains("\\u"), "{saved}");
        let back: Doc = serde_json::from_str(&saved).unwrap();
        assert_eq!(back.steps[0].instruction, words[1]);
        assert!(back.outline().to_string().contains(words[1]));
    }

    #[test]
    fn escapes_and_words_in_the_wordless_list_are_refused() {
        assert!(check_escapes(&json!({ "title": "\\uC591\\uCABD" })).is_err());
        assert!(check_escapes(&json!({ "inputs": ["ok", "\\uBE44\\uAD50"] })).is_err(), "inside a list too");
        assert!(check_escapes(&json!({ "title": "양쪽" })).is_ok());
        assert!(check_ascii(&json!({ "commands": [{ "op": "link", "from": "s1", "to": "s2" }] })).is_ok());
        assert!(check_ascii(&json!({ "commands": [{ "op": "delete", "ids": ["단계"] }] })).is_err());
        assert!(parse_edit(&json!({ "commands": [{ "op": "add_step", "title": "x", "instruction": "y" }] })).is_err());
        assert!(parse_edit(&json!({ "commands": [{ "op": "link", "from": "s1", "to": "s2" }, { "op": "auto_layout" }] })).is_ok());
    }

    #[test]
    fn an_old_or_empty_body_reads() {
        let d: Doc = serde_json::from_str("{}").unwrap();
        assert!(d.steps.is_empty());
        // A step that still carries an approval (taken out) reads; the value is ignored.
        let kept: Doc = serde_json::from_str(r#"{"steps": [{"id": "s1", "title": "t", "approval": "ask"}]}"#).unwrap();
        assert_eq!(kept.steps[0].id, "s1");
        assert!(!serde_json::to_string(&kept).unwrap().contains("approval"), "and is not written back");
        let mut d2 = Doc::default();
        let r = d2.apply(ops(json!([{ "op": "add_step", "title": "x", "instruction": "y", "approval": "ask" }])), &Actor::Human);
        assert_eq!(r[0]["ok"], true, "an edit naming it is still taken");
        // A body from before the switch was taken out still reads; the field is ignored.
        let old: Doc = serde_json::from_str(r#"{"use_in_run": false, "steps": []}"#).unwrap();
        assert!(old.steps.is_empty());
        let r = Routine {
            id: "ro001".into(),
            name: "deps".into(),
            instruction: "check".into(),
            cwd: "/w".into(),
            agent: "codex".into(),
            config: Default::default(),
            created_at: 1,
            updated_at: 1,
            runs: 0,
            last_run: None,
            last_run_at: None,
            plan: String::new(),
            color: String::new(),
            tags: Vec::new(),
        };
        let body: Doc = serde_json::from_str(&new_body(&r).unwrap()).unwrap();
        assert_eq!(body.routine, "ro001");
        assert_eq!(body.source.unwrap().instruction, "check");
    }

    #[test]
    fn a_run_is_given_the_plan_wave_by_wave() {
        let mut d = diamond();
        d.apply(ops(json!([{ "op": "update_step", "id": "s3", "on_fail": "continue" }])), &Actor::Human);
        assert!(guidance(&d).is_none(), "a plan never saved is not given to runs");
        d.apply(ops(json!([{ "op": "save", "note": "v1" }])), &Actor::Human);
        let g = guidance(&d).unwrap();
        let (w1, w2, w3) = (g.section.find("Wave 1").unwrap(), g.section.find("Wave 2 (side by side)").unwrap(), g.section.find("Wave 3").unwrap());
        assert!(w1 < w2 && w2 < w3, "{}", g.section);
        assert!(g.section.contains("- s4 \"report\" (after s2, s3)"), "{}", g.section);
        assert!(!g.section.contains("ask a person"), "steps no longer wait for a person");
        assert!(g.section.contains("carry on"));
        assert!(g.section.contains("\"steps\""), "the report is asked for step results");
        assert!(g.section.contains(PLAN_STEP), "and to say where it is as it goes");
        assert_eq!(g.ids, ["s1", "s2", "s3", "s4"]);
        assert_eq!(g.snapshot["steps"][3]["after"], json!(["s2", "s3"]));
        assert_eq!(g.snapshot["waves"], json!([["s1"], ["s2", "s3"], ["s4"]]));
        assert_eq!(g.snapshot["version"], 1);
        assert_eq!(g.snapshot["note"], "v1");

        // The last saved version is what runs get, not the working plan.
        d.apply(ops(json!([{ "op": "update_step", "id": "s1", "title": "fetch" }, { "op": "add_step", "title": "extra", "instruction": "x" }])), &Actor::Human);
        let g = guidance(&d).unwrap();
        assert!(g.section.contains("s1 \"pull\"") && !g.section.contains("extra"), "{}", g.section);
        assert_eq!(g.ids.len(), 4);
        d.apply(ops(json!([{ "op": "save" }])), &Actor::Human);
        let g = guidance(&d).unwrap();
        assert!(g.section.contains("s1 \"fetch\"") && g.section.contains("extra"));
        assert_eq!(g.snapshot["version"], 2);

        // Nothing to give: no saved version, or one with no steps.
        assert!(guidance(&Doc::default()).is_none());
        let mut emptied = Doc::default();
        emptied.saved.push(Version { n: 1, at: 1, note: String::new(), steps: Vec::new(), edited_at: None });
        assert!(guidance(&emptied).is_none());
        // An old switch left off in a stored body changes nothing: the last saved version is given.
        let old: Doc = serde_json::from_value(json!({ "use_in_run": false, "steps": [], "saved": serde_json::to_value(&d.saved).unwrap() })).unwrap();
        assert!(guidance(&old).is_some());
    }

    #[test]
    fn step_results_are_read_from_the_report_block_and_kept_to_the_plan() {
        let ids: Vec<String> = ["s1", "s2", "s3"].iter().map(|s| s.to_string()).collect();
        let reply = "done.\n\n```divixi-report\n{\"status\":\"done\",\"summary\":\"ok\",\"steps\":[\
            {\"id\":\"s1\",\"status\":\"done\"},\
            {\"id\":\"s2\",\"status\":\"failed\",\"note\":\"no network\"},\
            {\"id\":\"s2\",\"status\":\"done\"},\
            {\"id\":\"s9\",\"status\":\"done\"},\
            {\"id\":\"s3\",\"status\":\"maybe\"}]}\n```";
        let got = step_results(reply, &ids).unwrap();
        assert_eq!(got, json!([{ "id": "s1", "status": "done", "note": "" }, { "id": "s2", "status": "failed", "note": "no network" }]));
        assert!(step_results("no block here", &ids).is_none());
        assert!(step_results("```divixi-report\n{\"status\":\"done\",\"summary\":\"ok\"}\n```", &ids).is_none(), "a report without steps keeps none");
    }

    #[test]
    fn the_tools_are_the_ones_the_preamble_names() {
        for lang in ["ko", "en"] {
            let p = preamble(lang, "deps");
            for name in [PLAN_GET, PLAN_ADD_STEP, PLAN_UPDATE_STEP, PLAN_LINK, PLAN_EDIT, PLAN_CHECK, PLAN_PROPOSE_INSTRUCTION] {
                assert!(p.contains(name), "{lang} preamble names {name}");
            }
        }
    }
}
