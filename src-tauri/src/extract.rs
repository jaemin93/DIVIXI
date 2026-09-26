//! The words behind a board's references, for its agent: the page a link
//! card points at, and the text in a file card's PDF, Word, PowerPoint,
//! Excel or HTML file. Each goes to `extracted/<card id>.md` in the design's
//! folder (the agent's working folder), headed by where it came from; a
//! failure leaves `extracted/<card id>.err` saying why. The board outline
//! names the file, so the agent reads it with its own tools; a design
//! attached to a track carries these files along.
//!
//! Done in the background whenever the board changes: a card without its
//! text (or whose link now points elsewhere) gets it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tauri::{Emitter, Manager};

use crate::AppHandle;

use crate::design::{self, Kind};
use crate::AppState;

/// Pages larger than this are not read.
const MAX_PAGE: usize = 5 * 1024 * 1024;
/// The most text kept from one reference.
const MAX_TEXT: usize = 200_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Files whose text can be taken out.
const READABLE: &[&str] = &["pdf", "docx", "pptx", "xlsx", "html", "htm"];

/// Cards being read now, as `design/card`.
static IN_FLIGHT: std::sync::Mutex<Option<HashSet<String>>> = std::sync::Mutex::new(None);

fn claim(key: &str) -> bool {
    let mut g = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
    g.get_or_insert_with(HashSet::new).insert(key.to_string())
}

fn release(key: &str) {
    let mut g = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(set) = g.as_mut() {
        set.remove(key);
    }
}

fn folder(workdir: &Path) -> PathBuf {
    workdir.join("extracted")
}

/// Where a card's text is (relative to the design's folder), and where its failure is.
fn names(card: &str) -> (String, String) {
    (format!("extracted/{card}.md"), format!("extracted/{card}.err"))
}

/// The first line of a file, if it exists.
fn first_line(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().next().unwrap_or("").to_string())
}

/// What a card is read from: a web address or a file of the design.
enum Origin {
    Url(String),
    File(PathBuf, String),
}

impl Origin {
    fn label(&self) -> String {
        match self {
            Origin::Url(u) => u.clone(),
            Origin::File(_, name) => name.clone(),
        }
    }
}

/// Where a card's text comes from, when it is a card whose text can be read.
fn origin_of(state: &AppState, design: &str, n: &design::Node) -> Option<Origin> {
    match n.kind {
        Kind::Link if n.url.starts_with("http://") || n.url.starts_with("https://") => Some(Origin::Url(n.url.clone())),
        Kind::File => {
            let ext = Path::new(&n.name).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
            if !READABLE.contains(&ext.as_str()) {
                return None;
            }
            design::file_path(state, design, &n.src).ok().map(|p| Origin::File(p, n.name.clone()))
        }
        _ => None,
    }
}

/// Where a card's text stands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct State {
    pub card: String,
    /// `done`, `failed` or `reading`.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where each readable card's text stands, for the board to show.
pub fn states(state: &AppState, design: &str) -> Vec<State> {
    let Ok(doc) = design::doc(state, design) else { return Vec::new() };
    let Ok(workdir) = crate::artifact::workdir(state, design) else { return Vec::new() };
    doc.nodes
        .iter()
        .filter_map(|n| {
            let origin = origin_of(state, design, n)?;
            let head = format!("Source: {}", origin.label());
            let (md, err) = names(&n.id);
            let st = if first_line(&workdir.join(&md)).as_deref() == Some(head.as_str()) {
                State { card: n.id.clone(), state: "done", error: None }
            } else if let Some(why) = std::fs::read_to_string(workdir.join(&err)).ok().filter(|t| t.lines().next() == Some(head.as_str())) {
                State { card: n.id.clone(), state: "failed", error: Some(why.lines().skip(1).collect::<Vec<_>>().join(" ")) }
            } else {
                State { card: n.id.clone(), state: "reading", error: None }
            };
            Some(st)
        })
        .collect()
}

/// Read a card again (after a failure): its old result goes and it is scheduled.
pub fn retry(app: &AppHandle, design: &str, card: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let workdir = crate::artifact::workdir(&state, design)?;
    let (md, err) = names(card);
    let _ = std::fs::remove_file(workdir.join(md));
    let _ = std::fs::remove_file(workdir.join(err));
    schedule(app, design);
    Ok(())
}

/// Start reading every card of the design that has no text yet.
pub fn schedule(app: &AppHandle, design: &str) {
    let state = app.state::<AppState>();
    let Ok(doc) = design::doc(&state, design) else { return };
    let Ok(workdir) = crate::artifact::workdir(&state, design) else { return };
    for n in &doc.nodes {
        let Some(origin) = origin_of(&state, design, n) else { continue };
        let (md, err) = names(&n.id);
        let head = format!("Source: {}", origin.label());
        // Read already, from this very source (a link may have been changed since).
        if first_line(&workdir.join(&md)).as_deref() == Some(head.as_str()) || first_line(&workdir.join(&err)).as_deref() == Some(head.as_str()) {
            continue;
        }
        let key = format!("{design}/{}", n.id);
        if !claim(&key) {
            continue;
        }
        let (dir, card, app, design) = (workdir.clone(), n.id.clone(), app.clone(), design.to_string());
        tauri::async_runtime::spawn(async move {
            let key = key;
            let got = match &origin {
                Origin::Url(url) => fetch(url).await,
                Origin::File(path, _) => {
                    let path = path.clone();
                    tauri::async_runtime::spawn_blocking(move || file_text(&path)).await.map_err(|e| e.to_string()).and_then(|r| r)
                }
            };
            let (md, err) = names(&card);
            let _ = std::fs::create_dir_all(folder(&dir));
            match got {
                Ok(text) => {
                    let text: String = text.chars().take(MAX_TEXT).collect();
                    let _ = std::fs::remove_file(dir.join(&err));
                    if let Err(e) = std::fs::write(dir.join(&md), format!("{head}\n\n{text}")) {
                        tracing::warn!(%e, "could not keep a reference's text");
                    }
                }
                Err(why) => {
                    tracing::info!(card = %card, %why, "could not read a reference");
                    let _ = std::fs::remove_file(dir.join(&md));
                    let _ = std::fs::write(dir.join(&err), format!("{head}\n{why}"));
                }
            }
            release(&key);
            // The board shows where the card's text stands.
            let _ = app.emit("design_extract", &design);
        });
    }
}

/// A file's text by its kind.
fn file_text(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if ext == "html" || ext == "htm" {
        return Ok(orchestra_knowledge::read::html_text(&String::from_utf8_lossy(&bytes)));
    }
    orchestra_knowledge::read::document_text(&ext, &bytes).map_err(|e| e.to_string())
}

/// A web page's text: HTML without its tags, plain text as it is, a PDF's text layer.
async fn fetch(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("Mozilla/5.0 (Divixi; reading a reference for its user)")
        .build()
        .map_err(|e| e.to_string())?;
    let mut res = client.get(url).send().await.map_err(|e| format!("could not fetch: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("the page answered {}", res.status()));
    }
    let kind = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("could not read the page: {e}"))? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_PAGE {
            return Err("the page is larger than 5 MB".to_string());
        }
    }
    let text = if kind.contains("pdf") || body.starts_with(b"%PDF") {
        tauri::async_runtime::spawn_blocking(move || orchestra_knowledge::documents::pdf_text(&body))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
    } else if kind.contains("html") || kind.is_empty() {
        let html = String::from_utf8_lossy(&body);
        let title = title_of(&html);
        let text = orchestra_knowledge::read::html_text(&html);
        match title {
            Some(t) => format!("# {t}\n\n{text}"),
            None => text,
        }
    } else if kind.starts_with("text/") || kind.contains("json") || kind.contains("xml") {
        String::from_utf8_lossy(&body).into_owned()
    } else {
        return Err(format!("not a page it can read ({kind})"));
    };
    if text.trim().is_empty() {
        return Err("the page has no text (it may be drawn by scripts)".to_string());
    }
    Ok(text)
}

/// A page's <title>, if it has one.
fn title_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    let t = html.get(open_end..end)?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

/// The board outline with each card's text file (or why it has none).
pub fn annotate(state: &AppState, design: &str, mut outline: Value) -> Value {
    let Ok(workdir) = crate::artifact::workdir(state, design) else { return outline };
    if let Some(nodes) = outline.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for n in nodes {
            let Some(id) = n.get("id").and_then(|v| v.as_str()).map(str::to_string) else { continue };
            let (md, err) = names(&id);
            if workdir.join(&md).is_file() {
                n["text_file"] = Value::String(md);
            } else if let Ok(why) = std::fs::read_to_string(workdir.join(&err)) {
                n["text_error"] = Value::String(why.lines().skip(1).collect::<Vec<_>>().join(" "));
            }
        }
    }
    outline
}

/// A card's text file on disk, if it has one.
pub fn text_file(workdir: &Path, card: &str) -> Option<PathBuf> {
    let p = workdir.join(names(card).0);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the network: \`cargo test -p orchestra-app extract -- --ignored\`.
    #[test]
    #[ignore]
    fn reads_a_real_page() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let text = rt.block_on(fetch("https://example.com/")).unwrap();
        assert!(text.starts_with("# Example Domain"), "{text}");
        assert!(text.contains("documentation"), "{text}");
    }

    #[test]
    fn a_page_title() {
        assert_eq!(title_of("<html><head><TITLE>\n  Rust  Book </TITLE></head>").as_deref(), Some("Rust Book"));
        assert_eq!(title_of("<p>none</p>"), None);
    }
}
