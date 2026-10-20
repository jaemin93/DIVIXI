//! Where a worker may write besides its own folder.
//!
//! A track's workers each get a folder of their own under the track's and
//! are told to write only there. That is the right default for work that
//! produces something new, and the wrong one for a worker whose whole job
//! is `apps/web`: every report it sends comes back flagged "edits OUTSIDE
//! the worker's folder", the flag means nothing, and the one time it would
//! have mattered it is read past like all the others.
//!
//! So a worker can be given a scope: globs over the track folder it may
//! write under as well. Edits inside it are ordinary; the flag is kept for
//! edits outside both. The conductor sets it when it opens the worker
//! (`spawn_worker` takes `scope`) or later (`set_worker_scope`), and the
//! worker is told.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// A glob as written here: `*` stays within one folder and `**` crosses
/// them, as in a `.gitignore`.
fn glob(g: &str) -> Result<globset::Glob, globset::Error> {
    GlobBuilder::new(g).literal_separator(true).build()
}

use crate::AppState;

fn key(track: &str, worker: &str) -> String {
    format!("worker_scope:{track}/{worker}")
}

/// Check and tidy a list of globs: relative to the track folder, no
/// climbing out, each one a glob `globset` takes.
pub fn check(globs: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for g in globs {
        let g = g.trim().trim_start_matches("./").replace('\\', "/");
        if g.is_empty() {
            continue;
        }
        if g.starts_with('/') || g.get(1..3).is_some_and(|s| s == ":/") || g.split('/').any(|p| p == "..") {
            return Err(format!("a scope is relative to the track folder and does not climb out of it: {g:?}"));
        }
        glob(&g).map_err(|e| format!("not a glob: {g:?} ({e})"))?;
        if !out.contains(&g) {
            out.push(g);
        }
    }
    Ok(out)
}

/// A worker's scope as kept, or none.
pub fn get(state: &AppState, track: &str, worker: &str) -> Vec<String> {
    state
        .store
        .get_meta(&key(track, worker))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Keep a worker's scope (an empty list forgets it). Returns it as kept.
pub fn set(state: &AppState, track: &str, worker: &str, globs: &[String]) -> Result<Vec<String>, String> {
    let clean = check(globs)?;
    if clean.is_empty() {
        state.store.delete_meta(&key(track, worker)).map_err(|e| e.to_string())?;
    } else {
        state.store.set_meta(&key(track, worker), &serde_json::to_string(&clean).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(clean)
}

/// The globs compiled; `None` for an empty or unreadable scope.
fn compile(globs: &[String]) -> Option<GlobSet> {
    if globs.is_empty() {
        return None;
    }
    let mut b = GlobSetBuilder::new();
    for g in globs {
        b.add(glob(g).ok()?);
    }
    b.build().ok()
}

/// Whether `path` (as the agent named it: absolute, or relative to the
/// folder it ran in) is under the track folder and inside the scope.
/// Returns the path relative to the track folder when it is.
pub fn allows(globs: &[String], track_dir: &str, path: &str) -> Option<String> {
    let set = compile(globs)?;
    let root = track_dir.replace('\\', "/").trim_end_matches('/').to_string() + "/";
    let p = path.replace('\\', "/");
    // A file system may ignore case (macOS and Windows do by default, not
    // always), so an agent may name the track folder in another case. That
    // is the track only when the disk says the two are one folder;
    // otherwise `/srv/Foo` and `/srv/foo` are two.
    let same = |head: &str| head == root || (head.eq_ignore_ascii_case(&root) && same_file::is_same_file(head, &root).unwrap_or(false));
    let rel = if p.is_char_boundary(root.len()) && p.get(..root.len()).is_some_and(same) {
        p[root.len()..].to_string()
    } else if p.starts_with('/') || p.get(1..3).is_some_and(|s| s == ":/") {
        // Absolute and elsewhere: not under the track at all.
        return None;
    } else {
        p
    };
    // A path that climbs (`apps/web/../../etc`) is matched by nobody: the
    // glob would read it as under apps/web, the file system does not.
    // Such an edit keeps its warning.
    if rel.split('/').any(|part| part == ".." || part == ".") {
        return None;
    }
    set.is_match(&rel).then_some(rel)
}

/// What the worker is told about its scope, in its task.
pub fn told(globs: &[String], track_dir: &str) -> String {
    if globs.is_empty() {
        return String::new();
    }
    format!(" You may also write under these paths of {track_dir}, which are yours: {}.", globs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_is_relative_globs_and_nothing_else() {
        assert_eq!(check(&["apps/web/**".into(), " ./docs/*.md ".into(), "apps/web/**".into()]).unwrap(), vec!["apps/web/**", "docs/*.md"]);
        assert!(check(&["/etc/**".into()]).is_err(), "absolute");
        assert!(check(&["../other/**".into()]).is_err(), "climbs out");
        assert!(check(&["a[".into()]).is_err(), "not a glob");
        assert!(check(&["".into()]).unwrap().is_empty());
    }

    #[test]
    fn edits_inside_the_scope_are_the_workers_own() {
        let scope = vec!["apps/web/**".to_string(), "docs/*.md".to_string()];
        let root = "/home/u/many-startup";
        assert_eq!(allows(&scope, root, "/home/u/many-startup/apps/web/src/page.tsx").as_deref(), Some("apps/web/src/page.tsx"));
        assert_eq!(allows(&scope, root, "apps/web/next.config.js").as_deref(), Some("apps/web/next.config.js"), "relative to the track folder");
        assert_eq!(allows(&scope, root, "docs/README.md").as_deref(), Some("docs/README.md"));
        assert_eq!(allows(&scope, root, "docs/deep/README.md"), None, "one level only, as written");
        assert_eq!(allows(&scope, root, "/home/u/many-startup/src/core.ts"), None);
        assert_eq!(allows(&scope, root, "/etc/hosts"), None);
        // Climbing out through the scope is not inside it.
        assert_eq!(allows(&scope, root, "/home/u/many-startup/apps/web/../../../etc/hosts"), None);
        assert_eq!(allows(&scope, root, "apps/web/../../src/core.ts"), None);
        assert_eq!(allows(&scope, root, "apps/web/./x.ts"), None, "a `.` is not matched either; the agent names files plainly");
        assert_eq!(allows(&[], root, "apps/web/x"), None, "no scope, nothing allowed by it");
        // Another case is not the track when neither is on disk to ask.
        assert_eq!(allows(&scope, root, "/home/u/Many-Startup/apps/web/x.ts"), None);
        // On disk: the same folder only where the file system says so.
        let dir = std::env::temp_dir().join(format!("divixi-Scope-Case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.to_string_lossy().replace('\\', "/");
        let flipped = real.replace("divixi-Scope-Case", "DIVIXI-sCOPE-cASE");
        let one_folder = same_file::is_same_file(&real, &flipped).unwrap_or(false);
        let got = allows(&scope, &real, &format!("{flipped}/apps/web/x.ts"));
        assert_eq!(got.as_deref(), one_folder.then_some("apps/web/x.ts"), "case-insensitive here: {one_folder}");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(told(&scope, root).contains("apps/web/**"));
        assert_eq!(told(&[], root), "");
    }
}
