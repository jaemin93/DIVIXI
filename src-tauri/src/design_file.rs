//! A design as one file, to hand to someone else's DIVIXI.
//!
//! A `.divixi-design` file is JSON: the format's name and version, the
//! design's name, colour and tags, its board (cards, arrows, frames, ink, with
//! their ids, so the ids the agent named still point at the same cards), and
//! the files its cards show, each as base64 with its size and SHA-256.
//!
//! It is written to be sent to another person, so it carries nothing of this
//! machine or this person beyond the design itself: no conversation with the
//! agent, no agent or its settings, no suggestions waiting to be kept (they
//! name the agent's runs), no text read out of links and documents (made
//! again where it lands), no ids or times of this install, no paths.
//!
//! A file that arrives is someone else's, so reading one trusts nothing: its
//! size is capped before it is read, the format and version are checked,
//! every board item passes the board's own checks, file names are plain names
//! made here (nothing in the file names a place on disk, so there is no path
//! to escape through), each file's bytes must match its size and hash, and
//! only file types a board shows are taken. What is imported is always a new
//! design; nothing that exists is touched.

use std::collections::HashSet;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::design::{self, Doc, Edge, Kind, Node};

/// What the file says it is.
pub const FORMAT: &str = "divixi-design";
/// This build writes version 1 and reads it; a later version is refused by
/// name rather than half-read.
pub const VERSION: u32 = 1;
/// The file's extension, without the dot.
pub const EXTENSION: &str = "divixi-design";
/// The largest file read at all: the files below, as base64, and the board.
pub const MAX_PACKAGE: u64 = 140 * 1024 * 1024;
/// The files a design may carry, together.
pub const MAX_ASSETS: u64 = 100 * 1024 * 1024;
/// How many files a design may carry.
pub const MAX_ASSET_COUNT: usize = 500;
/// The most arrows a board may bring.
const MAX_EDGES: usize = 4 * design::MAX_NODES;

/// The file as written.
#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    pub format: String,
    pub version: u32,
    pub design: Meta,
    pub board: Board,
    #[serde(default)]
    pub files: Vec<Asset>,
}

/// What the design is called and how it looks in the list.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The board, without what belongs to this install (its version counter,
/// the agent's waiting suggestions).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Board {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub next: u64,
}

/// One file a card shows, by the plain name its card keeps (`files/<name>`).
#[derive(Debug, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    /// The bytes, base64 (standard alphabet, padded).
    pub data: String,
}

/// What reading a file gives: a design to make, and its files' bytes.
#[derive(Debug)]
pub struct Unpacked {
    pub meta: Meta,
    pub doc: Doc,
    pub files: Vec<(String, Vec<u8>)>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The plain name a card's `src` keeps, when it is one of the design's files.
fn file_name(src: &str) -> Option<String> {
    design::check_src(src).ok().map(|s| s["files/".len()..].to_string())
}

/// File types a board shows, by extension; anything else is not imported.
fn known(name: &str) -> bool {
    design::mime_of(name) != "application/octet-stream"
}

/// Write a design out: `read` gives a file's bytes by its plain name (none
/// when it is gone, and the card goes without it).
pub fn pack(meta: &Meta, doc: &Doc, read: impl Fn(&str) -> Option<Vec<u8>>) -> Result<String, String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for n in doc.nodes.iter().filter(|n| n.kind == Kind::File) {
        let Some(name) = file_name(&n.src) else { continue };
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(bytes) = read(&name) else { continue };
        total += bytes.len() as u64;
        if total > MAX_ASSETS || files.len() >= MAX_ASSET_COUNT {
            return Err(format!(
                "this design's files come to more than {} MB (or {MAX_ASSET_COUNT} files), too much for one file",
                MAX_ASSETS / (1024 * 1024)
            ));
        }
        files.push(Asset {
            name,
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(&bytes)),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
    }
    let package = Package {
        format: FORMAT.to_string(),
        version: VERSION,
        design: meta.clone(),
        board: Board { nodes: doc.nodes.clone(), edges: doc.edges.clone(), next: doc.next },
        files,
    };
    serde_json::to_string(&package).map_err(|e| e.to_string())
}

/// Only the head, so a later version is named before the rest is parsed.
#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
}

/// Read a file that arrived. Errors say what is wrong in a sentence.
pub fn unpack(text: &str) -> Result<Unpacked, String> {
    if text.len() as u64 > MAX_PACKAGE {
        return Err(format!("the file is larger than {} MB", MAX_PACKAGE / (1024 * 1024)));
    }
    let head: Head = serde_json::from_str(text).map_err(|_| "this is not a DIVIXI design file (it could not be read)".to_string())?;
    if head.format != FORMAT {
        return Err("this is not a DIVIXI design file".to_string());
    }
    if head.version > VERSION {
        return Err(format!("this design file is version {}, from a newer DIVIXI; update to open it", head.version));
    }
    if head.version < 1 {
        return Err("this design file has no version".to_string());
    }
    let package: Package = serde_json::from_str(text).map_err(|e| format!("the design file is damaged: {e}"))?;

    // The files first: the cards are checked against what came.
    if package.files.len() > MAX_ASSET_COUNT {
        return Err(format!("the file carries more than {MAX_ASSET_COUNT} files"));
    }
    let mut files = Vec::new();
    let mut names = HashSet::new();
    let mut total = 0u64;
    for a in &package.files {
        let plain = design::plain_name(&a.name);
        if plain != a.name || a.name.len() > 120 || !names.insert(a.name.clone()) {
            return Err(format!("the design file names a file oddly ({:?})", a.name.chars().take(60).collect::<String>()));
        }
        if !known(&a.name) {
            return Err(format!("the design file carries a file of a type a board does not show ({})", a.name));
        }
        if a.size > design::MAX_FILE || a.data.len() as u64 > design::MAX_FILE / 3 * 4 + 4 {
            return Err(format!("a file in the design ({}) is larger than 50 MB", a.name));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(a.data.as_bytes())
            .map_err(|_| format!("a file in the design ({}) is damaged", a.name))?;
        if bytes.len() as u64 != a.size || hex(&Sha256::digest(&bytes)) != a.sha256.to_ascii_lowercase() {
            return Err(format!("a file in the design ({}) is damaged (its size or checksum does not match)", a.name));
        }
        total += a.size;
        if total > MAX_ASSETS {
            return Err(format!("the design's files come to more than {} MB", MAX_ASSETS / (1024 * 1024)));
        }
        files.push((a.name.clone(), bytes));
    }

    let doc = board_of(package.board, &names)?;
    let meta = Meta {
        title: tidy_title(&package.design.title),
        color: package.design.color.trim().to_string(),
        tags: package.design.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
    };
    // Only files a card shows are kept.
    let used: HashSet<String> = doc.nodes.iter().filter_map(|n| file_name(&n.src)).collect();
    files.retain(|(name, _)| used.contains(name));
    Ok(Unpacked { meta, doc, files })
}

/// A title as the list shows it: one line, no control characters, at most 80.
fn tidy_title(title: &str) -> String {
    let one: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(80).collect()
}

/// An item's id as the board makes them: short, letters, digits, `-` and `_`.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn finite_box(n: &Node) -> bool {
    let ok = |v: f64| v.is_finite() && v.abs() <= 1.0e7;
    ok(n.x) && ok(n.y) && ok(n.w) && ok(n.h) && n.w >= 0.0 && n.h >= 0.0
}

/// The board as it may come in: every item through the board's own checks,
/// a card's file only when the file came with it, an arrow only between
/// cards that came.
fn board_of(board: Board, files: &HashSet<String>) -> Result<Doc, String> {
    if board.nodes.len() > design::MAX_NODES {
        return Err(format!("the design has more than {} items", design::MAX_NODES));
    }
    if board.edges.len() > MAX_EDGES {
        return Err(format!("the design has more than {MAX_EDGES} arrows"));
    }
    let bad = |what: &str| Err(format!("the design file is damaged ({what})"));
    let mut ids = HashSet::new();
    let mut nodes = Vec::with_capacity(board.nodes.len());
    let mut highest = 0u64;
    for mut n in board.nodes {
        if !plain_id(&n.id) || !ids.insert(n.id.clone()) {
            return bad("an item's id");
        }
        if !finite_box(&n) {
            return bad("an item's place or size");
        }
        if !design::TAGS.contains(&n.tag.as_str()) {
            n.tag.clear();
        }
        if n.by != "human" && n.by != "agent" {
            n.by.clear();
        }
        design::check_text(&n.text, design::MAX_TEXT, "a note")?;
        design::check_text(&n.answer, design::MAX_TEXT, "an answer")?;
        design::check_text(&n.name, design::MAX_LABEL, "a name")?;
        design::check_strokes(&n.strokes)?;
        // A card's file only if it came, and typed here, not by the file.
        match file_name(&n.src) {
            Some(name) if files.contains(&name) => n.mime = design::mime_of(&name).to_string(),
            _ => {
                n.src.clear();
                n.mime.clear();
            }
        }
        if !n.url.is_empty() {
            n.url = design::check_url(&n.url).unwrap_or_default();
        }
        if let Ok(num) = n.id.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>() {
            highest = highest.max(num);
        }
        nodes.push(n);
    }
    let mut edge_ids = HashSet::new();
    let mut edges = Vec::new();
    for mut e in board.edges {
        if !plain_id(&e.id) || !edge_ids.insert(e.id.clone()) || ids.contains(&e.id) {
            return bad("an arrow's id");
        }
        // An arrow whose ends did not come goes; the rest of the board stands.
        if !ids.contains(&e.from) || !ids.contains(&e.to) {
            continue;
        }
        design::check_text(&e.label, design::MAX_LABEL, "an arrow's label")?;
        if e.by != "human" && e.by != "agent" {
            e.by.clear();
        }
        if let Ok(num) = e.id.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>() {
            highest = highest.max(num);
        }
        edges.push(e);
    }
    let mut doc = Doc::default();
    doc.nodes = nodes;
    doc.edges = edges;
    doc.next = board.next.max(highest + 1);
    Ok(doc)
}

/// A name no design here has: the title, else with `suffix` ("(가져옴)"),
/// else with a number inside it ("(가져옴 2)").
pub fn unique_title(title: &str, taken: &[String], suffix: &str) -> String {
    let title = if title.trim().is_empty() { "Untitled".to_string() } else { title.trim().to_string() };
    let used: HashSet<&str> = taken.iter().map(String::as_str).collect();
    if !used.contains(title.as_str()) {
        return title;
    }
    let suffix = suffix.trim();
    let first = format!("{title} {suffix}");
    if !used.contains(first.as_str()) {
        return first;
    }
    // "(가져옴)" → "(가져옴 2)"; a suffix without brackets → "가져옴 2".
    let (open, word, close) = match (suffix.strip_prefix('('), suffix.strip_suffix(')')) {
        (Some(_), Some(_)) => ("(", &suffix[1..suffix.len() - 1], ")"),
        _ => ("", suffix, ""),
    };
    (2..)
        .map(|n| format!("{title} {open}{word} {n}{close}"))
        .find(|t| !used.contains(t.as_str()))
        .unwrap_or(first)
}

/// A file name to save a design under: its title, made plain.
pub fn file_name_for(title: &str) -> String {
    let base: String = title.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '(' | ')') { c } else { '_' }).collect();
    let base = base.trim().trim_matches('_').chars().take(60).collect::<String>();
    format!("{}.{EXTENSION}", if base.is_empty() { "design" } else { &base })
}

/// The files a design's cards show, as stored: by plain name, from its folder.
pub fn read_from(dir: std::path::PathBuf) -> impl Fn(&str) -> Option<Vec<u8>> {
    move |name: &str| {
        let path = dir.join("files").join(name);
        let meta = std::fs::metadata(&path).ok()?;
        (meta.is_file() && meta.len() <= design::MAX_FILE).then(|| std::fs::read(&path).ok()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn by_name(files: &[(String, Vec<u8>)]) -> HashMap<String, Vec<u8>> {
        files.iter().cloned().collect()
    }

    fn node(id: &str, kind: Kind, text: &str) -> Node {
        Node { id: id.into(), kind, x: 10.0, y: 20.0, w: 260.0, h: 150.0, text: text.into(), by: "human".into(), ..Default::default() }
    }

    fn sample() -> (Meta, Doc, HashMap<String, Vec<u8>>) {
        let mut pic = node("n3", Kind::File, "");
        pic.src = "files/ab12cd34-pic.png".into();
        pic.name = "그림.png".into();
        pic.mime = "image/png".into();
        let mut link = node("n4", Kind::Link, "");
        link.url = "https://example.com/page".into();
        let mut doc = Doc::default();
        doc.nodes = vec![node("n1", Kind::Note, "양쪽 비교 — 곳곳에 🧑‍💻 e\u{301}"), node("n2", Kind::Frame, "첫 화면"), pic, link];
        doc.edges = vec![Edge { id: "e5".into(), from: "n1".into(), to: "n3".into(), label: "본다".into(), by: "agent".into() }];
        doc.next = 6;
        let meta = Meta { title: "가게 앱 첫 화면".into(), color: "#c0392b".into(), tags: vec!["mobile".into()] };
        let files = HashMap::from([("ab12cd34-pic.png".to_string(), vec![0x89, b'P', b'N', b'G', 1, 2, 3, 250])]);
        (meta, doc, files)
    }

    fn packed() -> String {
        let (meta, doc, files) = sample();
        pack(&meta, &doc, |n| files.get(n).cloned()).unwrap()
    }

    fn with(text: &str, f: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn a_design_goes_out_and_comes_back_the_same() {
        let (meta, doc, files) = sample();
        let back = unpack(&packed()).unwrap();
        assert_eq!(back.meta, meta);
        assert_eq!(back.doc.nodes, doc.nodes, "cards, ids, Korean, emoji and a combining mark unchanged");
        assert_eq!(back.doc.edges, doc.edges);
        assert_eq!(back.doc.next, 6);
        assert_eq!(by_name(&back.files), files, "the picture's bytes unchanged");
    }

    #[test]
    fn what_belongs_to_this_install_stays_here() {
        let (meta, mut doc, files) = sample();
        doc.version = 42;
        doc.changes = vec![design::Change { id: 1, target: "n1".into(), run: Some("run-123".into()), before: None, after: None }];
        let text = pack(&meta, &doc, |n| files.get(n).cloned()).unwrap();
        assert!(!text.contains("run-123") && !text.contains("\"changes\"") && !text.contains("\"version\":42"));
        // Nothing of a path on this machine: only plain file names.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        for f in v["files"].as_array().unwrap() {
            assert!(!f["name"].as_str().unwrap().contains(['/', '\\', ':']));
        }
        assert_eq!(v["format"], FORMAT);
        assert_eq!(v["version"], VERSION);
        let back = unpack(&text).unwrap();
        assert!(back.doc.changes.is_empty() && back.doc.version == 0);
    }

    #[test]
    fn a_card_whose_file_is_gone_goes_without_it() {
        let (meta, doc, _) = sample();
        let back = unpack(&pack(&meta, &doc, |_| None).unwrap()).unwrap();
        let pic = back.doc.nodes.iter().find(|n| n.id == "n3").unwrap();
        assert!(pic.src.is_empty() && back.files.is_empty());
    }

    #[test]
    fn what_is_not_a_design_file_is_refused_by_name() {
        assert!(unpack("not json").unwrap_err().contains("not a DIVIXI design file"));
        assert!(unpack("{\"format\":\"something-else\",\"version\":1}").unwrap_err().contains("not a DIVIXI design file"));
        let newer = with(&packed(), |v| v["version"] = (VERSION + 1).into());
        assert!(unpack(&newer).unwrap_err().contains("newer DIVIXI"));
        let none = with(&packed(), |v| v["version"] = 0.into());
        assert!(unpack(&none).unwrap_err().contains("no version"));
        let cut = &packed()[..200];
        assert!(unpack(cut).is_err(), "a cut file");
    }

    #[test]
    fn a_damaged_or_swapped_file_is_refused() {
        let flipped = with(&packed(), |v| v["files"][0]["data"] = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3, 4, 5, 6, 7, 8]).into());
        assert!(unpack(&flipped).unwrap_err().contains("checksum"));
        let garbage = with(&packed(), |v| v["files"][0]["data"] = "***".into());
        assert!(unpack(&garbage).unwrap_err().contains("damaged"));
        let big = with(&packed(), |v| v["files"][0]["size"] = (design::MAX_FILE + 1).into());
        assert!(unpack(&big).unwrap_err().contains("50 MB"));
    }

    #[test]
    fn a_file_name_cannot_reach_outside_the_design() {
        for evil in ["../../evil.png", "..\\evil.png", "/etc/x.png", "C:\\x.png", ".hidden.png", "a/b.png", "x.png\u{0}"] {
            let text = with(&packed(), |v| v["files"][0]["name"] = evil.into());
            assert!(unpack(&text).unwrap_err().contains("oddly"), "{evil:?} refused");
        }
        // A card pointing outside its folder loses the pointer, not the board.
        let text = with(&packed(), |v| v["board"]["nodes"][2]["src"] = "files/../../secret.png".into());
        let back = unpack(&text).unwrap();
        assert!(back.doc.nodes[2].src.is_empty() && back.files.is_empty());
    }

    #[test]
    fn only_file_types_a_board_shows_come_in() {
        let text = with(&packed(), |v| {
            v["files"][0]["name"] = "ab12cd34-run.exe".into();
            v["board"]["nodes"][2]["src"] = "files/ab12cd34-run.exe".into();
        });
        assert!(unpack(&text).unwrap_err().contains("type a board does not show"));
    }

    #[test]
    fn a_too_large_file_is_refused_before_it_is_read() {
        let huge = " ".repeat(MAX_PACKAGE as usize + 1);
        assert!(unpack(&huge).unwrap_err().contains("larger than"));
    }

    #[test]
    fn the_board_passes_its_own_checks() {
        let long = with(&packed(), |v| v["board"]["nodes"][0]["text"] = "가".repeat(design::MAX_TEXT + 1).into());
        assert!(unpack(&long).is_err());
        let dup = with(&packed(), |v| v["board"]["nodes"][1]["id"] = "n1".into());
        assert!(unpack(&dup).unwrap_err().contains("id"));
        let odd = with(&packed(), |v| v["board"]["nodes"][1]["id"] = "<script>".into());
        assert!(unpack(&odd).unwrap_err().contains("id"));
        let far = with(&packed(), |v| v["board"]["nodes"][0]["x"] = 1.0e300.into());
        assert!(unpack(&far).unwrap_err().contains("place"));
        let js = with(&packed(), |v| v["board"]["nodes"][3]["url"] = "javascript:alert(1)".into());
        assert!(unpack(&js).unwrap().doc.nodes[3].url.is_empty(), "only http(s) links");
        let mime = with(&packed(), |v| v["board"]["nodes"][2]["mime"] = "text/html".into());
        assert_eq!(unpack(&mime).unwrap().doc.nodes[2].mime, "image/png", "typed by the name here, not by the file");
        let dangling = with(&packed(), |v| v["board"]["edges"][0]["to"] = "n99".into());
        assert!(unpack(&dangling).unwrap().doc.edges.is_empty(), "an arrow to nothing goes");
        let low = with(&packed(), |v| v["board"]["next"] = 1.into());
        assert_eq!(unpack(&low).unwrap().doc.next, 6, "new items never take an id that came");
    }

    #[test]
    fn a_title_is_one_plain_line() {
        let text = with(&packed(), |v| v["design"]["title"] = "  둘째\n줄\t<b>굵게</b>  ".into());
        assert_eq!(unpack(&text).unwrap().meta.title, "둘째 줄 <b>굵게</b>", "text, never markup: the list draws it as text");
    }

    #[test]
    fn an_imported_name_never_takes_one_that_is_there() {
        let taken = vec!["가게 앱".to_string(), "가게 앱 (가져옴)".to_string()];
        assert_eq!(unique_title("새 앱", &taken, "(가져옴)"), "새 앱");
        assert_eq!(unique_title("가게 앱", &taken[..1], "(가져옴)"), "가게 앱 (가져옴)");
        assert_eq!(unique_title("가게 앱", &taken, "(가져옴)"), "가게 앱 (가져옴 2)");
        assert_eq!(unique_title("App", &["App".to_string(), "App (imported)".to_string()], "(imported)"), "App (imported 2)");
        assert_eq!(unique_title("  ", &[], "(가져옴)"), "Untitled");
    }

    #[test]
    fn a_saved_file_is_named_after_the_design() {
        assert_eq!(file_name_for("가게 앱: 첫 화면"), "가게 앱_ 첫 화면.divixi-design");
        assert_eq!(file_name_for("../.."), "design.divixi-design");
        assert_eq!(file_name_for("가게 앱 (가져옴)"), "가게 앱 (가져옴).divixi-design");
    }
}
