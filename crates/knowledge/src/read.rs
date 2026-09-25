//! Which files can be knowledge, and reading them as text.

use std::path::Path;

use crate::chunk::Shape;

/// Text files larger than this are refused.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Documents (PDF, Word) are larger for the text they hold.
pub const MAX_DOCUMENT_BYTES: u64 = 50 * 1024 * 1024;

/// Documents whose text is extracted rather than read.
const DOCUMENTS: &[&str] = &["pdf", "docx"];

const MARKDOWN: &[&str] = &["md", "markdown", "mdx"];
const CODE: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "svelte", "vue", "go", "java", "kt", "kts", "swift", "c", "h", "cpp", "cc",
    "cxx", "hpp", "hh", "cs", "rb", "php", "lua", "r", "scala", "dart", "ex", "exs", "erl", "hs", "ml", "clj", "sh", "bash", "zsh",
    "ps1", "bat", "sql", "proto", "graphql", "tf", "gradle", "css", "scss",
];
const TEXT: &[&str] = &[
    "txt", "text", "org", "rst", "adoc", "csv", "tsv", "log", "json", "jsonc", "yaml", "yml", "toml", "ini", "cfg", "conf", "xml",
    "html", "htm",
];

/// The extension, lowercased, without the dot.
pub fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// How a supported file is chunked; `None` when it cannot be knowledge.
pub fn shape_of(path: &Path) -> Option<Shape> {
    let ext = extension(path);
    // A Word document's headings come out as markdown headings.
    if MARKDOWN.contains(&ext.as_str()) || ext == "docx" {
        Some(Shape::Markdown)
    } else if ext == "pdf" {
        Some(Shape::Text)
    } else if CODE.contains(&ext.as_str()) {
        Some(Shape::Code)
    } else if TEXT.contains(&ext.as_str()) {
        Some(Shape::Text)
    } else {
        None
    }
}

/// Every supported extension, for the UI.
pub fn supported_extensions() -> Vec<&'static str> {
    MARKDOWN.iter().chain(CODE).chain(TEXT).chain(DOCUMENTS).copied().collect()
}

/// Files that hold secrets are never read into the library.
pub fn is_sensitive(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    let ext = extension(path);
    if name.starts_with(".env") || name.starts_with("id_rsa") || name.starts_with("id_ed25519") || name.starts_with("credentials") {
        return true;
    }
    if matches!(name.as_str(), ".npmrc" | ".netrc" | ".pypirc" | ".git-credentials") {
        return true;
    }
    if matches!(ext.as_str(), "pem" | "key" | "p12" | "pfx" | "keystore" | "jks") {
        return true;
    }
    path.components().any(|c| {
        let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(c.as_str(), ".ssh" | ".aws" | ".gnupg" | ".kube")
    })
}

/// Why a file cannot be added, or `None` when it can.
pub fn refusal(path: &Path) -> Option<String> {
    if is_sensitive(path) {
        return Some("this file may hold secrets and is never added".to_string());
    }
    if shape_of(path).is_none() {
        return Some(format!("unsupported file type .{}", extension(path)));
    }
    None
}

/// A file read for indexing.
#[derive(Debug, Clone)]
pub struct FileText {
    pub text: String,
    pub size: u64,
    pub mtime_ms: i64,
    /// sha256 of the text, hex.
    pub hash: String,
}

/// Read a supported file as text. HTML loses its tags.
pub fn read_file(path: &Path) -> anyhow::Result<FileText> {
    if let Some(why) = refusal(path) {
        anyhow::bail!(why);
    }
    let meta = std::fs::metadata(path)?;
    let ext = extension(path);
    if DOCUMENTS.contains(&ext.as_str()) {
        if meta.len() > MAX_DOCUMENT_BYTES {
            anyhow::bail!("document is larger than {} MB", MAX_DOCUMENT_BYTES / 1024 / 1024);
        }
        let bytes = std::fs::read(path)?;
        let text = if ext == "pdf" { crate::documents::pdf_text(&bytes)? } else { crate::documents::docx_text(&bytes)? };
        return Ok(FileText { hash: hash(&text), size: meta.len(), mtime_ms: mtime_ms(&meta), text });
    }
    if meta.len() > MAX_FILE_BYTES {
        anyhow::bail!("file is larger than {} MB", MAX_FILE_BYTES / 1024 / 1024);
    }
    let bytes = std::fs::read(path)?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        anyhow::bail!("file looks binary");
    }
    let mut text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    if matches!(extension(path).as_str(), "html" | "htm") {
        text = strip_tags(&text);
    }
    Ok(FileText { hash: hash(&text), size: meta.len(), mtime_ms: mtime_ms(&meta), text })
}

/// Modification time in ms since the epoch, 0 when unknown.
pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// sha256 of a text, hex.
pub fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Text of an HTML page: tags, scripts and styles dropped, blank runs folded.
fn strip_tags(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let skip_to = ["script", "style"].iter().find_map(|tag| {
                lower[i + 1..].starts_with(tag).then(|| lower[i..].find(&format!("</{tag}")).map(|e| i + e))?
            });
            let from = skip_to.unwrap_or(i);
            match lower[from..].find('>') {
                Some(end) => {
                    i = from + end + 1;
                    out.push(' ');
                }
                None => break,
            }
        } else {
            let next = html[i..].find('<').map(|n| i + n).unwrap_or(html.len());
            out.push_str(&html[i..next]);
            i = next;
        }
    }
    let decoded = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
    let mut folded = String::new();
    let mut blank = 0;
    for line in decoded.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank == 1 && !folded.is_empty() {
                folded.push('\n');
            }
        } else {
            blank = 0;
            folded.push_str(&l);
            folded.push('\n');
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn shapes_and_refusals() {
        assert_eq!(shape_of(Path::new("a/README.md")), Some(Shape::Markdown));
        assert_eq!(shape_of(Path::new("src/main.RS")), Some(Shape::Code));
        assert_eq!(shape_of(Path::new("notes.txt")), Some(Shape::Text));
        assert!(refusal(Path::new("x.png")).is_some());
        assert!(refusal(Path::new(".env.local")).is_some());
        assert!(refusal(&PathBuf::from("home").join(".ssh").join("config.txt")).is_some());
        assert!(refusal(Path::new("docs/guide.md")).is_none());
        assert_eq!(shape_of(Path::new("spec.PDF")), Some(Shape::Text));
        assert_eq!(shape_of(Path::new("plan.docx")), Some(Shape::Markdown));
    }

    #[test]
    fn html_loses_tags() {
        let t = strip_tags("<html><style>p{}</style><p>Hello &amp; <b>bye</b></p><script>x()</script>\n\n\n<p>two</p></html>");
        assert!(t.contains("Hello & bye"), "{t}");
        assert!(!t.contains("x()") && !t.contains("p{}"), "{t}");
        assert!(t.contains("two"));
    }
}
