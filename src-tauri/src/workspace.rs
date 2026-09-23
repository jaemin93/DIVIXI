//! A track's working folder, as the side panel shows it: the file tree,
//! file contents for preview, and what git says has changed.
//!
//! Everything here is read-only and fenced to the track's folder: a path
//! is resolved against the folder and refused if it escapes. Git is the
//! user's own `git` on PATH, so what the panel shows is what their
//! terminal would say.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Most entries a tree listing returns; deeper than this a repo is not
/// something to browse in a side panel.
const MAX_ENTRIES: usize = 6000;

/// Largest file shown as text.
const MAX_TEXT: u64 = 2 * 1024 * 1024;

/// Largest image shown inline.
const MAX_IMAGE: u64 = 6 * 1024 * 1024;

/// One file or folder, relative to the track folder, `/`-separated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// What a file holds, in a form the panel can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub name: String,
    pub size: u64,
    /// `markdown`, `text`, `image`, `binary` or `large`.
    pub kind: String,
    /// Extension without the dot, lowercase.
    pub ext: String,
    pub text: Option<String>,
    /// `data:` URL for images.
    pub data_url: Option<String>,
}

/// One changed path as `git status` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    /// The two porcelain columns, e.g. ` M`, `A `, `??`, `R `.
    pub code: String,
    pub untracked: bool,
    /// For renames, where it came from.
    pub from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    pub repo: bool,
    pub branch: String,
    pub changes: Vec<Change>,
}

fn rel_string(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The folder's files and directories, honouring `.gitignore`, `.git`
/// itself left out, folders first then names, case-insensitive.
pub fn tree(root: &Path) -> Result<Vec<Entry>, String> {
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    let mut out = Vec::new();
    for entry in walker.flatten() {
        let p = entry.path();
        if p == root {
            continue;
        }
        let dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if dir { 0 } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
        out.push(Entry {
            path: rel_string(root, p),
            name: entry.file_name().to_string_lossy().into_owned(),
            dir,
            size,
        });
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out.sort_by(|a, b| {
        let da: Vec<&str> = a.path.split('/').collect();
        let db: Vec<&str> = b.path.split('/').collect();
        // Compare component by component: folders before files at each level.
        for (i, (x, y)) in da.iter().zip(db.iter()).enumerate() {
            let last_a = i == da.len() - 1;
            let last_b = i == db.len() - 1;
            let a_dir = !last_a || a.dir;
            let b_dir = !last_b || b.dir;
            if a_dir != b_dir {
                return b_dir.cmp(&a_dir);
            }
            let c = x.to_lowercase().cmp(&y.to_lowercase());
            if c != std::cmp::Ordering::Equal {
                return c;
            }
        }
        da.len().cmp(&db.len())
    });
    Ok(out)
}

/// `rel` under `root`, or an error if it points outside.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.contains("..") || rel.starts_with('/') || rel.starts_with('\\') || rel.contains(':') {
        return Err(format!("not a workspace path: {rel}"));
    }
    let root_c = root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?;
    let full = root.join(rel);
    let full_c = full.canonicalize().map_err(|e| format!("{}: {e}", full.display()))?;
    if !full_c.starts_with(&root_c) {
        return Err(format!("outside the workspace: {rel}"));
    }
    Ok(full_c)
}

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico"];

fn mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Read a file for the panel: text (with markdown flagged), an inline
/// image, or a note that it is binary or too large.
pub fn read(root: &Path, rel: &str) -> Result<FileContent, String> {
    let full = resolve(root, rel)?;
    let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Err(format!("{rel} is a directory"));
    }
    let name = full.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = full
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let size = meta.len();
    let mut out = FileContent {
        path: rel.to_string(),
        name,
        size,
        kind: "text".into(),
        ext: ext.clone(),
        text: None,
        data_url: None,
    };
    if IMAGE_EXT.contains(&ext.as_str()) {
        if size > MAX_IMAGE {
            out.kind = "large".into();
            return Ok(out);
        }
        let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
        out.kind = "image".into();
        out.data_url = Some(format!("data:{};base64,{}", mime(&ext), base64(&bytes)));
        return Ok(out);
    }
    if size > MAX_TEXT {
        out.kind = "large".into();
        return Ok(out);
    }
    let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
    // A NUL in the first bytes means "not text" for every format we show.
    if bytes.iter().take(8192).any(|b| *b == 0) {
        out.kind = "binary".into();
        return Ok(out);
    }
    out.text = Some(String::from_utf8_lossy(&bytes).into_owned());
    if matches!(ext.as_str(), "md" | "markdown" | "mdx") {
        out.kind = "markdown".into();
    }
    Ok(out)
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|e| format!("git: {e}"))
}

/// Branch and changed paths, as `git status --porcelain` sees them.
pub fn status(root: &Path) -> Result<GitStatus, String> {
    let inside = git(root, &["rev-parse", "--is-inside-work-tree"])?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Ok(GitStatus::default());
    }
    let out = git(root, &["status", "--porcelain=v1", "-b", "--untracked-files=all"])?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut status = GitStatus {
        repo: true,
        ..Default::default()
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            status.branch = rest.split("...").next().unwrap_or(rest).trim().to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let code = line[..2].to_string();
        let path_part = &line[3..];
        let (from, path) = match path_part.split_once(" -> ") {
            Some((a, b)) => (Some(a.to_string()), b.to_string()),
            None => (None, path_part.to_string()),
        };
        let unquote = |s: String| s.trim_matches('"').to_string();
        status.changes.push(Change {
            untracked: code == "??",
            path: unquote(path),
            code,
            from: from.map(unquote),
        });
    }
    Ok(status)
}

/// The unified diff of one path against HEAD; an untracked file diffs
/// against nothing, so it shows as all added.
pub fn diff(root: &Path, rel: &str, untracked: bool) -> Result<String, String> {
    resolve(root, rel).or_else(|e| {
        // A deleted file no longer exists to canonicalize; the path is
        // still fenced by the `..` check.
        if rel.contains("..") { Err(e) } else { Ok(root.join(rel)) }
    })?;
    let out = if untracked {
        git(root, &["diff", "--no-index", "--", "/dev/null", rel])?
    } else {
        git(root, &["diff", "HEAD", "--", rel])?
    };
    // `git diff` exits 1 when there are differences; only a real failure
    // has nothing on stdout and something on stderr.
    if out.stdout.is_empty() && !out.stderr.is_empty() && !untracked {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Show a file in the system file manager, selected.
pub fn reveal(root: &Path, rel: &str) -> Result<(), String> {
    let full = resolve(root, rel)?;
    #[cfg(target_os = "windows")]
    {
        // `\\?\`-prefixed paths confuse explorer; hand it the plain form.
        let plain = full.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        Command::new("explorer")
            .arg(format!("/select,{plain}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(&full).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = full.parent().unwrap_or(&full);
        Command::new("xdg-open").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orchestra-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tree_sorts_folders_first_and_skips_git_and_ignored() {
        let root = temp("tree");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref").unwrap();
        std::fs::write(root.join("target/out.o"), "x").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("b.md"), "# b").unwrap();
        std::fs::write(root.join("A.txt"), "a").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main(){}").unwrap();
        let paths: Vec<String> = tree(&root).unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, vec!["src", "src/main.rs", ".gitignore", "A.txt", "b.md"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_classifies_markdown_text_image_and_binary_and_fences_paths() {
        let root = temp("read");
        std::fs::write(root.join("NOTE.md"), "# hi").unwrap();
        std::fs::write(root.join("a.rs"), "fn a(){}").unwrap();
        std::fs::write(root.join("bin.dat"), [1u8, 0, 2]).unwrap();
        std::fs::write(root.join("p.png"), [137u8, 80, 78, 71]).unwrap();
        assert_eq!(read(&root, "NOTE.md").unwrap().kind, "markdown");
        let rs = read(&root, "a.rs").unwrap();
        assert_eq!((rs.kind.as_str(), rs.ext.as_str(), rs.text.as_deref()), ("text", "rs", Some("fn a(){}")));
        assert_eq!(read(&root, "bin.dat").unwrap().kind, "binary");
        let img = read(&root, "p.png").unwrap();
        assert_eq!(img.kind, "image");
        assert_eq!(img.data_url.as_deref(), Some("data:image/png;base64,iVBORw=="));
        assert!(read(&root, "../x").is_err());
        assert!(read(&root, "C:/x").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn status_and_diff_read_a_repo_and_say_so_when_there_is_none() {
        let root = temp("git");
        assert!(!status(&root).unwrap().repo, "a plain folder is not a repo");
        let run = |args: &[&str]| {
            let out = git(&root, args).unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "a"]);
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("new.md"), "# new\n").unwrap();
        let s = status(&root).unwrap();
        assert!(s.repo);
        assert_eq!(s.branch, "main");
        let mut codes: Vec<(String, String)> = s.changes.iter().map(|c| (c.path.clone(), c.code.clone())).collect();
        codes.sort();
        assert_eq!(codes, vec![("a.txt".to_string(), " M".to_string()), ("new.md".to_string(), "??".to_string())]);
        assert!(diff(&root, "a.txt", false).unwrap().contains("+two"));
        assert!(diff(&root, "new.md", true).unwrap().contains("+# new"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
