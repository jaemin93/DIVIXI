//! Designs: a whiteboard the human and an agent draw on together, kept as
//! an artifact beside tracks and attached to a track's conductor as
//! reference.
//!
//! The board is an Excalidraw scene: shapes (rectangle, ellipse, diamond)
//! with labels, arrows bound to them, lines, text, frames and the human's
//! freehand ink. The window runs Excalidraw itself; the core keeps the
//! scene (as the artifact's body), merges what the window saves with what
//! the agent writes, and gives the agent two tools: one reads the scene as
//! a short outline, one adds, changes and deletes elements.
//!
//! Elements are kept as Excalidraw writes them. The core reads only the
//! fields it needs and builds the elements the agent asks for with their
//! essentials; Excalidraw fills in the rest (and measures text) when it
//! loads them. Merging follows Excalidraw's own collaboration rule: of two
//! copies of an element the higher `version` wins, a tie goes to the lower
//! `versionNonce`, and deletions are kept as `isDeleted` tombstones.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orchestra_mcp::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// The artifact kind this module serves.
pub const KIND: &str = "design";

const MAX_OPS: usize = 200;
const MAX_ELEMENTS: usize = 5_000;
const MAX_TEXT: usize = 4_000;

/// Shape size when the agent gives none.
const SHAPE_W: f64 = 200.0;
const SHAPE_H: f64 = 100.0;
const FONT_SIZE: f64 = 20.0;
/// Excalifont, Excalidraw's hand-drawn face.
const FONT_FAMILY: i64 = 5;
const INK: &str = "#1e1e1e";

// ----- the scene -----

/// The whole board.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Bumped on every change the core keeps.
    #[serde(default)]
    pub version: u64,
    /// Excalidraw elements, deleted ones included (as tombstones).
    #[serde(default)]
    pub elements: Vec<Value>,
    /// Pictures on the board, by file id, as Excalidraw keeps them.
    #[serde(default)]
    pub files: Map<String, Value>,
}

fn s<'a>(e: &'a Value, key: &str) -> &'a str {
    e.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn n(e: &Value, key: &str) -> f64 {
    e.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn id_of(e: &Value) -> &str {
    s(e, "id")
}

fn is_deleted(e: &Value) -> bool {
    e.get("isDeleted").and_then(Value::as_bool).unwrap_or(false)
}

fn version_of(e: &Value) -> i64 {
    e.get("version").and_then(Value::as_i64).unwrap_or(0)
}

fn nonce_of(e: &Value) -> i64 {
    e.get("versionNonce").and_then(Value::as_i64).unwrap_or(0)
}

/// Whether `incoming` should replace `current` (Excalidraw's reconcile rule).
fn newer(incoming: &Value, current: &Value) -> bool {
    let (vi, vc) = (version_of(incoming), version_of(current));
    vi > vc || (vi == vc && nonce_of(incoming) < nonce_of(current))
}

/// A random positive 31-bit number, as Excalidraw's seeds and nonces are.
fn random_i32() -> i64 {
    let b = uuid::Uuid::new_v4().into_bytes();
    i64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 1)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Mark an element changed, the way Excalidraw does.
fn bump(e: &mut Value) {
    let v = version_of(e) + 1;
    e["version"] = json!(v);
    e["versionNonce"] = json!(random_i32());
    e["updated"] = json!(now_ms());
}

impl Scene {
    fn live(&self) -> impl Iterator<Item = &Value> {
        self.elements.iter().filter(|e| !is_deleted(e))
    }

    fn find(&self, id: &str) -> Option<usize> {
        self.elements.iter().position(|e| id_of(e) == id)
    }

    fn find_live(&self, id: &str) -> Option<usize> {
        self.find(id).filter(|&i| !is_deleted(&self.elements[i]))
    }

    /// A fresh element id no element has.
    fn fresh_id(&self, prefix: &str) -> String {
        loop {
            let id = format!("{prefix}{}", &uuid::Uuid::new_v4().simple().to_string()[..6]);
            if self.find(&id).is_none() {
                return id;
            }
        }
    }

    /// Fold another copy of the scene in, element by element: the newer
    /// copy of each element wins, elements only one side has are kept.
    /// Returns the elements that changed here.
    pub fn merge(&mut self, elements: Vec<Value>, files: Map<String, Value>) -> Vec<Value> {
        let mut changed = Vec::new();
        let at: HashMap<String, usize> = self.elements.iter().enumerate().map(|(i, e)| (id_of(e).to_string(), i)).collect();
        for e in elements {
            let id = id_of(&e).to_string();
            if id.is_empty() {
                continue;
            }
            match at.get(&id) {
                Some(&i) => {
                    if newer(&e, &self.elements[i]) {
                        self.elements[i] = e.clone();
                        changed.push(e);
                    }
                }
                None => {
                    self.elements.push(e.clone());
                    changed.push(e);
                }
            }
        }
        for (k, v) in files {
            self.files.entry(k).or_insert(v);
        }
        changed
    }

    /// The text bound to a container (a shape's or an arrow's label).
    fn label_of(&self, container: &str) -> Option<usize> {
        self.elements.iter().position(|e| !is_deleted(e) && s(e, "type") == "text" && s(e, "containerId") == container)
    }

    fn label_text(&self, container: &str) -> String {
        self.label_of(container).map(|i| s(&self.elements[i], "text").to_string()).unwrap_or_default()
    }

    /// The board as the agent reads it: one line per element, labels
    /// inline, bound text folded into its container.
    pub fn outline(&self) -> String {
        let live: Vec<&Value> = self.live().collect();
        if live.is_empty() {
            return "The board is empty.".to_string();
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for e in &live {
            x0 = x0.min(n(e, "x"));
            y0 = y0.min(n(e, "y"));
            x1 = x1.max(n(e, "x") + n(e, "width"));
            y1 = y1.max(n(e, "y") + n(e, "height"));
        }
        let mut out = format!(
            "{} elements; everything lies within x {:.0}..{:.0}, y {:.0}..{:.0}.",
            live.len(),
            x0,
            x1,
            y0,
            y1
        );
        let clip = |t: &str| {
            let t = t.replace('\n', " / ");
            if t.chars().count() > 200 {
                format!("{}…", t.chars().take(200).collect::<String>())
            } else {
                t
            }
        };
        for e in &live {
            let kind = s(e, "type");
            if kind == "text" && !s(e, "containerId").is_empty() {
                continue; // shown with its container
            }
            let id = id_of(e);
            let geo = format!("at ({:.0},{:.0}) {:.0}x{:.0}", n(e, "x"), n(e, "y"), n(e, "width"), n(e, "height"));
            let mut line = match kind {
                "arrow" | "line" => {
                    let from = e.pointer("/startBinding/elementId").and_then(Value::as_str).unwrap_or("");
                    let to = e.pointer("/endBinding/elementId").and_then(Value::as_str).unwrap_or("");
                    if !from.is_empty() || !to.is_empty() {
                        format!("{id} {kind} {} -> {}", if from.is_empty() { "?" } else { from }, if to.is_empty() { "?" } else { to })
                    } else {
                        format!("{id} {kind} {geo}")
                    }
                }
                "text" => format!("{id} text \"{}\" {geo}", clip(s(e, "text"))),
                "frame" => {
                    let members: Vec<&str> = live.iter().filter(|m| s(m, "frameId") == id).map(|m| id_of(m)).collect();
                    format!("{id} frame \"{}\" {geo} holds [{}]", clip(s(e, "name")), members.join(", "))
                }
                "freedraw" => format!("{id} freehand ink {geo}"),
                "image" => format!("{id} image {geo}"),
                _ => format!("{id} {kind} {geo}"),
            };
            if kind != "text" && kind != "frame" {
                let label = self.label_text(id);
                if !label.is_empty() {
                    line.push_str(&format!(" \"{}\"", clip(&label)));
                }
            }
            let bg = s(e, "backgroundColor");
            if !bg.is_empty() && bg != "transparent" && kind != "text" {
                line.push_str(&format!(" fill {bg}"));
            }
            let frame = s(e, "frameId");
            if !frame.is_empty() && kind != "frame" {
                line.push_str(&format!(" in {frame}"));
            }
            if e.pointer("/customData/by").and_then(Value::as_str) == Some("agent") {
                line.push_str(" (yours)");
            }
            out.push('\n');
            out.push_str(&line);
        }
        out
    }

    /// The board as a brief to attach to a track: title, then what the
    /// shapes and notes say and how the arrows connect them.
    pub fn brief(&self, title: &str) -> String {
        let mut out = format!("# {title}\n\nA design board, drawn in Excalidraw. Its picture is attached beside this brief.\n");
        let named = |id: &str| {
            let l = self.label_text(id);
            let l = if l.is_empty() {
                self.find_live(id).map(|i| s(&self.elements[i], "text").to_string()).unwrap_or_default()
            } else {
                l
            };
            if l.is_empty() { id.to_string() } else { l.replace('\n', " ") }
        };
        let mut items = Vec::new();
        let mut links = Vec::new();
        for e in self.live() {
            let id = id_of(e);
            match s(e, "type") {
                "arrow" | "line" => {
                    let from = e.pointer("/startBinding/elementId").and_then(Value::as_str);
                    let to = e.pointer("/endBinding/elementId").and_then(Value::as_str);
                    if let (Some(f), Some(t)) = (from, to) {
                        let label = self.label_text(id);
                        let label = if label.is_empty() { String::new() } else { format!(" ({})", label.replace('\n', " ")) };
                        links.push(format!("- {} → {}{label}", named(f), named(t)));
                    }
                }
                "text" if s(e, "containerId").is_empty() => items.push(format!("- {}", s(e, "text").replace('\n', " "))),
                "frame" => items.push(format!("- **{}** (frame)", s(e, "name"))),
                "rectangle" | "ellipse" | "diamond" => {
                    let l = self.label_text(id);
                    if !l.is_empty() {
                        items.push(format!("- {}", l.replace('\n', " ")));
                    }
                }
                _ => {}
            }
        }
        if !items.is_empty() {
            out.push_str("\n## On the board\n\n");
            out.push_str(&items.join("\n"));
            out.push('\n');
        }
        if !links.is_empty() {
            out.push_str("\n## Connections\n\n");
            out.push_str(&links.join("\n"));
            out.push('\n');
        }
        out
    }
}

// ----- building elements -----

fn base(id: &str, kind: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    let rounded = matches!(kind, "rectangle" | "diamond" | "frame");
    json!({
        "id": id,
        "type": kind,
        "x": x, "y": y, "width": w, "height": h,
        "angle": 0,
        "strokeColor": INK,
        "backgroundColor": "transparent",
        "fillStyle": "solid",
        "strokeWidth": 2,
        "strokeStyle": "solid",
        "roughness": 1,
        "opacity": 100,
        "groupIds": [],
        "frameId": null,
        "roundness": if rounded && kind != "frame" { json!({ "type": 3 }) } else { Value::Null },
        "seed": random_i32(),
        "version": 1,
        "versionNonce": random_i32(),
        "isDeleted": false,
        "boundElements": [],
        "updated": now_ms(),
        "link": null,
        "locked": false,
        "customData": { "by": "agent" },
    })
}

/// A text's box, roughly; Excalidraw measures it exactly when it loads.
fn text_box(text: &str, size: f64) -> (f64, f64) {
    let lines: Vec<&str> = text.split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().map(|c| if c.is_ascii() { 0.55 } else { 1.0 }).sum::<f64>()).fold(0.0, f64::max);
    (widest * size + 4.0, lines.len() as f64 * size * 1.25)
}

fn text_element(id: &str, text: &str, x: f64, y: f64, size: f64, container: Option<&str>) -> Value {
    let (w, h) = text_box(text, size);
    let mut e = base(id, "text", x, y, w, h);
    e["text"] = json!(text);
    e["originalText"] = json!(text);
    e["fontSize"] = json!(size);
    e["fontFamily"] = json!(FONT_FAMILY);
    e["lineHeight"] = json!(1.25);
    e["autoResize"] = json!(true);
    e["textAlign"] = json!(if container.is_some() { "center" } else { "left" });
    e["verticalAlign"] = json!(if container.is_some() { "middle" } else { "top" });
    e["containerId"] = json!(container);
    e
}

/// A label centred in `container` (a shape, or an arrow at its middle).
fn label_for(scene: &Scene, container: &Value, text: &str) -> Value {
    let id = scene.fresh_id("t");
    let (w, h) = text_box(text, FONT_SIZE);
    let (cx, cy) = if matches!(s(container, "type"), "arrow" | "line") {
        let pts = container.get("points").and_then(Value::as_array).cloned().unwrap_or_default();
        let last = pts.last().and_then(Value::as_array).map(|p| (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0))).unwrap_or((0.0, 0.0));
        (n(container, "x") + last.0 / 2.0, n(container, "y") + last.1 / 2.0)
    } else {
        (n(container, "x") + n(container, "width") / 2.0, n(container, "y") + n(container, "height") / 2.0)
    };
    text_element(&id, text, cx - w / 2.0, cy - h / 2.0, FONT_SIZE, Some(id_of(container)))
}

fn add_bound(e: &mut Value, id: &str, kind: &str) {
    let list = e.get_mut("boundElements");
    match list.and_then(Value::as_array_mut) {
        Some(a) => {
            if !a.iter().any(|b| s(b, "id") == id) {
                a.push(json!({ "id": id, "type": kind }));
            }
        }
        None => e["boundElements"] = json!([{ "id": id, "type": kind }]),
    }
}

/// Where the line from a box's centre towards `(tx, ty)` leaves the box,
/// pushed out by `gap`.
fn edge_point(e: &Value, tx: f64, ty: f64, gap: f64) -> (f64, f64) {
    let (x, y, w, h) = (n(e, "x"), n(e, "y"), n(e, "width").max(1.0), n(e, "height").max(1.0));
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (dx, dy) = (tx - cx, ty - cy);
    if dx == 0.0 && dy == 0.0 {
        return (cx, cy);
    }
    let sx = if dx != 0.0 { (w / 2.0) / dx.abs() } else { f64::INFINITY };
    let sy = if dy != 0.0 { (h / 2.0) / dy.abs() } else { f64::INFINITY };
    let t = sx.min(sy);
    let len = (dx * dx + dy * dy).sqrt();
    (cx + dx * t + dx / len * gap, cy + dy * t + dy / len * gap)
}

fn centre(e: &Value) -> (f64, f64) {
    (n(e, "x") + n(e, "width") / 2.0, n(e, "y") + n(e, "height") / 2.0)
}

/// Lay an arrow between the two elements it is bound to.
fn route(arrow: &mut Value, from: &Value, to: &Value) {
    let (a, b) = (centre(from), centre(to));
    let start = edge_point(from, b.0, b.1, 6.0);
    let end = edge_point(to, a.0, a.1, 6.0);
    arrow["x"] = json!(start.0);
    arrow["y"] = json!(start.1);
    arrow["width"] = json!((end.0 - start.0).abs());
    arrow["height"] = json!((end.1 - start.1).abs());
    arrow["points"] = json!([[0.0, 0.0], [end.0 - start.0, end.1 - start.1]]);
}

/// What the agent may draw.
const SHAPES: [&str; 3] = ["rectangle", "ellipse", "diamond"];

/// One command of `board_write`.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A new element. `ref` names it for later commands in the same call (`$ref`).
    Add {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        /// A shape's or an arrow's label; a text element's text; a frame's name.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        text: Option<String>,
        /// Arrows and lines: the elements they join.
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        to: Option<String>,
        /// Arrows and lines not joining elements: absolute points.
        #[serde(default)]
        points: Option<Vec<[f64; 2]>>,
        #[serde(default, rename = "strokeColor")]
        stroke_color: Option<String>,
        #[serde(default, rename = "backgroundColor")]
        background_color: Option<String>,
        #[serde(default, rename = "strokeStyle")]
        stroke_style: Option<String>,
        #[serde(default, rename = "fontSize")]
        font_size: Option<f64>,
        /// The frame to put it in.
        #[serde(default)]
        frame: Option<String>,
        #[serde(default, rename = "ref")]
        alias: Option<String>,
    },
    Update {
        id: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default, rename = "strokeColor")]
        stroke_color: Option<String>,
        #[serde(default, rename = "backgroundColor")]
        background_color: Option<String>,
        #[serde(default, rename = "strokeStyle")]
        stroke_style: Option<String>,
        #[serde(default)]
        frame: Option<String>,
    },
    /// Elements by id; a shape's label goes with it, arrows on it are let go.
    Delete { ids: Vec<String> },
}

fn check_text(t: &str) -> Result<(), String> {
    if t.chars().count() > MAX_TEXT {
        return Err(format!("text longer than {MAX_TEXT} characters"));
    }
    Ok(())
}

fn finite(v: f64, what: &str) -> Result<f64, String> {
    if v.is_finite() && v.abs() < 1e7 {
        Ok(v)
    } else {
        Err(format!("{what} is not a usable number"))
    }
}

fn check_color(c: &str) -> Result<String, String> {
    let ok = c == "transparent" || (c.starts_with('#') && matches!(c.len(), 4 | 7 | 9) && c[1..].chars().all(|x| x.is_ascii_hexdigit()));
    if ok {
        Ok(c.to_string())
    } else {
        Err(format!("colour {c:?} is not #rgb, #rrggbb or transparent"))
    }
}

fn check_style(st: &str) -> Result<String, String> {
    if matches!(st, "solid" | "dashed" | "dotted") {
        Ok(st.to_string())
    } else {
        Err(format!("strokeStyle {st:?} is not solid, dashed or dotted"))
    }
}

impl Scene {
    /// Run the agent's commands in order, each on its own: one that fails
    /// says why and the rest still run. Returns one result per command and
    /// the elements that changed.
    pub fn apply(&mut self, ops: Vec<Op>) -> (Vec<Value>, Vec<Value>) {
        let mut refs: HashMap<String, String> = HashMap::new();
        let mut touched: HashSet<String> = HashSet::new();
        let mut results = Vec::new();
        if ops.len() > MAX_OPS {
            return (vec![json!({ "ok": false, "error": format!("at most {MAX_OPS} commands per call") })], Vec::new());
        }
        for op in ops {
            let r = self.apply_one(op, &mut refs, &mut touched);
            results.push(match r {
                Ok(id) => json!({ "ok": true, "id": id }),
                Err(e) => json!({ "ok": false, "error": e }),
            });
        }
        let changed = self.elements.iter().filter(|e| touched.contains(id_of(e))).cloned().collect();
        (results, changed)
    }

    fn resolve(&self, id: &str, refs: &HashMap<String, String>) -> Result<String, String> {
        let real = match id.strip_prefix('$') {
            Some(r) => refs.get(r).cloned().ok_or_else(|| format!("no ${r} earlier in this call"))?,
            None => id.to_string(),
        };
        self.find_live(&real).map(|_| real.clone()).ok_or_else(|| format!("no element {real}"))
    }

    fn apply_one(&mut self, op: Op, refs: &mut HashMap<String, String>, touched: &mut HashSet<String>) -> Result<String, String> {
        match op {
            Op::Add { kind, x, y, width, height, label, text, from, to, points, stroke_color, background_color, stroke_style, font_size, frame, alias } => {
                if self.elements.len() >= MAX_ELEMENTS {
                    return Err(format!("the board holds at most {MAX_ELEMENTS} elements"));
                }
                let words = label.clone().or(text.clone()).unwrap_or_default();
                check_text(&words)?;
                let frame = frame.map(|f| self.resolve(&f, refs)).transpose()?;
                let prefix = match kind.as_str() {
                    "rectangle" => "r",
                    "ellipse" => "e",
                    "diamond" => "d",
                    "text" => "t",
                    "arrow" => "a",
                    "line" => "l",
                    "frame" => "f",
                    other => return Err(format!("cannot add a {other}; use rectangle, ellipse, diamond, text, arrow, line or frame")),
                };
                let id = self.fresh_id(prefix);
                let mut e = match kind.as_str() {
                    "arrow" | "line" => {
                        let mut e = base(&id, &kind, 0.0, 0.0, 0.0, 0.0);
                        e["points"] = json!([[0.0, 0.0], [0.0, 0.0]]);
                        e["lastCommittedPoint"] = Value::Null;
                        e["startArrowhead"] = Value::Null;
                        e["endArrowhead"] = if kind == "arrow" { json!("arrow") } else { Value::Null };
                        e["elbowed"] = json!(false);
                        match (&from, &to, &points) {
                            (Some(f), Some(t), _) => {
                                let (f, t) = (self.resolve(f, refs)?, self.resolve(t, refs)?);
                                if f == t {
                                    return Err("an arrow needs two different ends".to_string());
                                }
                                let (fi, ti) = (self.find_live(&f).unwrap_or(0), self.find_live(&t).unwrap_or(0));
                                let (fe, te) = (self.elements[fi].clone(), self.elements[ti].clone());
                                route(&mut e, &fe, &te);
                                e["startBinding"] = json!({ "elementId": f, "focus": 0, "gap": 6 });
                                e["endBinding"] = json!({ "elementId": t, "focus": 0, "gap": 6 });
                                for (i, other) in [(fi, &f), (ti, &t)] {
                                    add_bound(&mut self.elements[i], &id, "arrow");
                                    bump(&mut self.elements[i]);
                                    touched.insert(other.clone());
                                }
                            }
                            (_, _, Some(pts)) if pts.len() >= 2 => {
                                let (x0, y0) = (finite(pts[0][0], "x")?, finite(pts[0][1], "y")?);
                                let rel: Vec<[f64; 2]> = pts.iter().map(|p| [p[0] - x0, p[1] - y0]).collect();
                                let (mut w, mut h) = (0.0f64, 0.0f64);
                                for p in &rel {
                                    w = w.max(p[0].abs());
                                    h = h.max(p[1].abs());
                                }
                                e["x"] = json!(x0);
                                e["y"] = json!(y0);
                                e["width"] = json!(w);
                                e["height"] = json!(h);
                                e["points"] = json!(rel);
                            }
                            _ => return Err(format!("a {kind} needs from and to (element ids), or at least two points")),
                        }
                        e
                    }
                    "text" => {
                        if words.trim().is_empty() {
                            return Err("a text element needs text".to_string());
                        }
                        let (x, y) = (finite(x.unwrap_or(0.0), "x")?, finite(y.unwrap_or(0.0), "y")?);
                        text_element(&id, &words, x, y, font_size.unwrap_or(FONT_SIZE).clamp(8.0, 120.0), None)
                    }
                    _ => {
                        let (x, y) = (finite(x.ok_or("x is required")?, "x")?, finite(y.ok_or("y is required")?, "y")?);
                        let (w, h) = (finite(width.unwrap_or(SHAPE_W), "width")?.max(10.0), finite(height.unwrap_or(SHAPE_H), "height")?.max(10.0));
                        let mut e = base(&id, &kind, x, y, w, h);
                        if kind == "frame" {
                            e["name"] = json!(words);
                        }
                        e
                    }
                };
                if let Some(c) = stroke_color {
                    e["strokeColor"] = json!(check_color(&c)?);
                }
                if let Some(c) = background_color {
                    e["backgroundColor"] = json!(check_color(&c)?);
                }
                if let Some(st) = stroke_style {
                    e["strokeStyle"] = json!(check_style(&st)?);
                }
                if let Some(f) = &frame {
                    e["frameId"] = json!(f);
                }
                let labelled = SHAPES.contains(&kind.as_str()) || kind == "arrow";
                let label_el = (labelled && !words.trim().is_empty()).then(|| label_for(self, &e, &words));
                if let Some(l) = &label_el {
                    add_bound(&mut e, id_of(l), "text");
                }
                self.elements.push(e);
                touched.insert(id.clone());
                if let Some(mut l) = label_el {
                    if let Some(f) = &frame {
                        l["frameId"] = json!(f);
                    }
                    touched.insert(id_of(&l).to_string());
                    self.elements.push(l);
                }
                if let Some(a) = alias {
                    refs.insert(a, id.clone());
                }
                Ok(id)
            }
            Op::Update { id, label, text, x, y, width, height, stroke_color, background_color, stroke_style, frame } => {
                let id = self.resolve(&id, refs)?;
                let i = self.find_live(&id).ok_or_else(|| format!("no element {id}"))?;
                let kind = s(&self.elements[i], "type").to_string();
                let frame = frame.map(|f| if f.is_empty() { Ok(String::new()) } else { self.resolve(&f, refs) }).transpose()?;
                let (old_x, old_y) = (n(&self.elements[i], "x"), n(&self.elements[i], "y"));
                {
                    let e = &mut self.elements[i];
                    if let Some(v) = x {
                        e["x"] = json!(finite(v, "x")?);
                    }
                    if let Some(v) = y {
                        e["y"] = json!(finite(v, "y")?);
                    }
                    if let Some(v) = width {
                        e["width"] = json!(finite(v, "width")?.max(10.0));
                    }
                    if let Some(v) = height {
                        e["height"] = json!(finite(v, "height")?.max(10.0));
                    }
                    if let Some(c) = stroke_color {
                        e["strokeColor"] = json!(check_color(&c)?);
                    }
                    if let Some(c) = background_color {
                        e["backgroundColor"] = json!(check_color(&c)?);
                    }
                    if let Some(st) = stroke_style {
                        e["strokeStyle"] = json!(check_style(&st)?);
                    }
                    if let Some(f) = &frame {
                        e["frameId"] = if f.is_empty() { Value::Null } else { json!(f) };
                    }
                    bump(e);
                }
                touched.insert(id.clone());
                let words = if kind == "text" { text.or(label) } else { label.or(text) };
                if let Some(w) = words {
                    check_text(&w)?;
                    if kind == "text" {
                        let e = &mut self.elements[i];
                        e["text"] = json!(w);
                        e["originalText"] = json!(w);
                    } else if kind == "frame" {
                        self.elements[i]["name"] = json!(w);
                    } else {
                        match self.label_of(&id) {
                            Some(li) => {
                                let l = &mut self.elements[li];
                                l["text"] = json!(w);
                                l["originalText"] = json!(w);
                                bump(l);
                                touched.insert(id_of(l).to_string());
                            }
                            None if !w.trim().is_empty() => {
                                let l = label_for(self, &self.elements[i], &w);
                                let lid = id_of(&l).to_string();
                                add_bound(&mut self.elements[i], &lid, "text");
                                self.elements.push(l);
                                touched.insert(lid);
                            }
                            None => {}
                        }
                    }
                }
                // What moves with it: its label, and the arrows bound to it.
                let (dx, dy) = (n(&self.elements[i], "x") - old_x, n(&self.elements[i], "y") - old_y);
                if let Some(li) = self.label_of(&id) {
                    if dx != 0.0 || dy != 0.0 {
                        let l = &mut self.elements[li];
                        l["x"] = json!(n(l, "x") + dx);
                        l["y"] = json!(n(l, "y") + dy);
                        bump(l);
                        touched.insert(id_of(l).to_string());
                    }
                }
                self.reroute_arrows_on(&id, touched);
                Ok(id)
            }
            Op::Delete { ids } => {
                let mut gone = Vec::new();
                for raw in &ids {
                    let id = match self.resolve(raw, refs) {
                        Ok(id) => id,
                        // Already gone is what was asked.
                        Err(_) => continue,
                    };
                    gone.push(id);
                }
                for id in &gone {
                    if let Some(i) = self.find_live(id) {
                        self.elements[i]["isDeleted"] = json!(true);
                        bump(&mut self.elements[i]);
                        touched.insert(id.clone());
                    }
                    if let Some(li) = self.label_of(id) {
                        self.elements[li]["isDeleted"] = json!(true);
                        bump(&mut self.elements[li]);
                        touched.insert(id_of(&self.elements[li]).to_string());
                    }
                    // Arrows on it let go of it.
                    for e in self.elements.iter_mut().filter(|e| !is_deleted(e)) {
                        let mut let_go = false;
                        for end in ["startBinding", "endBinding"] {
                            if e.pointer(&format!("/{end}/elementId")).and_then(Value::as_str) == Some(id.as_str()) {
                                e[end] = Value::Null;
                                let_go = true;
                            }
                        }
                        if let_go {
                            bump(e);
                            touched.insert(id_of(e).to_string());
                        }
                    }
                }
                Ok(gone.join(","))
            }
        }
    }

    /// Lay out again the arrows bound at both ends that touch `id`.
    fn reroute_arrows_on(&mut self, id: &str, touched: &mut HashSet<String>) {
        let arrows: Vec<usize> = self
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                !is_deleted(e)
                    && matches!(s(e, "type"), "arrow" | "line")
                    && (e.pointer("/startBinding/elementId").and_then(Value::as_str) == Some(id)
                        || e.pointer("/endBinding/elementId").and_then(Value::as_str) == Some(id))
            })
            .map(|(i, _)| i)
            .collect();
        for ai in arrows {
            let a = &self.elements[ai];
            let (Some(f), Some(t)) = (
                a.pointer("/startBinding/elementId").and_then(Value::as_str).and_then(|f| self.find_live(f)),
                a.pointer("/endBinding/elementId").and_then(Value::as_str).and_then(|t| self.find_live(t)),
            ) else {
                continue;
            };
            let (fe, te) = (self.elements[f].clone(), self.elements[t].clone());
            let arrow = &mut self.elements[ai];
            route(arrow, &fe, &te);
            bump(arrow);
            touched.insert(id_of(arrow).to_string());
            let aid = id_of(arrow).to_string();
            if let Some(li) = self.label_of(&aid) {
                let (x, y, pts) = (n(&self.elements[ai], "x"), n(&self.elements[ai], "y"), self.elements[ai]["points"].clone());
                let (px, py) = pts.get(1).and_then(Value::as_array).map(|p| (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0))).unwrap_or((0.0, 0.0));
                let l = &mut self.elements[li];
                l["x"] = json!(x + px / 2.0 - n(l, "width") / 2.0);
                l["y"] = json!(y + py / 2.0 - n(l, "height") / 2.0);
                bump(l);
                touched.insert(id_of(l).to_string());
            }
        }
    }
}

// ----- boards made before Excalidraw -----

/// The sketch board a design was before it was Excalidraw: notes (tagged
/// goal, constraint, question or idea), freehand sketches and arrows. Read
/// once, when such a design is opened, and turned into a scene.
mod legacy {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Stroke {
        pub points: Vec<[f64; 3]>,
        #[serde(default)]
        pub color: String,
        #[serde(default)]
        pub size: Option<f64>,
    }

    #[derive(Deserialize)]
    pub struct Node {
        pub id: String,
        pub kind: String,
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
        #[serde(default)]
        pub text: String,
        #[serde(default)]
        pub tag: String,
        #[serde(default)]
        pub strokes: Vec<Stroke>,
    }

    #[derive(Deserialize)]
    pub struct Edge {
        pub id: String,
        pub from: String,
        pub to: String,
        #[serde(default)]
        pub label: String,
    }

    #[derive(Deserialize)]
    pub struct Doc {
        #[serde(default)]
        pub version: u64,
        #[serde(default)]
        pub nodes: Vec<Node>,
        #[serde(default)]
        pub edges: Vec<Edge>,
    }
}

/// A note's fill by its old tag.
fn tag_fill(tag: &str) -> &'static str {
    match tag {
        "goal" => "#b2f2bb",
        "constraint" => "#ffd8a8",
        "question" => "#d0bfff",
        "idea" => "#ffec99",
        _ => "#e9ecef",
    }
}

fn from_legacy(old: legacy::Doc) -> Scene {
    let mut scene = Scene { version: old.version + 1, ..Default::default() };
    for node in &old.nodes {
        if node.kind == "sketch" {
            for (k, st) in node.strokes.iter().enumerate() {
                if st.points.is_empty() {
                    continue;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in &st.points {
                    x0 = x0.min(p[0]);
                    y0 = y0.min(p[1]);
                    x1 = x1.max(p[0]);
                    y1 = y1.max(p[1]);
                }
                let mut e = base(&format!("{}-{k}", node.id), "freedraw", node.x + x0, node.y + y0, x1 - x0, y1 - y0);
                e["points"] = json!(st.points.iter().map(|p| [p[0] - x0, p[1] - y0]).collect::<Vec<_>>());
                e["pressures"] = json!(st.points.iter().map(|p| p[2]).collect::<Vec<_>>());
                e["simulatePressure"] = json!(false);
                e["strokeWidth"] = json!(st.size.unwrap_or(4.0) / 2.0);
                if !st.color.is_empty() {
                    e["strokeColor"] = json!(st.color);
                }
                e["customData"] = json!({ "by": "human" });
                scene.elements.push(e);
            }
            continue;
        }
        let mut e = base(&node.id, "rectangle", node.x, node.y, node.w, node.h);
        e["backgroundColor"] = json!(tag_fill(&node.tag));
        e["customData"] = json!({});
        if !node.text.trim().is_empty() {
            let text = if node.tag.is_empty() { node.text.clone() } else { format!("[{}] {}", node.tag, node.text) };
            let l = label_for(&scene, &e, &text);
            add_bound(&mut e, id_of(&l), "text");
            scene.elements.push(e);
            scene.elements.push(l);
        } else {
            scene.elements.push(e);
        }
    }
    for edge in &old.edges {
        let (Some(fi), Some(ti)) = (scene.find(&edge.from), scene.find(&edge.to)) else { continue };
        let mut a = base(&edge.id, "arrow", 0.0, 0.0, 0.0, 0.0);
        a["endArrowhead"] = json!("arrow");
        a["startArrowhead"] = Value::Null;
        a["elbowed"] = json!(false);
        let (fe, te) = (scene.elements[fi].clone(), scene.elements[ti].clone());
        route(&mut a, &fe, &te);
        a["startBinding"] = json!({ "elementId": edge.from, "focus": 0, "gap": 6 });
        a["endBinding"] = json!({ "elementId": edge.to, "focus": 0, "gap": 6 });
        a["customData"] = json!({});
        add_bound(&mut scene.elements[fi], &edge.id, "arrow");
        add_bound(&mut scene.elements[ti], &edge.id, "arrow");
        let label = (!edge.label.is_empty()).then(|| label_for(&scene, &a, &edge.label));
        if let Some(l) = &label {
            add_bound(&mut a, id_of(l), "text");
        }
        scene.elements.push(a);
        if let Some(l) = label {
            scene.elements.push(l);
        }
    }
    scene
}

/// A design's body as a scene: an empty body is a new board, an old sketch
/// board is converted, anything else that does not read is an error (never
/// an empty board a later save would write over the real one).
pub fn parse_body(raw: &str) -> Result<Scene, String> {
    if raw.trim().is_empty() {
        return Ok(Scene::default());
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if v.get("nodes").is_some() && v.get("elements").is_none() {
        let old: legacy::Doc = serde_json::from_value(v).map_err(|e| e.to_string())?;
        return Ok(from_legacy(old));
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

// ----- the boards, cached from the store -----

/// One board, loaded on first use. Its lock is held for a whole edit
/// (read, change, save), so a save from the window and the agent's tool
/// call never start from the same state and overwrite each other.
type Slot = Arc<parking_lot::Mutex<Option<Scene>>>;

/// Every open design's board.
#[derive(Default)]
pub struct Boards {
    scenes: parking_lot::Mutex<HashMap<String, Slot>>,
}

impl Boards {
    fn slot(&self, id: &str) -> Slot {
        self.scenes.lock().entry(id.to_string()).or_default().clone()
    }

    /// Forget a board (its artifact is gone).
    pub fn forget(&self, id: &str) {
        self.scenes.lock().remove(id);
    }
}

fn load_scene(state: &AppState, id: &str) -> Result<Scene, String> {
    let (info, raw) = state.store.artifact(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no design {id}"))?;
    if info.kind != KIND {
        return Err(format!("{id} is not a design"));
    }
    parse_body(&raw).map_err(|e| format!("design {id} could not be read: {e}"))
}

/// The board as it is now.
pub fn scene(state: &AppState, id: &str) -> Result<Scene, String> {
    let slot = state.boards.slot(id);
    let mut board = slot.lock();
    if board.is_none() {
        *board = Some(load_scene(state, id)?);
    }
    board.clone().ok_or_else(|| format!("design {id} vanished"))
}

/// What the window hears after the agent changed the board: the elements
/// that changed, to reconcile into what it shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DesignDelta {
    pub id: String,
    pub version: u64,
    pub elements: Vec<Value>,
}

/// Run `f` on the board and save it when it changed anything. The board's
/// lock is held from reading it to keeping the result. `f` returns its
/// result and the elements it changed.
fn edit<R>(app: &AppHandle, id: &str, f: impl FnOnce(&mut Scene) -> (R, Vec<Value>)) -> Result<(R, Vec<Value>, u64), String> {
    let state = app.state::<AppState>();
    let slot = state.boards.slot(id);
    let mut board = slot.lock();
    if board.is_none() {
        *board = Some(load_scene(&state, id)?);
    }
    let old = board.take().ok_or_else(|| format!("design {id} vanished"))?;
    let mut current = old.clone();
    let (out, changed) = f(&mut current);
    if changed.is_empty() {
        let version = old.version;
        *board = Some(old);
        return Ok((out, changed, version));
    }
    current.version += 1;
    let saved = serde_json::to_string(&current).map_err(|e| e.to_string()).and_then(|json| {
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
    let version = current.version;
    *board = Some(current);
    Ok((out, changed, version))
}

/// Keep what the window saved: its elements merged in, element by element.
pub fn save(app: &AppHandle, id: &str, elements: Vec<Value>, files: Map<String, Value>) -> Result<u64, String> {
    if elements.len() > MAX_ELEMENTS * 2 {
        return Err(format!("the board holds at most {MAX_ELEMENTS} elements"));
    }
    edit(app, id, |sc| {
        let files_before = sc.files.len();
        let mut changed = sc.merge(elements, files);
        // New pictures alone are a change worth keeping too.
        if changed.is_empty() && sc.files.len() != files_before {
            changed.push(Value::Null);
        }
        ((), changed)
    })
    .map(|(_, _, v)| v)
}

/// Run the agent's commands and show the window what changed.
pub fn apply(app: &AppHandle, id: &str, ops: Vec<Op>) -> Result<Vec<Value>, String> {
    let (results, changed, version) = edit(app, id, |sc| sc.apply(ops))?;
    if !changed.is_empty() {
        let _ = app.emit("design", &DesignDelta { id: id.to_string(), version, elements: changed });
    }
    Ok(results)
}

/// What goes with each message: the board as an outline and the elements
/// the human picked on it.
pub fn context(state: &AppState, id: &str, selected: &[String]) -> Result<String, String> {
    let mut out = format!("[board]\n{}", scene(state, id)?.outline());
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
            r#"당신은 Divixi의 디자인 파트너입니다. 사람과 함께 Excalidraw 화이트보드에서 "{title}"의 설계를 그립니다. 이 디자인은 나중에 트랙의 지휘자에게 참고 자료(그림과 요약)로 첨부됩니다.

보드:
- Excalidraw 장면입니다: 도형(rectangle, ellipse, diamond)과 그 안의 라벨, 도형에 붙는 화살표(arrow)와 선(line), 글자(text), 묶음 틀(frame), 사람의 손그림(freedraw), 이미지.
- 매 메시지마다 보드 개요가 [board] 아래 한 줄씩(id, 종류, 위치·크기, 라벨, 화살표의 from -> to) 오고, 보드 그림이 첨부로 옵니다. 사람이 고른 요소는 selected로 옵니다. 최신 상태가 필요하면 `board_read`.
- 그릴 때는 `board_write`를 씁니다. 한 번에 여러 명령을 보내고, add에 ref를 주면 같은 호출 안에서 "$ref"로 가리킬 수 있습니다(화살표의 from/to, frame).
- 당신이 그린 것은 바로 보드에 나타나고, 사람은 Ctrl+Z로 되돌릴 수 있습니다. 사람이 지운 것을 다시 그리지 마세요.

그리는 법:
- 개념·화면·구성요소는 라벨 달린 도형으로(기본 200×100), 흐름·의존·호출은 from/to로 도형에 붙인 화살표로, 관계 이름은 화살표 label로.
- 결정 지점은 diamond, 시작/끝이나 사람·외부 시스템은 ellipse, 나머지는 rectangle.
- 관련 요소는 frame으로 묶고 이름을 붙입니다(frame을 먼저 add하고 요소에 frame: "$ref").
- 배치: 격자처럼 정렬, 간격 60 이상, 흐름은 왼쪽에서 오른쪽 또는 위에서 아래로. 개요의 범위를 보고 빈 자리에 그리며 기존 요소와 겹치지 않게 합니다.
- 색은 의미가 있을 때만: 목표 #b2f2bb, 제약 #ffd8a8, 미해결 질문 #d0bfff, 아이디어 #ffec99 (backgroundColor).
- 라벨은 짧게. 긴 설명은 text 요소나 대화로.
- 사람의 손그림은 의도를 담고 있습니다. 그림을 읽고, 모호하면 추측하지 말고 묻습니다.
- 목표, 제약, 미해결 질문을 드러내는 것이 목적입니다. 설계가 트랙에 넘겨도 될 만큼 분명해지면 그렇다고 말합니다.
- 대화 답은 짧게. 보드에 그린 것을 대화에서 되풀이하지 않습니다. 한국어로 말합니다.
"#
        );
    }
    format!(
        r#"You are Divixi's design partner. With the human you draw the design of "{title}" on an Excalidraw whiteboard. The design will later be attached to a track's conductor as reference (a picture and a brief).

The board:
- An Excalidraw scene: shapes (rectangle, ellipse, diamond) with labels inside, arrows and lines bound to shapes, text, frames that group, the human's freehand ink, images.
- Every message brings an outline of the board under [board], one line per element (id, type, position and size, label, an arrow's from -> to), and a picture of the board attached. Elements the human selected come as selected. Call `board_read` for the latest state.
- Draw with `board_write`: several commands per call; give an add a ref to point at it as "$ref" later in the same call (an arrow's from/to, a frame).
- What you draw appears on the board at once; the human can undo it with Ctrl+Z. Do not redraw what they deleted.

How to draw:
- Concepts, screens and components are labelled shapes (200×100 by default); flows, dependencies and calls are arrows bound to shapes with from/to, a relation's name the arrow's label.
- Decisions are diamonds, starts and ends or people and outside systems ellipses, the rest rectangles.
- Group related elements in a named frame (add the frame first, then give elements frame: "$ref").
- Layout: aligned like a grid, 60 or more apart, flowing left to right or top to bottom. Read the outline's bounds, draw in free space, never over what is there.
- Colour only when it means something: goals #b2f2bb, constraints #ffd8a8, open questions #d0bfff, ideas #ffec99 (backgroundColor).
- Short labels. Longer explanations go in a text element or in chat.
- The human's ink carries intent. Read what a drawing means; when unclear, ask rather than guess.
- The aim is to surface goals, constraints and open questions. When the design is clear enough to hand to a track, say so.
- Keep chat replies short and do not repeat in chat what you drew. Speak English.
"#
    )
}

/// The agent's two board tools, by name and description; the app and the
/// `design_agent` example build them from the same text.
pub const BOARD_READ: &str = "board_read";
pub const BOARD_READ_DESC: &str = "Read the design's Excalidraw board: one line per element with its id, type, position and size, label, frame, and for arrows which elements they join.";
pub const BOARD_WRITE: &str = "board_write";
pub const BOARD_WRITE_DESC: &str = "Draw on the design's Excalidraw board with a list of commands, applied in order; each result says ok with the element id, or the error. \
add {type: rectangle|ellipse|diamond|text|arrow|line|frame, x, y, width?, height?, label?, ref?, frame?, strokeColor?, backgroundColor?, strokeStyle?} — shapes get a centred label; \
text uses text (and fontSize?); arrows and lines join two elements with from and to (ids or \"$ref\"), label? names the relation, or take points [[x,y],…]; a frame's label is its name. \
update {id, label?, text?, x?, y?, width?, height?, strokeColor?, backgroundColor?, strokeStyle?, frame?} — moving a shape carries its label and bound arrows along. \
delete {ids} — a shape's label goes with it; arrows on it stay, unbound. Colours are #rrggbb or transparent.";

pub fn board_write_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "commands": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["add", "update", "delete"] },
                        "type": { "type": "string", "enum": ["rectangle", "ellipse", "diamond", "text", "arrow", "line", "frame"] },
                        "id": { "type": "string" },
                        "ids": { "type": "array", "items": { "type": "string" } },
                        "ref": { "type": "string" },
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "width": { "type": "number" },
                        "height": { "type": "number" },
                        "label": { "type": "string" },
                        "text": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "points": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } },
                        "frame": { "type": "string" },
                        "strokeColor": { "type": "string" },
                        "backgroundColor": { "type": "string" },
                        "strokeStyle": { "type": "string", "enum": ["solid", "dashed", "dotted"] },
                        "fontSize": { "type": "number" }
                    },
                    "required": ["op"]
                }
            }
        },
        "required": ["commands"]
    })
}

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
                    Ok(Value::String(scene(&state, &id)?.outline()))
                }
            },
        ),
        Tool::new(BOARD_WRITE, BOARD_WRITE_DESC, board_write_schema(), move |args| {
            let (app, id) = write.clone();
            async move {
                let ops = parse_commands(&args)?;
                let results = apply(&app, &id, ops)?;
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

    fn get<'a>(sc: &'a Scene, id: &str) -> &'a Value {
        &sc.elements[sc.find(id).unwrap()]
    }

    #[test]
    fn draws_labelled_shapes_joined_by_arrows() {
        let mut sc = Scene::default();
        let (res, changed) = sc.apply(ops(json!([
            { "op": "add", "type": "frame", "x": 0, "y": 0, "width": 800, "height": 300, "label": "Login", "ref": "f" },
            { "op": "add", "type": "rectangle", "x": 40, "y": 80, "label": "Form", "ref": "a", "frame": "$f", "backgroundColor": "#b2f2bb" },
            { "op": "add", "type": "diamond", "x": 400, "y": 80, "label": "Valid?", "ref": "b", "frame": "$f" },
            { "op": "add", "type": "arrow", "from": "$a", "to": "$b", "label": "submit" },
            { "op": "add", "type": "hexagon", "x": 0, "y": 0 }
        ])));
        assert!(res[..4].iter().all(|r| r["ok"] == true), "{res:?}");
        assert_eq!(res[4]["ok"], false);
        let (a, b, arrow) = (res[1]["id"].as_str().unwrap(), res[2]["id"].as_str().unwrap(), res[3]["id"].as_str().unwrap());
        assert_eq!(sc.label_text(a), "Form");
        assert_eq!(get(&sc, a)["frameId"], res[0]["id"]);
        assert_eq!(get(&sc, arrow)["startBinding"]["elementId"], a);
        assert!(get(&sc, a)["boundElements"].as_array().unwrap().iter().any(|x| x["id"] == arrow), "the shape knows its arrow");
        // The arrow leaves the form's right edge and stops short of the diamond.
        let ax = n(get(&sc, arrow), "x");
        assert!(ax > 240.0 && ax < 250.0, "{ax}");
        assert_eq!(changed.len(), sc.elements.len(), "everything new is sent to the window");
        let outline = sc.outline();
        assert!(outline.contains(&format!("{arrow} arrow {a} -> {b} \"submit\"")), "{outline}");
        assert!(outline.contains("frame \"Login\""), "{outline}");
        assert!(outline.contains("fill #b2f2bb"));
    }

    #[test]
    fn moving_a_shape_carries_its_label_and_arrows() {
        let mut sc = Scene::default();
        let (res, _) = sc.apply(ops(json!([
            { "op": "add", "type": "rectangle", "x": 0, "y": 0, "label": "A", "ref": "a" },
            { "op": "add", "type": "rectangle", "x": 400, "y": 0, "label": "B", "ref": "b" },
            { "op": "add", "type": "arrow", "from": "$a", "to": "$b" }
        ])));
        let (b, arrow) = (res[1]["id"].as_str().unwrap().to_string(), res[2]["id"].as_str().unwrap().to_string());
        let label_x = n(&sc.elements[sc.label_of(&b).unwrap()], "x");
        let v = version_of(get(&sc, &arrow));
        let (res, changed) = sc.apply(ops(json!([{ "op": "update", "id": b, "y": 300, "label": "B2" }])));
        assert_eq!(res[0]["ok"], true);
        assert_eq!(sc.label_text(&b), "B2");
        assert_eq!(n(&sc.elements[sc.label_of(&b).unwrap()], "x"), label_x);
        assert!(n(&sc.elements[sc.label_of(&b).unwrap()], "y") > 300.0);
        assert!(version_of(get(&sc, &arrow)) > v, "the arrow was laid again");
        assert!(n(get(&sc, &arrow), "height") > 150.0, "it now runs from A down to B");
        assert!(changed.iter().any(|e| id_of(e) == arrow));
    }

    #[test]
    fn deleting_leaves_tombstones_and_frees_arrows() {
        let mut sc = Scene::default();
        let (res, _) = sc.apply(ops(json!([
            { "op": "add", "type": "ellipse", "x": 0, "y": 0, "label": "A", "ref": "a" },
            { "op": "add", "type": "ellipse", "x": 400, "y": 0, "ref": "b" },
            { "op": "add", "type": "arrow", "from": "$a", "to": "$b" }
        ])));
        let (a, arrow) = (res[0]["id"].as_str().unwrap().to_string(), res[2]["id"].as_str().unwrap().to_string());
        let (res, _) = sc.apply(ops(json!([{ "op": "delete", "ids": [a.clone(), "gone"] }])));
        assert_eq!(res[0]["ok"], true);
        assert!(is_deleted(get(&sc, &a)));
        assert!(sc.label_of(&a).is_none(), "its label went with it");
        assert!(get(&sc, &arrow)["startBinding"].is_null());
        assert!(!sc.outline().contains(&format!("{a} ")));
    }

    #[test]
    fn merge_keeps_the_newer_copy_of_each_element() {
        let mut sc = Scene::default();
        sc.apply(ops(json!([{ "op": "add", "type": "rectangle", "x": 0, "y": 0, "ref": "a" }])));
        let mut mine = sc.elements[0].clone();
        let id = id_of(&mine).to_string();
        // The window moved it (version 2); a stale copy (version 1) is ignored.
        mine["x"] = json!(50);
        mine["version"] = json!(2);
        let stale = sc.elements[0].clone();
        let human = json!({ "id": "h1", "type": "freedraw", "version": 1, "versionNonce": 5, "x": 0, "y": 0, "width": 1, "height": 1 });
        let changed = sc.merge(vec![mine, stale, human], Map::new());
        assert_eq!(changed.len(), 2);
        assert_eq!(n(get(&sc, &id), "x"), 50.0);
        assert!(sc.find("h1").is_some());
    }

    #[test]
    fn old_sketch_boards_become_scenes() {
        let old = json!({
            "version": 7,
            "nodes": [
                { "id": "n1", "kind": "note", "x": 0, "y": 0, "w": 260, "h": 150, "text": "Ship it", "tag": "goal" },
                { "id": "n2", "kind": "note", "x": 400, "y": 0, "w": 260, "h": 150, "text": "" },
                { "id": "n3", "kind": "sketch", "x": 100, "y": 300, "w": 50, "h": 50, "strokes": [{ "points": [[10, 10, 0.5], [40, 30, 0.5]], "color": "#e03131", "size": 4 }] }
            ],
            "edges": [{ "id": "e1", "from": "n1", "to": "n2", "label": "then" }],
            "changes": [], "next": 9
        });
        let sc = parse_body(&old.to_string()).unwrap();
        assert_eq!(sc.version, 8);
        assert_eq!(sc.label_text("n1"), "[goal] Ship it");
        assert_eq!(get(&sc, "n1")["backgroundColor"], "#b2f2bb");
        assert_eq!(get(&sc, "e1")["endBinding"]["elementId"], "n2");
        assert_eq!(sc.label_text("e1"), "then");
        let ink = get(&sc, "n3-0");
        assert_eq!((n(ink, "x"), n(ink, "y")), (110.0, 310.0));
        assert_eq!(ink["points"][1], json!([30.0, 20.0]));
        assert!(parse_body("").unwrap().elements.is_empty());
        assert!(parse_body("{not json").is_err(), "never an empty board over a broken one");
        // A scene reads back as itself.
        let again = parse_body(&serde_json::to_string(&sc).unwrap()).unwrap();
        assert_eq!(again, sc);
    }

    #[test]
    fn briefs_list_what_the_board_says() {
        let mut sc = Scene::default();
        sc.apply(ops(json!([
            { "op": "add", "type": "rectangle", "x": 0, "y": 0, "label": "Client", "ref": "a" },
            { "op": "add", "type": "rectangle", "x": 400, "y": 0, "label": "API", "ref": "b" },
            { "op": "add", "type": "arrow", "from": "$a", "to": "$b", "label": "calls" },
            { "op": "add", "type": "text", "x": 0, "y": 200, "text": "Keep it offline-first" }
        ])));
        let brief = sc.brief("Sync");
        assert!(brief.contains("- Client") && brief.contains("- Keep it offline-first"), "{brief}");
        assert!(brief.contains("- Client → API (calls)"), "{brief}");
    }

    #[test]
    fn refuses_what_it_cannot_draw() {
        let mut sc = Scene::default();
        let (res, changed) = sc.apply(ops(json!([
            { "op": "add", "type": "rectangle", "y": 0 },
            { "op": "add", "type": "arrow", "from": "nope", "to": "nada" },
            { "op": "add", "type": "rectangle", "x": 0, "y": 0, "backgroundColor": "red" },
            { "op": "update", "id": "missing", "label": "x" }
        ])));
        assert!(res.iter().all(|r| r["ok"] == false), "{res:?}");
        assert!(changed.is_empty() && sc.elements.is_empty());
    }
}
