//! A folder's files and subfolders on this machine, for a picker another
//! device draws (a remote instance's window, a phone): adding a document to
//! the knowledge library there picks a file on this machine, where it is
//! read and indexed, not one on the device asking.
//!
//! Read only: names, kinds and sizes, never a byte of a file. The path asked
//! for is made absolute and canonical first (`..` and links in it resolved),
//! so what is shown is where it really is. Entries that are links (symlinks,
//! and junctions on Windows) are left out rather than followed: a folder
//! lists what is in it, and nothing reached through it. Hidden entries
//! (a leading dot, or Windows' hidden attribute) are left out unless asked.

use std::path::{Path, PathBuf};

use orchestra_core::path::plain;
use serde::Serialize;

/// The most entries one folder lists; the rest are counted out (`more`).
pub const MAX_ENTRIES: usize = 5000;

/// One entry of a folder: `dir` or `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// Bytes, for a file; 0 for a folder.
    pub size: u64,
}

/// A folder as the picker shows it.
#[derive(Debug, Serialize)]
pub struct Listing {
    /// The folder, canonical and plain (never `\\?\`).
    pub path: String,
    pub parent: Option<String>,
    pub home: String,
    /// Folders first, then files, each by name without case.
    pub entries: Vec<Entry>,
    /// Whether entries past [`MAX_ENTRIES`] were left out.
    pub more: bool,
}

/// The home folder of the user this Divixi runs as.
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(crate::workspace_root)
}

/// The folder `asked` names, canonical. Nothing (or blank) is `home`; `~`
/// and `~/…` start there too. Anything else must be absolute: a relative
/// path would be read from wherever the server happened to start.
pub fn resolve(asked: Option<&str>, home: &Path) -> Result<PathBuf, String> {
    let asked = asked.map(str::trim).filter(|p| !p.is_empty());
    let path = match asked {
        None | Some("~") => home.to_path_buf(),
        Some(p) if p.contains('\0') => return Err("not a path".to_string()),
        Some(p) => match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
            Some(rest) => home.join(rest),
            None => PathBuf::from(p),
        },
    };
    if !path.is_absolute() {
        return Err(format!("{} is not a full path", path.display()));
    }
    let dir = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", show(&dir)));
    }
    Ok(dir)
}

/// What `dir` holds, as [`Entry`]s.
pub fn list(dir: &Path, hidden: bool) -> Result<(Vec<Entry>, bool), String> {
    let mut entries = Vec::new();
    let mut more = false;
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", show(dir)))?.filter_map(Result::ok) {
        // The entry's own type: a link is never followed to find out.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Its own metadata too (not the target's): read for the size, and
        // on Windows for the hidden attribute.
        let meta = entry.metadata().ok();
        if !hidden && is_hidden(&name, meta.as_ref()) {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            more = true;
            break;
        }
        let dir = kind.is_dir();
        let size = if dir { 0 } else { meta.map(|m| m.len()).unwrap_or(0) };
        entries.push(Entry { name, dir, size });
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok((entries, more))
}

fn is_hidden(name: &str, meta: Option<&std::fs::Metadata>) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if meta.is_some_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0) {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = meta;
    false
}

/// A path as shown and as picked: never `\\?\`.
fn show(p: &Path) -> String {
    plain(&p.display().to_string())
}

/// A folder of this machine with its files, for picking a file on it from
/// another device (the knowledge library's "Add source" on a remote
/// instance or a phone). No path: the home folder. `hidden` lists hidden
/// entries too.
#[tauri::command(async)]
pub fn browse_files(path: Option<String>, hidden: Option<bool>) -> Result<Listing, String> {
    let home = home();
    let dir = resolve(path.as_deref(), &home)?;
    let (entries, more) = list(&dir, hidden.unwrap_or(false))?;
    Ok(Listing { path: show(&dir), parent: dir.parent().map(show), home: show(&home), entries, more })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system's temp folder, removed on drop. Its
    /// path is the plain one, as typed; `canon` is what `resolve` answers.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("divixi-browse-{tag}-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    fn canon(p: &Path) -> PathBuf {
        p.canonicalize().unwrap()
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nothing_or_a_tilde_is_home() {
        let home = Scratch::new("home");
        std::fs::create_dir(home.0.join("docs")).unwrap();
        assert_eq!(resolve(None, &home.0).unwrap(), canon(&home.0));
        assert_eq!(resolve(Some("  "), &home.0).unwrap(), canon(&home.0));
        assert_eq!(resolve(Some("~"), &home.0).unwrap(), canon(&home.0));
        assert_eq!(resolve(Some("~/docs"), &home.0).unwrap(), canon(&home.0.join("docs")));
        assert_eq!(resolve(Some("~\\docs"), &home.0).unwrap(), canon(&home.0.join("docs")));
    }

    #[test]
    fn a_path_is_made_canonical() {
        let s = Scratch::new("canon");
        std::fs::create_dir_all(s.0.join("a").join("b")).unwrap();
        let typed = format!("  {}  ", s.0.join("a").join("b").join("..").display());
        assert_eq!(resolve(Some(&typed), &s.0).unwrap(), canon(&s.0.join("a")));
    }

    #[test]
    fn relative_missing_files_and_nul_are_refused() {
        let s = Scratch::new("refuse");
        std::fs::write(s.0.join("note.md"), "x").unwrap();
        assert!(resolve(Some("docs"), &s.0).unwrap_err().contains("not a full path"));
        assert!(resolve(Some("../etc"), &s.0).unwrap_err().contains("not a full path"));
        assert!(resolve(Some(&s.0.join("gone").display().to_string()), &s.0).is_err());
        assert!(resolve(Some(&s.0.join("note.md").display().to_string()), &s.0).unwrap_err().contains("is not a folder"));
        assert_eq!(resolve(Some("/tmp\0x"), &s.0).unwrap_err(), "not a path");
    }

    #[test]
    fn folders_first_then_files_with_sizes() {
        let s = Scratch::new("list");
        std::fs::create_dir(s.0.join("zeta")).unwrap();
        std::fs::create_dir(s.0.join("Alpha")).unwrap();
        std::fs::write(s.0.join("b.md"), "hello").unwrap();
        std::fs::write(s.0.join("A.txt"), "").unwrap();
        let (entries, more) = list(&s.0, false).unwrap();
        assert!(!more);
        let got: Vec<(&str, bool, u64)> = entries.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect();
        assert_eq!(got, vec![("Alpha", true, 0), ("zeta", true, 0), ("A.txt", false, 0), ("b.md", false, 5)]);
    }

    #[test]
    fn hidden_entries_only_when_asked() {
        let s = Scratch::new("hidden");
        std::fs::create_dir(s.0.join(".git")).unwrap();
        std::fs::write(s.0.join(".env"), "SECRET=1").unwrap();
        std::fs::write(s.0.join("seen.md"), "").unwrap();
        let names = |hidden| list(&s.0, hidden).unwrap().0.into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(false), vec!["seen.md"]);
        assert_eq!(names(true), vec![".git", ".env", "seen.md"]);
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_hidden_attribute_hides() {
        let s = Scratch::new("attr");
        let f = s.0.join("desktop.ini");
        std::fs::write(&f, "").unwrap();
        let ok = std::process::Command::new("attrib").arg("+h").arg(&f).status().map(|st| st.success()).unwrap_or(false);
        if !ok {
            return; // no attrib here: nothing to check
        }
        assert!(list(&s.0, false).unwrap().0.is_empty());
        assert_eq!(list(&s.0, true).unwrap().0.len(), 1);
    }

    /// A link is left out, whether it points in or out of the folder: the
    /// picker never walks through one.
    #[test]
    fn links_are_not_followed() {
        let s = Scratch::new("links");
        let outside = Scratch::new("outside");
        std::fs::write(outside.0.join("secret.md"), "x").unwrap();
        std::fs::write(s.0.join("real.md"), "x").unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside.0, s.0.join("out")).is_ok() && std::os::unix::fs::symlink(outside.0.join("secret.md"), s.0.join("secret.md")).is_ok();
        // A junction needs no privilege on Windows, unlike a symlink.
        #[cfg(windows)]
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(s.0.join("out"))
            .arg(&outside.0)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !made {
            return; // links cannot be made here
        }
        let names: Vec<String> = list(&s.0, true).unwrap().0.into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn a_long_folder_is_cut() {
        let s = Scratch::new("many");
        for i in 0..MAX_ENTRIES + 3 {
            std::fs::write(s.0.join(format!("f{i}.md")), "").unwrap();
        }
        let (entries, more) = list(&s.0, false).unwrap();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert!(more);
    }

    #[test]
    fn the_command_answers_plain_paths() {
        let s = Scratch::new("cmd");
        std::fs::create_dir(s.0.join("marker")).unwrap();
        let got = browse_files(Some(s.0.display().to_string()), None).unwrap();
        assert!(!got.path.starts_with(r"\\?\"), "{}", got.path);
        assert_eq!(got.entries, vec![Entry { name: "marker".into(), dir: true, size: 0 }]);
        assert!(got.parent.is_some());
    }
}
