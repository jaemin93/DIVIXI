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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A card of text.
    Note,
    /// Freehand ink.
    Sketch,
}

/// One item on the board. Position and size are in board units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Ink: a new sketch, or with `id` the whole of an existing one replaced
    /// (strokes merged into it, erased from it).
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
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        tag: Option<String>,
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
const NOTE_W: f64 = 260.0;
const NOTE_H: f64 = 150.0;

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
        let mut aliases: HashMap<String, String> = HashMap::new();
        let resolve = |aliases: &HashMap<String, String>, id: &str| -> String {
            id.strip_prefix('$').and_then(|a| aliases.get(a).cloned()).unwrap_or_else(|| id.to_string())
        };
        let mut out = Vec::new();
        for op in ops {
            let result: Result<Value, String> = (|| match op {
                Op::CreateNote { x, y, w, h, text, tag, alias } => {
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
                    };
                    let id = node.id.clone();
                    self.nodes.push(node);
                    self.record(actor, &id, None);
                    if let Some(a) = alias {
                        aliases.insert(a, id.clone());
                    }
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::SetSketch { id, x, y, w, h, strokes } => {
                    if matches!(actor, Actor::Agent { .. }) {
                        return Err("agents do not draw ink; use notes and arrows".to_string());
                    }
                    let (x, y, w, h) = (finite(x, "x")?, finite(y, "y")?, finite(w, "w")?.max(1.0), finite(h, "h")?.max(1.0));
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
                            };
                            let id = node.id.clone();
                            self.nodes.push(node);
                            self.record(actor, &id, None);
                            Ok(json!({ "ok": true, "id": id }))
                        }
                    }
                }
                Op::Update { id, text, tag } => {
                    let id = resolve(&aliases, &id);
                    let before = self.entity(&id);
                    let tag = tag.map(|t| check_tag(&t)).transpose()?;
                    let node = self.nodes.iter_mut().find(|n| n.id == id).ok_or_else(|| format!("no item {id}"))?;
                    if let Some(t) = text {
                        node.text = t;
                    }
                    if let Some(t) = tag {
                        node.tag = t;
                    }
                    self.record(actor, &id, before);
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::Move { id, x, y, w, h } => {
                    let id = resolve(&aliases, &id);
                    let before = self.entity(&id);
                    let node = self.nodes.iter_mut().find(|n| n.id == id).ok_or_else(|| format!("no item {id}"))?;
                    node.x = finite(x, "x")?;
                    node.y = finite(y, "y")?;
                    if let Some(w) = w {
                        node.w = finite(w, "w")?.clamp(1.0, 4000.0);
                    }
                    if let Some(h) = h {
                        node.h = finite(h, "h")?.clamp(1.0, 4000.0);
                    }
                    self.record(actor, &id, before);
                    Ok(json!({ "ok": true, "id": id }))
                }
                Op::Delete { ids } => {
                    let mut gone = Vec::new();
                    for raw in ids {
                        let id = resolve(&aliases, &raw);
                        if self.nodes.iter().any(|n| n.id == id) {
                            let attached: Vec<String> =
                                self.edges.iter().filter(|e| e.from == id || e.to == id).map(|e| e.id.clone()).collect();
                            for e in attached {
                                let before = self.entity(&e);
                                self.edges.retain(|x| x.id != e);
                                self.record(actor, &e, before);
                            }
                        }
                        let before = self.entity(&id).ok_or_else(|| format!("no item {id}"))?;
                        self.nodes.retain(|n| n.id != id);
                        self.edges.retain(|e| e.id != id);
                        self.record(actor, &id, Some(before));
                        gone.push(id);
                    }
                    Ok(json!({ "ok": true, "deleted": gone }))
                }
                Op::Connect { from, to, label } => {
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
            })
            .collect();
        let edges: Vec<Value> = self.edges.iter().map(|e| json!({ "id": e.id, "from": e.from, "to": e.to, "label": e.label })).collect();
        let pending: Vec<&str> = self.changes.iter().map(|c| c.target.as_str()).collect();
        json!({ "nodes": nodes, "edges": edges, "pending_suggestions": pending })
    }
}

// ----- the boards, cached from the store -----

/// Every open design's board.
#[derive(Default)]
pub struct Boards {
    docs: parking_lot::Mutex<HashMap<String, Doc>>,
}

impl Boards {
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
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

/// The board as it is now.
pub fn doc(state: &AppState, id: &str) -> Result<Doc, String> {
    if let Some(d) = state.boards.docs.lock().get(id) {
        return Ok(d.clone());
    }
    let d = load_doc(state, id)?;
    state.boards.docs.lock().insert(id.to_string(), d.clone());
    Ok(d)
}

/// What the window hears after every change.
#[derive(Clone, Serialize)]
struct DesignEvent<'a> {
    id: &'a str,
    doc: &'a Doc,
}

/// Run `f` on the board, save it, and tell the window.
fn edit<R>(app: &AppHandle, id: &str, f: impl FnOnce(&mut Doc) -> R) -> Result<R, String> {
    let state = app.state::<AppState>();
    let mut current = doc(&state, id)?;
    let out = f(&mut current);
    current.version += 1;
    let json = serde_json::to_string(&current).map_err(|e| e.to_string())?;
    state
        .store
        .update_artifact(id, &orchestra_store::ArtifactPatch { body: Some(json), ..Default::default() })
        .map_err(|e| e.to_string())?;
    state.boards.docs.lock().insert(id.to_string(), current.clone());
    let _ = app.emit("design", DesignEvent { id, doc: &current });
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
- 메모(note), 손그림(sketch), 화살표(edge)가 있습니다. 메모에는 태그를 붙일 수 있습니다: goal(목표), constraint(제약), question(미해결 질문), idea(아이디어).
- 매 메시지마다 보드 개요가 [board] 아래 JSON으로, 손그림이 있으면 보드 전체 그림이 첨부로 옵니다. 사람이 고른 항목은 selected로 옵니다.
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
- Notes, freehand sketches and arrows (edges). Notes can carry a tag: goal, constraint, question (open question) or idea.
- Every message brings an outline of the board as JSON under [board], and a picture of the whole board attached when it has sketches. Items the human selected come as selected.
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
pub const BOARD_READ_DESC: &str = "Read the design's board: every note (id, tag, text, position, size, who made it), sketch (position, size, stroke count) and arrow, and which items are suggestions still waiting on the human.";
pub const BOARD_WRITE: &str = "board_write";
pub const BOARD_WRITE_DESC: &str = "Change the design's board with a list of commands, applied in order. Each result says ok with the item id, or the error. create_note {x, y, text, tag?, w?, h?, ref?} (tag: goal | constraint | question | idea); update {id, text?, tag?}; move {id, x, y, w?, h?}; delete {ids}; connect {from, to, label?}. Ids may be \"$ref\" for a note made earlier in the same call. Your changes show as suggestions the human keeps or reverts.";

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
                        "op": { "type": "string", "enum": ["create_note", "update", "move", "delete", "connect"] },
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
                        "label": { "type": "string" }
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
