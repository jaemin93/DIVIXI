//! Designs: a sketch board the human and an agent rough out together, kept
//! as an artifact beside tracks and attached to a track's conductor as
//! reference.
//!
//! After Microsoft's Huabu: the board is notes, freehand ink and arrows; the
//! agent reads it as an outline (and sketches as a picture sent with each
//! message) and changes it through one command tool. Its changes land on the
//! board at once and stay marked as suggestions until the human keeps or
//! reverts them; a human edit to a suggested item keeps it.
//!
//! The board lives here, in the core, so the agent's tool calls and the
//! human's edits go through the same `apply`; every change is saved (as the
//! artifact's body) and sent to the window as a `design` event.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orchestra_mcp::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// The artifact kind this module serves.
pub const KIND: &str = "design";

// ----- the board -----

/// One freehand stroke, in its sketch's own coordinates: `[x, y, pressure]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f64; 3]>,
    #[serde(default)]
    pub color: String,
    #[serde(default = "default_stroke")]
    pub size: f64,
}

fn default_stroke() -> f64 {
    4.0
}

/// What a board item is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A card of text.
    #[default]
    Note,
    /// Freehand ink.
    Sketch,
    /// A reference file the human brought: an image, a PDF, a document.
    File,
    /// A web page, by its address.
    Link,
    /// A titled area that groups what lies inside it.
    Frame,
    /// An open question, with the human's answer under it.
    Question,
}

/// One item on the board. Position and size are in board units.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub text: String,
    /// What the note is to the design: `goal`, `constraint`, `question`,
    /// `idea`, or empty. Goals, constraints and open questions are what a
    /// track is made from.
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    /// `human` or `agent`: who made it.
    #[serde(default)]
    pub by: String,
    /// A file's place in the design's folder (`files/<name>`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub src: String,
    /// A file's original name, or a link's page title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// A file's media type.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    /// A link's address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// A question's answer, the human's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
}

/// An arrow from one item to another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub by: String,
}

/// A board item as a change remembers it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entity {
    Node(Node),
    Edge(Edge),
}

/// An agent change waiting for the human: what the item was (none when the
/// agent made it) and what it is now (none when the agent removed it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: u64,
    /// The item it is about.
    pub target: String,
    /// The turn that made it.
    #[serde(default)]
    pub run: Option<String>,
    pub before: Option<Entity>,
    pub after: Option<Entity>,
}

/// The whole board.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Agent changes the human has not kept or reverted yet.
    #[serde(default)]
    pub changes: Vec<Change>,
    /// Counter for item and change ids.
    #[serde(default)]
    pub next: u64,
}

/// One edit to the board. The window and the agent speak the same set; the
/// agent cannot draw ink.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A new note. `ref` names it for later commands in the same call (`$ref`).
    CreateNote {
        x: f64,
        y: f64,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
        #[serde(default)]
        text: String,
        #[serde(default)]
        tag: String,
        #[serde(default, rename = "ref")]
        alias: Option<String>,
    },
    /// One freehand stroke in board units, from the window's pen: merged
    /// into the newest sketch it touches, else a sketch of its own. The core
    /// does the merge, so quick strokes never overwrite each other.
    AddStroke { stroke: Stroke },
    /// Ink: a new sketch, or with `id` the whole of an existing one replaced
    /// (strokes erased from it).
    SetSketch {
        #[serde(default)]
        id: Option<String>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        strokes: Vec<Stroke>,
    },
    Update {
        id: String,
        /// A note's or question's text, a frame's title, a link's or file's note.
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        /// A question's answer (the human's).
        #[serde(default)]
        answer: Option<String>,
        /// A link's address and title.
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    Move {
        id: String,
        x: f64,
        y: f64,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
    },
    /// A titled area; what lies inside it moves with it.
    CreateFrame {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default)]
        title: String,
        #[serde(default, rename = "ref")]
        alias: Option<String>,
    },
    /// An open question for the human to answer on the board.
    CreateQuestion {
        x: f64,
        y: f64,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
        #[serde(default)]
        text: String,
        #[serde(default, rename = "ref")]
        alias: Option<String>,
    },
    /// A web page as a reference card: its address, its title, and a note
    /// on why it matters.
    CreateLink {
        x: f64,
        y: f64,
        url: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        text: String,
        #[serde(default, rename = "ref")]
        alias: Option<String>,
    },
    /// A file already copied into the design's `files/` folder, from the
    /// window (the human drops or pastes it). Agents cannot add files.
    AddFile {
        x: f64,
        y: f64,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
        src: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        mime: String,
    },
    /// Items and arrows, by id; arrows on a removed item go with it.
    Delete { ids: Vec<String> },
    Connect {
        from: String,
        to: String,
        #[serde(default)]
        label: String,
    },
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

const TAGS: [&str; 5] = ["", "goal", "constraint", "question", "idea"];

/// Bounds on what one board holds, so a runaway agent or window cannot grow
/// it without end (every edit saves and sends the whole board).
const MAX_OPS: usize = 200;
const MAX_NODES: usize = 2_000;
const MAX_TEXT: usize = 4_000;
const MAX_LABEL: usize = 200;
const MAX_STROKES: usize = 500;
const MAX_POINTS: usize = 5_000;

fn check_text(text: &str, max: usize, what: &str) -> Result<(), String> {
    if text.chars().count() > max {
        Err(format!("{what} is longer than {max} characters"))
    } else {
        Ok(())
    }
}

/// The box around strokes given in board units, padded by their width.
fn strokes_box(strokes: &[Stroke]) -> (f64, f64, f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for s in strokes {
        for p in &s.points {
            x0 = x0.min(p[0] - s.size);
            y0 = y0.min(p[1] - s.size);
            x1 = x1.max(p[0] + s.size);
            y1 = y1.max(p[1] + s.size);
        }
    }
    if x0 > x1 {
        return (0.0, 0.0, 1.0, 1.0);
    }
    (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
}

/// Strokes moved by `(dx, dy)`: between a sketch's own units and the board's.
fn shifted(strokes: &[Stroke], dx: f64, dy: f64) -> Vec<Stroke> {
    strokes
        .iter()
        .map(|s| Stroke { points: s.points.iter().map(|p| [p[0] + dx, p[1] + dy, p[2]]).collect(), ..s.clone() })
        .collect()
}

fn check_strokes(strokes: &[Stroke]) -> Result<(), String> {
    if strokes.len() > MAX_STROKES {
        return Err(format!("a sketch holds at most {MAX_STROKES} strokes"));
    }
    for s in strokes {
        if s.points.len() > MAX_POINTS {
            return Err(format!("a stroke holds at most {MAX_POINTS} points"));
        }
        if !s.size.is_finite() || !(0.5..=64.0).contains(&s.size) || s.color.len() > 32 {
            return Err("a stroke's size or colour is out of range".to_string());
        }
        if s.points.iter().flatten().any(|v| !v.is_finite() || v.abs() > 1.0e7) {
            return Err("a stroke point is out of range".to_string());
        }
    }
    Ok(())
}
const NOTE_W: f64 = 260.0;
const NOTE_H: f64 = 150.0;
const LINK_W: f64 = 300.0;
const LINK_H: f64 = 110.0;
const MAX_URL: usize = 2_000;

/// An address a link card may hold: http or https, no spaces.
fn check_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    let ok = (u.starts_with("https://") || u.starts_with("http://")) && u.len() <= MAX_URL && !u.chars().any(char::is_whitespace);
    if ok {
        Ok(u.to_string())
    } else {
        Err(format!("{u:?} is not an http(s) address"))
    }
}

/// A file's place as a node keeps it: `files/<one plain name>`.
fn check_src(src: &str) -> Result<String, String> {
    let name = src.strip_prefix("files/").ok_or("a file lives under files/")?;
    let plain = !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".." && !name.starts_with('.');
    if plain {
        Ok(src.to_string())
    } else {
        Err(format!("{src:?} is not a file of the design"))
    }
}

/// Whether `inner` lies wholly inside `outer`'s box.
fn inside(inner: &Node, outer: &Node) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

fn finite(v: f64, what: &str) -> Result<f64, String> {
    if v.is_finite() && v.abs() < 1.0e7 {
        Ok(v)
    } else {
        Err(format!("{what} is out of range"))
    }
}

fn check_tag(tag: &str) -> Result<String, String> {
    let t = tag.trim().to_ascii_lowercase();
    if TAGS.contains(&t.as_str()) {
        Ok(t)
    } else {
        Err(format!("tag must be one of goal, constraint, question, idea or empty, not {tag}"))
    }
}

impl Doc {
    fn fresh_id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn entity(&self, id: &str) -> Option<Entity> {
        if let Some(n) = self.nodes.iter().find(|n| n.id == id) {
            return Some(Entity::Node(n.clone()));
        }
        self.edges.iter().find(|e| e.id == id).map(|e| Entity::Edge(e.clone()))
    }

    /// Put an item back as it was, or take it away when it was not there.
    fn restore(&mut self, target: &str, what: Option<&Entity>) {
        self.nodes.retain(|n| n.id != target);
        self.edges.retain(|e| e.id != target);
        match what {
            Some(Entity::Node(n)) => self.nodes.push(n.clone()),
            Some(Entity::Edge(e)) => self.edges.push(e.clone()),
            None => {}
        }
    }

    /// Note what an agent edit did to `target`, folding repeated edits of
    /// one item into a single change from its first state to its last.
    fn record(&mut self, actor: &Actor, target: &str, before: Option<Entity>) {
        let after = self.entity(target);
        match actor {
            // A human touching an item settles any suggestion on it.
            Actor::Human => self.changes.retain(|c| c.target != target),
            Actor::Agent { run } => {
                if let Some(c) = self.changes.iter_mut().find(|c| c.target == target) {
                    c.after = after;
                    c.run = run.clone();
                } else {
                    self.next += 1;
                    let id = self.next;
                    self.changes.push(Change { id, target: target.to_string(), run: run.clone(), before, after });
                }
                // Made and removed in the same breath: nothing to review.
                self.changes.retain(|c| c.before.is_some() || c.after.is_some());
            }
        }
    }

    /// Apply edits in order. Each result is `{ok, id?}` or `{ok: false, error}`;
    /// a failed edit does not stop the rest.
    pub fn apply(&mut self, ops: Vec<Op>, actor: &Actor) -> Vec<Value> {
        if ops.len() > MAX_OPS {
            return vec![json!({ "ok": false, "error": format!("at most {MAX_OPS} commands per call") })];
        }
        let mut aliases: HashMap<String, String> = HashMap::new();
        let resolve = |aliases: &HashMap<String, String>, id: &str| -> String {
            id.strip_prefix('$').and_then(|a| aliases.get(a).cloned()).unwrap_or_else(|| id.to_string())
        };
        let mut out = Vec::new();
        for op in ops {
            let result: Result<Value, String> = (|| match op {
                Op::CreateNote { x, y, w, h, text, tag, alias } => {
                    check_text(&text, MAX_TEXT, "a note")?;
                    if self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    let node = Node {
                        id: self.fresh_id("n"),
                        kind: Kind::Note,
                        x: finite(x, "x")?,
                        y: finite(y, "y")?,
                        w: finite(w.unwrap_or(NOTE_W), "w")?.clamp(80.0, 1600.0),
                        h: finite(h.unwrap_or(NOTE_H), "h")?.clamp(40.0, 1600.0),
                        text,
                        tag: check_tag(&tag)?,
                        strokes: Vec::new(),
                        by: actor.name().to_string(),
                        ..Default::default()
                    };
                    let id = node.id.clone();
                    self.nodes.push(node);
                    self.record(actor, &id, None);
                    if let Some(a) = alias {
                        aliases.insert(a, id.clone());
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::AddStroke { stroke } => {
                    if matches!(actor, Actor::Agent { .. }) {
                        return Err("agents do not draw ink; use notes and arrows".to_string());
                    }
                    check_strokes(std::slice::from_ref(&stroke))?;
                    if stroke.points.is_empty() {
                        return Err("an empty stroke".to_string());
                    }
                    let (bx, by, bw, bh) = strokes_box(std::slice::from_ref(&stroke));
                    const GAP: f64 = 24.0;
                    let target = self
                        .nodes
                        .iter()
                        .rev()
                        .find(|n| {
                            n.kind == Kind::Sketch
                                && n.x - GAP < bx + bw
                                && bx - GAP < n.x + n.w
                                && n.y - GAP < by + bh
                                && by - GAP < n.y + n.h
                        })
                        .map(|n| n.id.clone());
                    match target {
                        Some(id) => {
                            let before = self.entity(&id);
                            let node = self.nodes.iter_mut().find(|n| n.id == id).ok_or_else(|| format!("no sketch {id}"))?;
                            let mut all = shifted(&node.strokes, node.x, node.y);
                            all.push(stroke);
                            check_strokes(&all)?;
                            let (x, y, w, h) = strokes_box(&all);
                            (node.x, node.y, node.w, node.h) = (x, y, w, h);
                            node.strokes = shifted(&all, -x, -y);
                            self.record(actor, &id, before);
                            Ok(json!({ "ok": true, "id": id }))
                        }
                        None => {
                            if self.nodes.len() >= MAX_NODES {
                                return Err(format!("a board holds at most {MAX_NODES} items"));
                            }
                            let node = Node {
                                id: self.fresh_id("s"),
                                kind: Kind::Sketch,
                                x: bx,
                                y: by,
                                w: bw,
                                h: bh,
                                text: String::new(),
                                tag: String::new(),
                                strokes: shifted(std::slice::from_ref(&stroke), -bx, -by),
                                by: actor.name().to_string(),
                                ..Default::default()
                            };
                            let id = node.id.clone();
                            self.nodes.push(node);
                            self.record(actor, &id, None);
                            Ok(json!({ "ok": true, "id": id }))
                        }
                    }
                }
                Op::SetSketch { id, x, y, w, h, strokes } => {
                    if matches!(actor, Actor::Agent { .. }) {
                        return Err("agents do not draw ink; use notes and arrows".to_string());
                    }
                    let (x, y, w, h) = (finite(x, "x")?, finite(y, "y")?, finite(w, "w")?.max(1.0), finite(h, "h")?.max(1.0));
                    check_strokes(&strokes)?;
                    if id.is_none() && self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    match id {
                        Some(id) => {
                            let before = self.entity(&id);
                            let node = self
                                .nodes
                                .iter_mut()
                                .find(|n| n.id == id && n.kind == Kind::Sketch)
                                .ok_or_else(|| format!("no sketch {id}"))?;
                            if strokes.is_empty() {
                                self.nodes.retain(|n| n.id != id);
                                self.edges.retain(|e| e.from != id && e.to != id);
                            } else {
                                (node.x, node.y, node.w, node.h, node.strokes) = (x, y, w, h, strokes);
                            }
                            self.record(actor, &id, before);
                            Ok(json!({ "ok": true, "id": id }))
                        }
                        None => {
                            let node = Node {
                                id: self.fresh_id("s"),
                                kind: Kind::Sketch,
                                x,
                                y,
                                w,
                                h,
                                text: String::new(),
                                tag: String::new(),
                                strokes,
                                by: actor.name().to_string(),
                                ..Default::default()
                            };
                            let id = node.id.clone();
                            self.nodes.push(node);
                            self.record(actor, &id, None);
                            Ok(json!({ "ok": true, "id": id }))
                        }
                    }
                }
                Op::Update { id, text, tag, answer, url, title } => {
                    let id = resolve(&aliases, &id);
                    let before = self.entity(&id);
                    let tag = tag.map(|t| check_tag(&t)).transpose()?;
                    if let Some(t) = &text {
                        check_text(t, MAX_TEXT, "a note")?;
                    }
                    if let Some(a) = &answer {
                        if matches!(actor, Actor::Agent { .. }) {
                            return Err("answers are the human's; ask in chat or with another question".to_string());
                        }
                        check_text(a, MAX_TEXT, "an answer")?;
                    }
                    if let Some(t) = &title {
                        check_text(t, MAX_LABEL, "a title")?;
                    }
                    let url = url.map(|u| check_url(&u)).transpose()?;
                    let node = self.nodes.iter_mut().find(|n| n.id == id).ok_or_else(|| format!("no item {id}"))?;
                    if let Some(t) = text {
                        node.text = t;
                    }
                    if let Some(t) = tag {
                        node.tag = t;
                    }
                    if let Some(a) = answer {
                        if node.kind != Kind::Question {
                            return Err(format!("{id} is not a question"));
                        }
                        node.answer = a;
                    }
                    if let Some(u) = url {
                        if node.kind != Kind::Link {
                            return Err(format!("{id} is not a link"));
                        }
                        node.url = u;
                    }
                    if let Some(t) = title {
                        node.name = t;
                    }
                    self.record(actor, &id, before);
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::CreateFrame { x, y, w, h, title, alias } => {
                    check_text(&title, MAX_LABEL, "a frame's title")?;
                    if self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    let node = Node {
                        id: self.fresh_id("f"),
                        kind: Kind::Frame,
                        x: finite(x, "x")?,
                        y: finite(y, "y")?,
                        w: finite(w, "w")?.clamp(120.0, 8000.0),
                        h: finite(h, "h")?.clamp(80.0, 8000.0),
                        text: title,
                        by: actor.name().to_string(),
                        ..Default::default()
                    };
                    let id = node.id.clone();
                    // Under everything else, so what it holds stays on top.
                    self.nodes.insert(0, node);
                    self.record(actor, &id, None);
                    if let Some(a) = alias {
                        aliases.insert(a, id.clone());
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::CreateQuestion { x, y, w, h, text, alias } => {
                    check_text(&text, MAX_TEXT, "a question")?;
                    // The human makes one empty and types into it; the agent says what it asks.
                    if text.trim().is_empty() && matches!(actor, Actor::Agent { .. }) {
                        return Err("a question needs its text".to_string());
                    }
                    if self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    let node = Node {
                        id: self.fresh_id("q"),
                        kind: Kind::Question,
                        x: finite(x, "x")?,
                        y: finite(y, "y")?,
                        w: finite(w.unwrap_or(NOTE_W), "w")?.clamp(120.0, 1600.0),
                        h: finite(h.unwrap_or(NOTE_H + 40.0), "h")?.clamp(80.0, 1600.0),
                        text,
                        by: actor.name().to_string(),
                        ..Default::default()
                    };
                    let id = node.id.clone();
                    self.nodes.push(node);
                    self.record(actor, &id, None);
                    if let Some(a) = alias {
                        aliases.insert(a, id.clone());
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::CreateLink { x, y, url, title, text, alias } => {
                    let url = check_url(&url)?;
                    check_text(&title, MAX_LABEL, "a link's title")?;
                    check_text(&text, MAX_TEXT, "a link's note")?;
                    if self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    let node = Node {
                        id: self.fresh_id("l"),
                        kind: Kind::Link,
                        x: finite(x, "x")?,
                        y: finite(y, "y")?,
                        w: LINK_W,
                        h: LINK_H,
                        text,
                        name: title,
                        url,
                        by: actor.name().to_string(),
                        ..Default::default()
                    };
                    let id = node.id.clone();
                    self.nodes.push(node);
                    self.record(actor, &id, None);
                    if let Some(a) = alias {
                        aliases.insert(a, id.clone());
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::AddFile { x, y, w, h, src, name, mime } => {
                    if matches!(actor, Actor::Agent { .. }) {
                        return Err("files come from the human; add a link or a note instead".to_string());
                    }
                    let src = check_src(&src)?;
                    check_text(&name, MAX_LABEL, "a file name")?;
                    check_text(&mime, 100, "a media type")?;
                    if self.nodes.len() >= MAX_NODES {
                        return Err(format!("a board holds at most {MAX_NODES} items"));
                    }
                    let node = Node {
                        id: self.fresh_id("r"),
                        kind: Kind::File,
                        x: finite(x, "x")?,
                        y: finite(y, "y")?,
                        w: finite(w.unwrap_or(320.0), "w")?.clamp(60.0, 4000.0),
                        h: finite(h.unwrap_or(240.0), "h")?.clamp(40.0, 4000.0),
                        src,
                        name,
                        mime,
                        by: actor.name().to_string(),
                        ..Default::default()
                    };
                    let id = node.id.clone();
                    self.nodes.push(node);
                    self.record(actor, &id, None);
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::Move { id, x, y, w, h } => {
                    let id = resolve(&aliases, &id);
                    // Every value checked before the item is touched, so a bad
                    // one leaves it as it was.
                    let (x, y) = (finite(x, "x")?, finite(y, "y")?);
                    let w = w.map(|w| finite(w, "w").map(|w| w.clamp(1.0, 4000.0))).transpose()?;
                    let h = h.map(|h| finite(h, "h").map(|h| h.clamp(1.0, 4000.0))).transpose()?;
                    let before = self.entity(&id);
                    let old = self.nodes.iter().find(|n| n.id == id).cloned().ok_or_else(|| format!("no item {id}"))?;
                    // What a frame holds moves with it (the window moves those itself).
                    let members: Vec<String> = if old.kind == Kind::Frame && matches!(actor, Actor::Agent { .. }) {
                        self.nodes.iter().filter(|n| n.id != id && n.kind != Kind::Frame && inside(n, &old)).map(|n| n.id.clone()).collect()
                    } else {
                        Vec::new()
                    };
                    let node = self.nodes.iter_mut().find(|n| n.id == id).ok_or_else(|| format!("no item {id}"))?;
                    (node.x, node.y) = (x, y);
                    if let Some(w) = w {
                        node.w = w;
                    }
                    if let Some(h) = h {
                        node.h = h;
                    }
                    self.record(actor, &id, before);
                    let (dx, dy) = (x - old.x, y - old.y);
                    for m in members {
                        let before = self.entity(&m);
                        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == m) {
                            n.x += dx;
                            n.y += dy;
                        }
                        self.record(actor, &m, before);
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::Delete { ids } => {
                    // Arrows go with their note; naming one of those too, or
                    // naming an item twice, is not an error.
                    let mut gone: Vec<String> = Vec::new();
                    let mut missing: Vec<String> = Vec::new();
                    for raw in ids {
                        let id = resolve(&aliases, &raw);
                        if gone.contains(&id) {
                            continue;
                        }
                        if self.nodes.iter().any(|n| n.id == id) {
                            let attached: Vec<String> =
                                self.edges.iter().filter(|e| e.from == id || e.to == id).map(|e| e.id.clone()).collect();
                            for e in attached {
                                let before = self.entity(&e);
                                self.edges.retain(|x| x.id != e);
                                self.record(actor, &e, before);
                                gone.push(e);
                            }
                        }
                        match self.entity(&id) {
                            Some(before) => {
                                self.nodes.retain(|n| n.id != id);
                                self.edges.retain(|e| e.id != id);
                                self.record(actor, &id, Some(before));
                                gone.push(id);
                            }
                            None => missing.push(id),
                        }
                    }
                    Ok(json!({ "ok": missing.is_empty(), "deleted": gone, "missing": missing }))
                }
                Op::Connect { from, to, label } => {
                    check_text(&label, MAX_LABEL, "an arrow's label")?;
                    let (from, to) = (resolve(&aliases, &from), resolve(&aliases, &to));
                    for end in [&from, &to] {
                        if !self.nodes.iter().any(|n| &n.id == end) {
                            return Err(format!("no item {end}"));
                        }
                    }
                    if from == to {
                        return Err("an arrow needs two different items".to_string());
                    }
                    let edge = Edge { id: self.fresh_id("e"), from, to, label, by: actor.name().to_string() };
                    let id = edge.id.clone();
                    self.edges.push(edge);
                    self.record(actor, &id, None);
                    Ok(json!({ "ok": true, "id": id }))
                }
            })();
            out.push(result.unwrap_or_else(|error| json!({ "ok": false, "error": error })));
        }
        out
    }

    /// Settle agent changes: keep them as they are, or put the items back.
    pub fn review(&mut self, ids: &[u64], keep: bool) {
        let (settled, rest): (Vec<Change>, Vec<Change>) = std::mem::take(&mut self.changes).into_iter().partition(|c| ids.contains(&c.id));
        self.changes = rest;
        if keep {
            return;
        }
        // Newest first, so an arrow comes back only after the note it hangs on.
        for c in settled.iter().rev() {
            self.restore(&c.target, c.before.as_ref());
        }
        // Arrows whose ends are gone go too.
        let nodes: HashSet<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        self.edges.retain(|e| nodes.contains(&e.from) && nodes.contains(&e.to));
    }

    /// The board as the agent reads it: every item with its place, and the
    /// suggestions still waiting on the human.
    pub fn outline(&self) -> Value {
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| match n.kind {
                Kind::Note => json!({
                    "id": n.id, "kind": "note", "tag": n.tag, "text": n.text, "by": n.by,
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
                Kind::Sketch => json!({
                    "id": n.id, "kind": "sketch", "strokes": n.strokes.len(),
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
                // The path is in the agent's working folder, to read with its own tools.
                Kind::File => json!({
                    "id": n.id, "kind": "file", "name": n.name, "mime": n.mime, "path": n.src, "text": n.text, "by": n.by,
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
                Kind::Link => json!({
                    "id": n.id, "kind": "link", "url": n.url, "title": n.name, "text": n.text, "by": n.by,
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
                Kind::Frame => json!({
                    "id": n.id, "kind": "frame", "title": n.text, "by": n.by,
                    "holds": self.nodes.iter().filter(|m| m.id != n.id && m.kind != Kind::Frame && inside(m, n)).map(|m| m.id.as_str()).collect::<Vec<_>>(),
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
                Kind::Question => json!({
                    "id": n.id, "kind": "question", "text": n.text, "answer": n.answer, "by": n.by,
                    "x": n.x.round(), "y": n.y.round(), "w": n.w.round(), "h": n.h.round(),
                }),
            })
            .collect();
        let edges: Vec<Value> = self.edges.iter().map(|e| json!({ "id": e.id, "from": e.from, "to": e.to, "label": e.label })).collect();
        let pending: Vec<&str> = self.changes.iter().map(|c| c.target.as_str()).collect();
        json!({ "nodes": nodes, "edges": edges, "pending_suggestions": pending })
    }
}

// ----- the boards, cached from the store -----

/// One board, loaded on first use. Its lock is held for a whole edit
/// (read, change, save), so a human's edit and the agent's tool call never
/// start from the same version and overwrite each other.
type Slot = Arc<parking_lot::Mutex<Option<Doc>>>;

/// Every open design's board.
#[derive(Default)]
pub struct Boards {
    docs: parking_lot::Mutex<HashMap<String, Slot>>,
}

impl Boards {
    fn slot(&self, id: &str) -> Slot {
        self.docs.lock().entry(id.to_string()).or_default().clone()
    }

    /// Forget a board (its artifact is gone).
    pub fn forget(&self, id: &str) {
        self.docs.lock().remove(id);
    }
}

fn load_doc(state: &AppState, id: &str) -> Result<Doc, String> {
    let (info, raw) = state.store.artifact(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no design {id}"))?;
    if info.kind != KIND {
        return Err(format!("{id} is not a design"));
    }
    // Only an empty body is a new board. Anything else that does not read
    // stops here, rather than showing an empty board the next edit would
    // save over the real one.
    if raw.trim().is_empty() {
        return Ok(Doc::default());
    }
    parse_body(&raw).map_err(|e| format!("design {id} could not be read: {e}"))
}

/// A design's body as a board. Designs were briefly Excalidraw scenes
/// (2026-09-25); one saved then is read back into notes, arrows and ink.
pub fn parse_body(raw: &str) -> Result<Doc, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if v.get("elements").is_some() && v.get("nodes").is_none() {
        return Ok(from_scene(&v));
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// A note's tag from the fill an Excalidraw shape carried.
fn tag_of_fill(fill: &str) -> &'static str {
    match fill.to_ascii_lowercase().as_str() {
        "#b2f2bb" => "goal",
        "#ffd8a8" => "constraint",
        "#d0bfff" => "question",
        "#ffec99" => "idea",
        _ => "",
    }
}

/// An Excalidraw scene as a board: shapes, free text and frames become
/// notes (a "[tag] " the text starts with, or the shape's fill, gives the
/// tag), arrows joined at both ends become arrows, freehand ink becomes
/// sketches. Deleted elements are left out.
fn from_scene(v: &Value) -> Doc {
    let empty = Vec::new();
    let elements: Vec<&Value> = v
        .get("elements")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter(|e| !e.get("isDeleted").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    let s = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let f = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or_default();
    let label = |id: &str| {
        elements
            .iter()
            .find(|t| t.get("type").and_then(Value::as_str) == Some("text") && t.get("containerId").and_then(Value::as_str) == Some(id))
            .map(|t| s(t, "text"))
            .unwrap_or_default()
    };
    let mut doc = Doc { version: v.get("version").and_then(Value::as_u64).unwrap_or(0) + 1, ..Default::default() };
    let mut ids: HashSet<String> = HashSet::new();
    for e in &elements {
        let id = s(e, "id");
        let kind = s(e, "type");
        let (x, y, w, h) = (f(e, "x"), f(e, "y"), f(e, "width"), f(e, "height"));
        let text = match kind.as_str() {
            "rectangle" | "ellipse" | "diamond" => label(&id),
            "text" if s(e, "containerId").is_empty() => s(e, "text"),
            "frame" | "magicframe" => s(e, "name"),
            "freedraw" => {
                let points: Vec<[f64; 3]> = e
                    .get("points")
                    .and_then(Value::as_array)
                    .map(|pts| {
                        let pressures = e.get("pressures").and_then(Value::as_array);
                        pts.iter()
                            .enumerate()
                            .filter_map(|(i, p)| {
                                let p = p.as_array()?;
                                let pr = pressures.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(0.5);
                                Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?, pr])
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if points.is_empty() {
                    continue;
                }
                let color = s(e, "strokeColor");
                doc.nodes.push(Node {
                    id: id.clone(),
                    kind: Kind::Sketch,
                    x,
                    y,
                    w: w.max(1.0),
                    h: h.max(1.0),
                    text: String::new(),
                    tag: String::new(),
                    strokes: vec![Stroke { points, color, size: (f(e, "strokeWidth") * 2.0).max(1.0) }],
                    by: "human".to_string(),
                    ..Default::default()
                });
                ids.insert(id);
                continue;
            }
            _ => continue,
        };
        let (tag, text) = match text.strip_prefix('[').and_then(|t| t.split_once("] ")) {
            Some((t, rest)) if TAGS.contains(&t) => (t.to_string(), rest.to_string()),
            _ => (tag_of_fill(&s(e, "backgroundColor")).to_string(), text),
        };
        let by = e.pointer("/customData/by").and_then(Value::as_str).unwrap_or("human").to_string();
        doc.nodes.push(Node {
            id: id.clone(),
            kind: Kind::Note,
            x,
            y,
            // A note is at least note-sized, whatever box it came from.
            w: w.max(NOTE_W),
            h: h.max(NOTE_H),
            text,
            tag,
            strokes: Vec::new(),
            by,
            ..Default::default()
        });
        ids.insert(id);
    }
    for e in &elements {
        if !matches!(e.get("type").and_then(Value::as_str), Some("arrow" | "line")) {
            continue;
        }
        let from = e.pointer("/startBinding/elementId").and_then(Value::as_str).unwrap_or_default();
        let to = e.pointer("/endBinding/elementId").and_then(Value::as_str).unwrap_or_default();
        if ids.contains(from) && ids.contains(to) && from != to {
            let id = s(e, "id");
            doc.edges.push(Edge { label: label(&id), id, from: from.to_string(), to: to.to_string(), by: "human".to_string() });
        }
    }
    doc
}

/// The board as it is now.
pub fn doc(state: &AppState, id: &str) -> Result<Doc, String> {
    let slot = state.boards.slot(id);
    let mut board = slot.lock();
    if board.is_none() {
        *board = Some(load_doc(state, id)?);
    }
    board.clone().ok_or_else(|| format!("design {id} vanished"))
}

/// What the window hears after every change: only what changed. `base`
/// is the version the change was made on; a window holding another version
/// has missed one and reads the whole board again.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DesignDelta {
    pub id: String,
    pub base: u64,
    pub version: u64,
    /// Items new or changed, in board order.
    pub nodes: Vec<Node>,
    pub removed_nodes: Vec<String>,
    pub edges: Vec<Edge>,
    pub removed_edges: Vec<String>,
    /// The suggestions waiting now (whole: they are few, and small unless
    /// the agent removed ink).
    pub changes: Vec<Change>,
    pub next: u64,
}

impl DesignDelta {
    /// What turned `old` into `new`.
    pub fn between(id: &str, old: &Doc, new: &Doc) -> Self {
        let old_nodes: HashMap<&str, &Node> = old.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let old_edges: HashMap<&str, &Edge> = old.edges.iter().map(|e| (e.id.as_str(), e)).collect();
        let new_nodes: HashSet<&str> = new.nodes.iter().map(|n| n.id.as_str()).collect();
        let new_edges: HashSet<&str> = new.edges.iter().map(|e| e.id.as_str()).collect();
        DesignDelta {
            id: id.to_string(),
            base: old.version,
            version: new.version,
            nodes: new.nodes.iter().filter(|n| old_nodes.get(n.id.as_str()) != Some(n)).cloned().collect(),
            removed_nodes: old.nodes.iter().filter(|n| !new_nodes.contains(n.id.as_str())).map(|n| n.id.clone()).collect(),
            edges: new.edges.iter().filter(|e| old_edges.get(e.id.as_str()) != Some(e)).cloned().collect(),
            removed_edges: old.edges.iter().filter(|e| !new_edges.contains(e.id.as_str())).map(|e| e.id.clone()).collect(),
            changes: new.changes.clone(),
            next: new.next,
        }
    }
}

/// Run `f` on the board, save it, and tell the window. The board's lock
/// is held from reading it to keeping the result; only telling the window
/// happens after.
fn edit<R>(app: &AppHandle, id: &str, f: impl FnOnce(&mut Doc) -> R) -> Result<R, String> {
    let state = app.state::<AppState>();
    let slot = state.boards.slot(id);
    let (out, delta) = {
        let mut board = slot.lock();
        if board.is_none() {
            *board = Some(load_doc(&state, id)?);
        }
        let old = board.take().ok_or_else(|| format!("design {id} vanished"))?;
        let mut current = old.clone();
        let out = f(&mut current);
        current.version += 1;
        let saved = serde_json::to_string(&current)
            .map_err(|e| e.to_string())
            .and_then(|json| {
                state
                    .store
                    .update_artifact(id, &orchestra_store::ArtifactPatch { body: Some(json), ..Default::default() })
                    .map_err(|e| e.to_string())
            });
        if let Err(err) = saved {
            // Not saved: the board stays as it was.
            *board = Some(old);
            return Err(err);
        }
        let delta = DesignDelta::between(id, &old, &current);
        *board = Some(current);
        (out, delta)
    };
    let _ = app.emit("design", &delta);
    Ok(out)
}

/// Apply edits to a design's board.
pub fn apply(app: &AppHandle, id: &str, ops: Vec<Op>, actor: Actor) -> Result<Vec<Value>, String> {
    edit(app, id, |d| d.apply(ops, &actor))
}

/// Keep or revert agent changes.
pub fn review(app: &AppHandle, id: &str, changes: &[u64], keep: bool) -> Result<(), String> {
    edit(app, id, |d| d.review(changes, keep))
}

// ----- reference files -----

/// Largest file brought onto a board.
const MAX_FILE: u64 = 50 * 1024 * 1024;
/// Files brought in one go.
const MAX_FILES: usize = 20;

/// A media type by extension; what the board and the agent go by.
pub fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// A file name safe to keep: letters, digits, dot, dash, underscore.
fn plain_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let clean = clean.trim_start_matches('.').to_string();
    if clean.trim_matches(['.', '_']).is_empty() { "file".to_string() } else { clean.chars().take(80).collect() }
}

/// A card's first size for a file of this type.
fn file_size(mime: &str) -> (f64, f64) {
    if mime.starts_with("image/") {
        (320.0, 240.0)
    } else if mime == "application/pdf" {
        (360.0, 460.0)
    } else {
        (280.0, 110.0)
    }
}

/// What the window brings onto a board: a file on disk, or bytes (a pasted
/// image).
pub enum Incoming {
    Path(std::path::PathBuf),
    Bytes { name: String, data: Vec<u8> },
}

/// Copy files into the design's `files/` folder and lay them out as cards
/// in a row from `(x, y)`. Returns one result per file.
pub fn add_files(app: &AppHandle, id: &str, incoming: Vec<Incoming>, x: f64, y: f64) -> Result<Vec<Value>, String> {
    if incoming.len() > MAX_FILES {
        return Err(format!("at most {MAX_FILES} files at once"));
    }
    let state = app.state::<AppState>();
    let dir = crate::artifact::workdir(&state, id)?.join("files");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut ops = Vec::new();
    let mut failed = Vec::new();
    let mut at = x;
    for item in incoming {
        let (name, copied) = match item {
            Incoming::Path(p) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
                let ok = std::fs::metadata(&p).map_err(|e| e.to_string()).and_then(|m| {
                    if !m.is_file() {
                        Err("not a file".to_string())
                    } else if m.len() > MAX_FILE {
                        Err("larger than 50 MB".to_string())
                    } else {
                        Ok(())
                    }
                });
                (name, ok.map(|_| Source::Path(p)))
            }
            Incoming::Bytes { name, data } => {
                let ok = if data.len() as u64 > MAX_FILE { Err("larger than 50 MB".to_string()) } else { Ok(Source::Bytes(data)) };
                (name, ok)
            }
        };
        let source = match copied {
            Ok(s) => s,
            Err(e) => {
                failed.push(json!({ "ok": false, "name": name, "error": e }));
                continue;
            }
        };
        let stamp = uuid::Uuid::new_v4().simple().to_string();
        let stored = format!("{}-{}", &stamp[..8], plain_name(&name));
        let target = dir.join(&stored);
        let written = match source {
            Source::Path(p) => std::fs::copy(&p, &target).map(|_| ()),
            Source::Bytes(b) => std::fs::write(&target, b),
        };
        if let Err(e) = written {
            failed.push(json!({ "ok": false, "name": name, "error": e.to_string() }));
            continue;
        }
        let mime = mime_of(&name).to_string();
        let (w, h) = file_size(&mime);
        ops.push(Op::AddFile { x: at, y, w: Some(w), h: Some(h), src: format!("files/{stored}"), name, mime });
        at += w + 40.0;
    }
    let mut out = if ops.is_empty() { Vec::new() } else { apply(app, id, ops, Actor::Human)? };
    out.extend(failed);
    Ok(out)
}

enum Source {
    Path(std::path::PathBuf),
    Bytes(Vec<u8>),
}

/// Where a reference file of a design is on disk; only its own files.
pub fn file_path(state: &AppState, id: &str, src: &str) -> Result<std::path::PathBuf, String> {
    let src = check_src(src)?;
    let path = crate::artifact::workdir(state, id)?.join(src);
    if path.is_file() {
        Ok(path)
    } else {
        Err("the file is gone".to_string())
    }
}

/// The files of a board's reference cards, as paths on disk: all of them,
/// or those among `only`.
pub fn reference_files(state: &AppState, id: &str, only: Option<&[String]>) -> Vec<std::path::PathBuf> {
    let Ok(d) = doc(state, id) else { return Vec::new() };
    let Ok(dir) = crate::artifact::workdir(state, id) else { return Vec::new() };
    d.nodes
        .iter()
        .filter(|n| n.kind == Kind::File && only.is_none_or(|o| o.contains(&n.id)))
        .filter_map(|n| check_src(&n.src).ok().map(|src| dir.join(src)))
        .filter(|p| p.is_file())
        .collect()
}

/// What goes with each message: the board as an outline and the items the
/// human picked on it.
pub fn context(state: &AppState, id: &str, selected: &[String]) -> Result<String, String> {
    let mut out = format!("[board]\n{}", doc(state, id)?.outline());
    if !selected.is_empty() {
        out.push_str(&format!("\nselected: {}", selected.join(", ")));
    }
    Ok(out)
}

// ----- the agent -----

/// What the design agent is told once, at the start of its session.
pub fn preamble(lang: &str, title: &str) -> String {
    if lang == "ko" {
        return format!(
            r#"당신은 Divixi의 디자인 파트너입니다. 사람과 함께 스케치 보드에서 "{title}"의 설계 초안을 잡습니다. 이 디자인은 나중에 트랙의 지휘자에게 참고 자료로 첨부됩니다. 지금은 생각을 꺼내 놓고, 모양을 잡고, 무엇을 만들지 분명히 하는 단계입니다.

보드:
- 메모(note), 손그림(sketch), 화살표(edge), 레퍼런스 파일(file: 이미지·PDF·문서), 링크(link: 웹 페이지), 틀(frame: 제목 붙은 묶음), 질문(question: 사람이 답을 적는 카드)이 있습니다. 메모에는 태그를 붙일 수 있습니다: goal(목표), constraint(제약), question(미해결 질문), idea(아이디어).
- 매 메시지마다 보드 개요가 [board] 아래 JSON으로, 손그림이 있으면 보드 전체 그림이 첨부로 옵니다. 사람이 고른 항목은 selected로 오고, 고른 파일은 메시지에 첨부됩니다.
- 파일 카드의 path(files/…)는 당신의 작업 폴더 안에 있습니다. 이미지와 PDF, 문서는 당신의 파일 읽기 도구로 직접 읽으세요. 레퍼런스를 읽고 핵심을 메모로 정리하고, 관련된 것끼리 화살표로 잇거나 틀로 묶습니다.
- 사람에게 물어야 할 것은 create_question으로 보드에 올립니다. 사람이 적은 답은 개요의 answer에 옵니다. 답을 대신 적지 않습니다.
- 보드를 바꿀 때는 `board_write`를 씁니다. 한 번에 여러 명령을 보낼 수 있고, create_note에 ref를 주면 같은 호출 안에서 "$ref"로 가리킬 수 있습니다. 최신 상태가 필요하면 `board_read`.
- 당신이 바꾼 것은 사람이 유지하거나 되돌리기 전까지 "제안"으로 표시됩니다. 되돌려진 것을 다시 밀어붙이지 마세요.

일하는 법:
- 메모는 짧게: 한 메모에 한 생각, 두세 줄. 긴 설명은 메모를 나누거나 대화로.
- 배치: 메모 기본 크기 260×150, 간격 40. 관련 있는 것은 가까이, 흐름은 왼쪽에서 오른쪽, 위에서 아래로. 기존 항목과 겹치지 않게 빈 자리를 찾습니다.
- 사람의 손그림은 의도를 담고 있습니다. 그림이 무엇을 뜻하는지 읽고, 모호하면 추측하지 말고 question 메모나 대화로 묻습니다.
- 목표, 제약, 미해결 질문을 드러내는 것이 목적입니다. 설계가 트랙에 넘겨도 될 만큼 분명해지면 그렇다고 말합니다.
- 대화 답은 짧게. 보드에 쓴 것을 대화에서 되풀이하지 않습니다. 한국어로 말합니다.
"#
        );
    }
    format!(
        r#"You are Divixi's design partner. With the human you rough out a first design of "{title}" on a sketch board. The design will later be attached to a track's conductor as reference. This is the stage of getting thoughts out, giving them shape, and making clear what should be built.

The board:
- Notes, freehand sketches, arrows (edges), reference files (file: images, PDFs, documents), links (link: web pages), frames (a titled group) and questions (a card the human answers). Notes can carry a tag: goal, constraint, question (open question) or idea.
- Every message brings an outline of the board as JSON under [board], and a picture of the whole board attached when it has sketches. Items the human selected come as selected; selected files are attached to the message.
- A file card's path (files/…) is in your working folder: read images, PDFs and documents with your own file tools. Read the references, sum up what matters in notes, connect related things with arrows or group them in frames.
- Put what you need to ask the human on the board with create_question; their answer comes as the question's answer in the outline. Never write answers yourself.
- Change the board with `board_write`: several commands per call; give create_note a ref to point at it as "$ref" later in the same call. Call `board_read` for the latest state.
- What you change shows as a suggestion until the human keeps or reverts it. Do not push back what was reverted.

How to work:
- Keep notes short: one thought per note, two or three lines. Split long explanations or say them in chat.
- Layout: notes are 260×150 by default, 40 apart. Related things close together; flow left to right, top to bottom. Find empty space; do not overlap what is there.
- The human's sketches carry intent. Read what a drawing means; when it is unclear, ask with a question note or in chat rather than guessing.
- The aim is to surface goals, constraints and open questions. When the design is clear enough to hand to a track, say so.
- Keep chat replies short and do not repeat in chat what you wrote on the board. Speak English.
"#
    )
}

/// The agent's two board tools, by name and description; the app and the
/// `design_agent` example build them from the same text.
pub const BOARD_READ: &str = "board_read";
pub const BOARD_READ_DESC: &str = "Read the design's board: every note (id, tag, text, position, size, who made it), sketch, reference file (name, media type, path in your working folder), link (url, title), frame (title, what it holds), question (text, the human's answer) and arrow, and which items are suggestions still waiting on the human.";
pub const BOARD_WRITE: &str = "board_write";
pub const BOARD_WRITE_DESC: &str = "Change the design's board with a list of commands, applied in order. Each result says ok with the item id, or the error. create_note {x, y, text, tag?, w?, h?, ref?} (tag: goal | constraint | question | idea); create_question {x, y, text, w?, h?, ref?} (a card the human answers); create_link {x, y, url, title?, text?, ref?} (a web page as a reference); create_frame {x, y, w, h, title?, ref?} (a titled area; what lies inside moves with it); update {id, text?, tag?, url?, title?}; move {id, x, y, w?, h?}; delete {ids}; connect {from, to, label?}. Ids may be \"$ref\" for an item made earlier in the same call. Files come only from the human. Your changes show as suggestions the human keeps or reverts.";

/// Arguments of `board_write`: a list of commands.
pub fn board_write_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "commands": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["create_note", "create_question", "create_link", "create_frame", "update", "move", "delete", "connect"] },
                        "id": { "type": "string" },
                        "ids": { "type": "array", "items": { "type": "string" } },
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "w": { "type": "number" },
                        "h": { "type": "number" },
                        "text": { "type": "string" },
                        "tag": { "type": "string", "enum": ["", "goal", "constraint", "question", "idea"] },
                        "ref": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "label": { "type": "string" },
                        "url": { "type": "string" },
                        "title": { "type": "string" }
                    },
                    "required": ["op"]
                }
            }
        },
        "required": ["commands"]
    })
}

/// The commands in a `board_write` call.
pub fn parse_commands(args: &Value) -> Result<Vec<Op>, String> {
    let raw = args.get("commands").cloned().unwrap_or(Value::Null);
    serde_json::from_value(raw).map_err(|e| format!("bad commands: {e}"))
}

/// The design agent's tools, scoped to one design.
pub fn tools(app: AppHandle, id: String) -> Vec<Tool> {
    let read = (app.clone(), id.clone());
    let write = (app, id);
    vec![
        Tool::new(
            BOARD_READ,
            BOARD_READ_DESC,
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            move |_args| {
                let (app, id) = read.clone();
                async move {
                    let state = app.state::<AppState>();
                    Ok(doc(&state, &id)?.outline())
                }
            },
        ),
        Tool::new(BOARD_WRITE, BOARD_WRITE_DESC, board_write_schema(), move |args| {
            let (app, id) = write.clone();
            async move {
                let ops = parse_commands(&args)?;
                let run = crate::artifact::running(&app, &id).await;
                let results = apply(&app, &id, ops, Actor::Agent { run })?;
                Ok(json!({ "results": results }))
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_links_questions_and_files() {
        let mut d = Doc::default();
        let agent = Actor::Agent { run: None };
        let out = d.apply(
            ops(json!([
                { "op": "create_frame", "x": 0, "y": 0, "w": 800, "h": 500, "title": "Research", "ref": "f" },
                { "op": "create_note", "x": 40, "y": 60, "text": "inside" },
                { "op": "create_note", "x": 900, "y": 60, "text": "outside" },
                { "op": "create_link", "x": 400, "y": 60, "url": "https://example.com/a", "title": "Example", "text": "why it matters" },
                { "op": "create_question", "x": 40, "y": 240, "text": "Who uses it?", "ref": "q" },
                { "op": "create_link", "x": 0, "y": 0, "url": "javascript:alert(1)" },
                { "op": "add_file", "x": 0, "y": 0, "src": "files/a.png" }
            ])),
            &agent,
        );
        assert!(out[..5].iter().all(|r| r["ok"] == true), "{out:?}");
        assert_eq!(out[5]["ok"], false, "only http(s) links");
        assert_eq!(out[6]["ok"], false, "files come from the human");
        let frame = out[0]["id"].as_str().unwrap().to_string();
        let q = out[4]["id"].as_str().unwrap().to_string();
        assert_eq!(d.nodes[0].kind, Kind::Frame, "a frame lies under the rest");
        let outline = d.outline();
        let holds = outline["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "frame").unwrap()["holds"].clone();
        assert_eq!(holds.as_array().unwrap().len(), 3, "the note, the link and the question: {holds}");

        // The agent moves the frame: what it holds comes along, what is outside does not.
        let before: Vec<(String, f64)> = d.nodes.iter().map(|n| (n.text.clone(), n.x)).collect();
        d.apply(ops(json!([{ "op": "move", "id": frame, "x": 100, "y": 0 }])), &agent);
        let x_of = |t: &str| d.nodes.iter().find(|n| n.text == t).unwrap().x;
        assert_eq!(x_of("inside"), 140.0);
        assert_eq!(x_of("outside"), 900.0);
        assert!(before.iter().any(|(t, _)| t == "inside"));

        // Answers are the human's.
        let refused = d.apply(ops(json!([{ "op": "update", "id": q, "answer": "me" }])), &agent);
        assert_eq!(refused[0]["ok"], false);
        let answered = d.apply(ops(json!([{ "op": "update", "id": q, "answer": "Parents first" }])), &Actor::Human);
        assert_eq!(answered[0]["ok"], true);
        assert_eq!(d.outline()["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "question").unwrap()["answer"], "Parents first");

        // The window adds files it copied into files/; nothing outside it.
        let file = d.apply(ops(json!([{ "op": "add_file", "x": 0, "y": 500, "src": "files/ab-spec.pdf", "name": "spec.pdf", "mime": "application/pdf" }])), &Actor::Human);
        assert_eq!(file[0]["ok"], true);
        for bad in ["../secret", "files/../x", "files/", "files/.env", "files/a/b"] {
            assert!(check_src(bad).is_err(), "{bad}");
        }
        let f = d.outline()["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "file").unwrap().clone();
        assert_eq!((f["path"].as_str(), f["mime"].as_str()), (Some("files/ab-spec.pdf"), Some("application/pdf")));
        assert_eq!(mime_of("Photo.JPG"), "image/jpeg");
        assert_eq!(plain_name("../../내 사진 (1).png"), "_.._내_사진__1_.png");
    }

    #[test]
    fn excalidraw_scenes_read_back_as_boards() {
        let scene = json!({
            "version": 4,
            "elements": [
                { "id": "n1", "type": "rectangle", "x": 0, "y": 0, "width": 260, "height": 150, "backgroundColor": "#b2f2bb" },
                { "id": "t1", "type": "text", "containerId": "n1", "text": "[goal] Ship it", "x": 5, "y": 5, "width": 10, "height": 10 },
                { "id": "n2", "type": "diamond", "x": 400, "y": 0, "width": 200, "height": 100, "backgroundColor": "#d0bfff" },
                { "id": "t2", "type": "text", "containerId": "n2", "text": "Offline?", "x": 5, "y": 5, "width": 10, "height": 10 },
                { "id": "free", "type": "text", "text": "a loose thought", "x": 0, "y": 300, "width": 120, "height": 25 },
                { "id": "gone", "type": "rectangle", "x": 0, "y": 0, "width": 50, "height": 50, "isDeleted": true },
                { "id": "a1", "type": "arrow", "x": 260, "y": 75, "width": 140, "height": 0,
                  "startBinding": { "elementId": "n1" }, "endBinding": { "elementId": "n2" } },
                { "id": "t3", "type": "text", "containerId": "a1", "text": "then", "x": 0, "y": 0, "width": 1, "height": 1 },
                { "id": "ink", "type": "freedraw", "x": 10, "y": 400, "width": 30, "height": 20, "strokeColor": "#e03131", "strokeWidth": 2,
                  "points": [[0, 0], [30, 20]], "pressures": [0.4, 0.6] }
            ],
            "files": {}
        });
        let d = parse_body(&scene.to_string()).unwrap();
        assert_eq!(d.version, 5);
        let note = |id: &str| d.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!((note("n1").text.as_str(), note("n1").tag.as_str()), ("Ship it", "goal"));
        assert_eq!((note("n2").text.as_str(), note("n2").tag.as_str()), ("Offline?", "question"), "the fill gives the tag");
        assert_eq!(note("free").text, "a loose thought");
        assert_eq!(note("free").w, NOTE_W, "a thin text box becomes a note-sized one");
        assert!(d.nodes.iter().all(|n| n.id != "gone" && n.id != "t1"));
        assert_eq!(d.edges.len(), 1);
        assert_eq!((d.edges[0].from.as_str(), d.edges[0].to.as_str(), d.edges[0].label.as_str()), ("n1", "n2", "then"));
        let ink = note("ink");
        assert_eq!(ink.kind, Kind::Sketch);
        assert_eq!(ink.strokes[0].points[1], [30.0, 20.0, 0.6]);
        assert_eq!(ink.strokes[0].color, "#e03131");
        // A board reads back as itself.
        let again = parse_body(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(again, d);
    }

    fn ops(v: Value) -> Vec<Op> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn agent_edits_are_suggestions_until_kept_or_reverted() {
        let mut d = Doc::default();
        let human = d.apply(ops(json!([{ "op": "create_note", "x": 0, "y": 0, "text": "mine" }])), &Actor::Human);
        let mine = human[0]["id"].as_str().unwrap().to_string();
        assert!(d.changes.is_empty(), "the human's own edits are not suggestions");

        let agent = Actor::Agent { run: Some("t001".into()) };
        let out = d.apply(
            ops(json!([
                { "op": "create_note", "x": 300, "y": 0, "text": "goal", "tag": "goal", "ref": "g" },
                { "op": "connect", "from": mine, "to": "$g" },
                { "op": "update", "id": mine, "text": "mine, sharper" },
                { "op": "update", "id": mine, "text": "mine, sharpest" }
            ])),
            &agent,
        );
        assert!(out.iter().all(|r| r["ok"] == true), "{out:?}");
        assert_eq!(d.changes.len(), 3, "two edits of one note fold into one change");
        let edit = d.changes.iter().find(|c| c.target == mine).unwrap();
        assert!(matches!(&edit.before, Some(Entity::Node(n)) if n.text == "mine"));

        // Revert everything: the note is back, the agent's note and arrow are gone.
        let all: Vec<u64> = d.changes.iter().map(|c| c.id).collect();
        d.review(&all, false);
        assert_eq!(d.nodes.len(), 1);
        assert_eq!(d.nodes[0].text, "mine");
        assert!(d.edges.is_empty() && d.changes.is_empty());
    }

    #[test]
    fn a_human_edit_keeps_the_suggestion_and_bad_commands_fail_alone() {
        let mut d = Doc::default();
        let agent = Actor::Agent { run: None };
        let out = d.apply(
            ops(json!([
                { "op": "create_note", "x": 0, "y": 0, "text": "a" },
                { "op": "update", "id": "nope", "text": "x" },
                { "op": "create_note", "x": 0, "y": 0, "tag": "wrong" }
            ])),
            &agent,
        );
        assert_eq!((out[0]["ok"].clone(), out[1]["ok"].clone(), out[2]["ok"].clone()), (json!(true), json!(false), json!(false)));
        let id = out[0]["id"].as_str().unwrap().to_string();
        d.apply(ops(json!([{ "op": "move", "id": id, "x": 50, "y": 50 }])), &Actor::Human);
        assert!(d.changes.is_empty(), "touching a suggestion keeps it");
        let ink = d.apply(ops(json!([{ "op": "set_sketch", "x": 0, "y": 0, "w": 1, "h": 1, "strokes": [] }])), &agent);
        assert_eq!(ink[0]["ok"], false, "agents do not draw");
    }

    #[test]
    fn a_bad_move_leaves_the_item_and_delete_tolerates_what_went_already() {
        let mut d = Doc::default();
        let made = d.apply(ops(json!([{ "op": "create_note", "x": 0, "y": 0, "text": "a" }, { "op": "create_note", "x": 300, "y": 0 }])), &Actor::Human);
        let (a, b) = (made[0]["id"].as_str().unwrap().to_string(), made[1]["id"].as_str().unwrap().to_string());
        let out = d.apply(ops(json!([{ "op": "move", "id": a, "x": 100, "y": 2.0e7 }])), &Actor::Agent { run: None });
        assert_eq!(out[0]["ok"], false);
        assert_eq!(d.nodes[0].x, 0.0, "nothing moved");
        assert!(d.changes.is_empty(), "nothing to review");

        let arrow = d.apply(ops(json!([{ "op": "connect", "from": a, "to": b }])), &Actor::Human)[0]["id"].as_str().unwrap().to_string();
        let out = d.apply(ops(json!([{ "op": "delete", "ids": [a, arrow, "nope"] }])), &Actor::Human);
        assert_eq!(out[0]["deleted"].as_array().unwrap().len(), 2, "{out:?}");
        assert_eq!(out[0]["missing"], json!(["nope"]));
        assert_eq!((d.nodes.len(), d.edges.len()), (1, 0));
    }

    #[test]
    fn strokes_merge_in_the_core_without_losing_one() {
        let mut d = Doc::default();
        let stroke = |x: f64| json!({ "points": [[x, 0.0, 0.5], [x + 10.0, 10.0, 0.5]], "size": 4.0 });
        // Two quick strokes close together land in one sketch, both kept.
        let a = d.apply(ops(json!([{ "op": "add_stroke", "stroke": stroke(0.0) }])), &Actor::Human);
        let b = d.apply(ops(json!([{ "op": "add_stroke", "stroke": stroke(20.0) }])), &Actor::Human);
        assert_eq!(a[0]["id"], b[0]["id"]);
        assert_eq!((d.nodes.len(), d.nodes[0].strokes.len()), (1, 2));
        let n = &d.nodes[0];
        assert!(n.x <= -4.0 && n.x + n.w >= 34.0, "the box covers both: {n:?}");
        assert!(n.strokes.iter().flat_map(|s| &s.points).all(|p| p[0] >= 0.0 && p[1] >= 0.0), "kept in the sketch's own units");
        // One far away is a sketch of its own.
        d.apply(ops(json!([{ "op": "add_stroke", "stroke": stroke(500.0) }])), &Actor::Human);
        assert_eq!(d.nodes.len(), 2);
        let agent = d.apply(ops(json!([{ "op": "add_stroke", "stroke": stroke(0.0) }])), &Actor::Agent { run: None });
        assert_eq!(agent[0]["ok"], false, "agents do not draw");
    }

    #[test]
    fn a_delta_carries_only_what_changed() {
        let mut d = Doc::default();
        d.apply(ops(json!([
            { "op": "create_note", "x": 0, "y": 0, "text": "a" },
            { "op": "create_note", "x": 300, "y": 0, "text": "b" },
            { "op": "create_note", "x": 600, "y": 0, "text": "c" }
        ])), &Actor::Human);
        let (a, b) = (d.nodes[0].id.clone(), d.nodes[1].id.clone());
        let old = d.clone();
        d.apply(ops(json!([{ "op": "move", "id": a, "x": 5, "y": 5 }, { "op": "delete", "ids": [b] }, { "op": "create_note", "x": 0, "y": 300 }])), &Actor::Human);
        d.version += 1;
        let delta = DesignDelta::between("ar001", &old, &d);
        assert_eq!((delta.base, delta.version), (old.version, d.version));
        let changed: Vec<&str> = delta.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(changed.len(), 2, "the moved note and the new one: {changed:?}");
        assert!(changed.contains(&a.as_str()));
        assert_eq!(delta.removed_nodes, vec![b]);
        assert!(delta.edges.is_empty() && delta.removed_edges.is_empty());
    }

    #[test]
    fn limits_keep_a_board_bounded() {
        let mut d = Doc::default();
        let long = "x".repeat(MAX_TEXT + 1);
        let out = d.apply(ops(json!([{ "op": "create_note", "x": 0, "y": 0, "text": long }])), &Actor::Agent { run: None });
        assert_eq!(out[0]["ok"], false);
        let many: Vec<[f64; 3]> = vec![[0.0, 0.0, 0.5]; MAX_POINTS + 1];
        let out = d.apply(ops(json!([{ "op": "set_sketch", "x": 0, "y": 0, "w": 1, "h": 1, "strokes": [{ "points": many }] }])), &Actor::Human);
        assert_eq!(out[0]["ok"], false);
        let too_many: Vec<Value> = (0..=MAX_OPS).map(|_| json!({ "op": "delete", "ids": [] })).collect();
        assert_eq!(d.apply(ops(Value::Array(too_many)), &Actor::Human).len(), 1);
        assert!(d.nodes.is_empty());
    }

    #[test]
    fn deleting_a_note_takes_its_arrows_and_revert_brings_both_back() {
        let mut d = Doc::default();
        let made = d.apply(
            ops(json!([
                { "op": "create_note", "x": 0, "y": 0, "text": "a" },
                { "op": "create_note", "x": 300, "y": 0, "text": "b" }
            ])),
            &Actor::Human,
        );
        let (a, b) = (made[0]["id"].as_str().unwrap().to_string(), made[1]["id"].as_str().unwrap().to_string());
        d.apply(ops(json!([{ "op": "connect", "from": a, "to": b }])), &Actor::Human);
        d.apply(ops(json!([{ "op": "delete", "ids": [a] }])), &Actor::Agent { run: None });
        assert_eq!((d.nodes.len(), d.edges.len(), d.changes.len()), (1, 0, 2));
        let all: Vec<u64> = d.changes.iter().map(|c| c.id).collect();
        d.review(&all, false);
        assert_eq!((d.nodes.len(), d.edges.len()), (2, 1));
    }
}
