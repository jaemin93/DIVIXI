//! Workers' own checkouts.
//!
//! When a track's folder is inside a git repository, each worker works in a
//! git worktree of its own (branch `divixi/<track>/<worker>`, folder
//! `<app data>/worktrees/<track>/<worker>`), started from the track folder's
//! current state, uncommitted edits included (`git stash create`). Nothing
//! the worker does reaches the track folder until the human merges it.
//! Outside git, workers share the track folder as before.
//!
//! Merging goes file by file rather than through `git apply`: a patch made
//! from committed blobs does not apply to a working tree with CRLF endings.
//! For each file the worker changed: if the track folder still has the
//! base version, the worker's file is copied over; if both sides changed
//! it, `git merge-file` merges them (line endings normalised); if that
//! conflicts, nothing at all is written and the conflicting files are
//! named.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use orchestra_store::Store;

/// A worker's checkout, as remembered in the store (`worktree:<track>/<worker>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkout {
    /// The worktree's folder.
    pub path: PathBuf,
    pub branch: String,
    /// The commit the worker's changes are measured from: where it started,
    /// or its last merged or discarded state.
    pub base: String,
    /// The track folder relative to the repository's top, `/`-separated
    /// ("" when the track is the top).
    pub sub: String,
    /// The repository's top, where merges land.
    pub top: PathBuf,
}

impl Checkout {
    /// Where the worker's agent runs: the track folder's place in the worktree.
    pub fn cwd(&self) -> PathBuf {
        if self.sub.is_empty() {
            self.path.clone()
        } else {
            self.path.join(&self.sub)
        }
    }
}

/// One file a worker changed and the human has not merged.
#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    /// Relative to the repository's top.
    pub path: String,
    /// `A`dded, `M`odified or `D`eleted.
    pub status: String,
    /// Lines, `None` for binary files.
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

/// What a worker has waiting, for the report card.
#[derive(Debug, Clone, Serialize)]
pub struct Changes {
    /// Whether the worker works in its own checkout (false: the shared track folder).
    pub isolated: bool,
    pub branch: String,
    /// The track folder relative to the repository's top, to show paths as the track sees them.
    pub sub: String,
    pub files: Vec<Pending>,
}

/// How a merge ended.
#[derive(Debug, Clone, Serialize)]
pub struct Merged {
    /// Files written into the track folder (relative to the repository's top).
    pub files: Vec<String>,
    /// Files both sides changed and could not be merged; when any, nothing was written.
    pub conflicts: Vec<String>,
}

fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|e| format!("git: {e}"))
}

/// stdout of a git command that must succeed.
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(dir, args)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let verb = args.iter().find(|a| !a.starts_with('-') && !a.contains('=')).unwrap_or(&"");
        return Err(format!("git {verb}: {err}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Snapshot commits in a worktree carry the app's name, not the human's,
/// and never run the repository's hooks or ask for a signature.
const IDENTITY: [&str; 6] = ["-c", "user.name=Divixi", "-c", "user.email=divixi@localhost", "-c", "commit.gpgsign=false"];

/// The top of the git repository `cwd` is in, if it is in one with a commit.
pub fn repository(cwd: &Path) -> Option<PathBuf> {
    let top = run(cwd, &["rev-parse", "--show-toplevel"]).ok()?;
    // A repository without a commit has nothing to branch from.
    run(cwd, &["rev-parse", "--verify", "-q", "HEAD"]).ok()?;
    Some(PathBuf::from(top.trim()))
}

fn key(track: &str, worker: &str) -> String {
    format!("worktree:{track}/{worker}")
}

/// Letters git and file systems take in a branch or folder name.
fn slug(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "worker".to_string()
    } else {
        s
    }
}

pub fn record(store: &Store, track: &str, worker: &str) -> Option<Checkout> {
    let json = store.get_meta(&key(track, worker)).ok().flatten()?;
    serde_json::from_str(&json).ok()
}

fn remember(store: &Store, track: &str, worker: &str, c: &Checkout) {
    match serde_json::to_string(c) {
        Ok(json) => {
            if let Err(err) = store.set_meta(&key(track, worker), &json) {
                tracing::warn!(%err, "could not remember a worker's checkout");
            }
        }
        Err(err) => tracing::warn!(%err, "could not encode a worker's checkout"),
    }
}

/// The track folder's state as a commit: HEAD with uncommitted changes to
/// tracked files on top (untracked files stay out), or HEAD itself.
fn snapshot_of(top: &Path) -> Result<String, String> {
    let stash = run(top, &["stash", "create"])?;
    let stash = stash.trim();
    if stash.is_empty() {
        Ok(run(top, &["rev-parse", "HEAD"])?.trim().to_string())
    } else {
        Ok(stash.to_string())
    }
}

/// Everything the worker has done, staged, so it can be diffed against `base`.
fn stage_all(c: &Checkout) -> Result<(), String> {
    run(&c.path, &["add", "-A"]).map(|_| ())
}

/// The files the worker changed since `base`.
fn pending_of(c: &Checkout) -> Result<Vec<Pending>, String> {
    stage_all(c)?;
    let status = run(&c.path, &["diff", "--cached", "--no-renames", "--name-status", "-z", &c.base])?;
    let numstat = run(&c.path, &["diff", "--cached", "--no-renames", "--numstat", "-z", &c.base])?;
    let mut counts = std::collections::HashMap::new();
    for rec in numstat.split('\0').filter(|r| !r.is_empty()) {
        let mut parts = rec.splitn(3, '\t');
        let (a, r, p) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        counts.insert(p.to_string(), (a.parse().ok(), r.parse().ok()));
    }
    let mut files = Vec::new();
    let mut it = status.split('\0').filter(|r| !r.is_empty());
    while let (Some(st), Some(path)) = (it.next(), it.next()) {
        let (added, removed) = counts.get(path).copied().unwrap_or((None, None));
        files.push(Pending {
            path: path.to_string(),
            status: st.chars().next().unwrap_or('M').to_string(),
            added,
            removed,
        });
    }
    Ok(files)
}

/// The worker's checkout, made the first time; `None` outside git, where
/// the worker shares the track folder. A checkout with nothing pending is
/// brought up to the track folder's current state, so a worker picks up
/// what was merged or edited since.
pub fn ensure(store: &Store, worktrees_dir: &Path, track: &str, worker: &str, cwd: &Path) -> Result<Option<Checkout>, String> {
    let Some(top) = repository(cwd) else { return Ok(None) };
    if let Some(mut c) = record(store, track, worker) {
        if c.path.join(".git").exists() {
            if pending_of(&c)?.is_empty() {
                let fresh = snapshot_of(&top)?;
                if fresh != c.base {
                    run(&c.path, &["reset", "-q", "--hard", &fresh])?;
                    run(&c.path, &["clean", "-fdq"])?;
                    c.base = fresh;
                    remember(store, track, worker, &c);
                }
            }
            return Ok(Some(c));
        }
        // Its folder is gone: make it again below.
    }
    let top_c = top.canonicalize().map_err(|e| e.to_string())?;
    let cwd_c = cwd.canonicalize().map_err(|e| e.to_string())?;
    let sub = cwd_c
        .strip_prefix(&top_c)
        .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
        .unwrap_or_default();
    let path = worktrees_dir.join(slug(track)).join(slug(worker));
    let branch = format!("divixi/{}/{}", slug(track), slug(worker));
    let base = snapshot_of(&top)?;
    let _ = run(&top, &["worktree", "prune"]);
    if path.exists() {
        let _ = run(&top, &["worktree", "remove", "--force", &path.to_string_lossy()]);
        let _ = std::fs::remove_dir_all(&path);
    }
    if run(&top, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_ok() {
        run(&top, &["branch", "-D", &branch])?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    run(&top, &["worktree", "add", "-q", "-b", &branch, &path.to_string_lossy(), &base])?;
    let c = Checkout { path, branch, base, sub, top };
    remember(store, track, worker, &c);
    tracing::info!(%track, %worker, path = %c.path.display(), "worker checkout made");
    Ok(Some(c))
}

/// What a worker has waiting.
pub fn changes(store: &Store, track: &str, worker: &str) -> Result<Changes, String> {
    let Some(c) = record(store, track, worker).filter(|c| c.path.join(".git").exists()) else {
        return Ok(Changes { isolated: false, branch: String::new(), sub: String::new(), files: Vec::new() });
    };
    Ok(Changes { isolated: true, branch: c.branch.clone(), sub: c.sub.clone(), files: pending_of(&c)? })
}

/// One pending file's diff, as the worker changed it.
pub fn file_diff(store: &Store, track: &str, worker: &str, path: &str) -> Result<String, String> {
    let c = record(store, track, worker).ok_or("the worker has no checkout")?;
    stage_all(&c)?;
    let out = git(&c.path, &["diff", "--cached", "--no-renames", &c.base, "--", path])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A blob of a commit, `None` when the commit does not have the path.
fn blob(top: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    let out = git(top, &["cat-file", "blob", &format!("{commit}:{path}")]).ok()?;
    out.status.success().then_some(out.stdout)
}

fn lf(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a == b || lf(a) == lf(b)
}

/// What merging one file does.
enum Step {
    Write(PathBuf, Vec<u8>),
    Remove(PathBuf),
    Nothing,
}

/// Three-way merge of text with `git merge-file`; `None` when it conflicts
/// or a side is not text.
fn merge_text(scratch: &Path, ours: &[u8], base: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    let text = |b: &[u8]| std::str::from_utf8(b).is_ok() && !b.contains(&0);
    if !(text(ours) && text(base) && text(theirs)) {
        return None;
    }
    let crlf = ours.windows(2).any(|w| w == b"\r\n");
    std::fs::create_dir_all(scratch).ok()?;
    let (o, b, t) = (scratch.join("ours"), scratch.join("base"), scratch.join("theirs"));
    std::fs::write(&o, lf(ours)).ok()?;
    std::fs::write(&b, lf(base)).ok()?;
    std::fs::write(&t, lf(theirs)).ok()?;
    let out = git(scratch, &["merge-file", "-p", "ours", "base", "theirs"]).ok()?;
    let _ = std::fs::remove_dir_all(scratch);
    if !out.status.success() {
        return None;
    }
    let merged = out.stdout;
    Some(if crlf {
        String::from_utf8_lossy(&merged).replace('\n', "\r\n").into_bytes()
    } else {
        merged
    })
}

/// Bring the worker's changes into the track folder. All or nothing: when
/// any file conflicts, nothing is written and the conflicts are named.
pub fn merge(store: &Store, track: &str, worker: &str, scratch: &Path) -> Result<Merged, String> {
    let mut c = record(store, track, worker).ok_or("the worker has no checkout")?;
    stage_all(&c)?;
    // A snapshot commit on the worker's branch: what is merged, by name.
    let staged = git(&c.path, &["diff", "--cached", "--quiet", "HEAD"])?;
    if !staged.status.success() {
        let mut args: Vec<&str> = IDENTITY.to_vec();
        let msg = format!("divixi: {worker}");
        args.extend(["commit", "-q", "--no-verify", "-m", &msg]);
        run(&c.path, &args)?;
    }
    let tip = run(&c.path, &["rev-parse", "HEAD"])?.trim().to_string();
    let listed = run(&c.path, &["diff", "--no-renames", "--name-status", "-z", &c.base, &tip])?;
    let mut steps = Vec::new();
    let mut conflicts = Vec::new();
    let mut files = Vec::new();
    let mut it = listed.split('\0').filter(|r| !r.is_empty());
    while let (Some(st), Some(path)) = (it.next(), it.next()) {
        let target = c.top.join(path);
        let current = std::fs::read(&target).ok();
        let base = blob(&c.top, &c.base, path);
        let step = if st.starts_with('D') {
            match (&current, &base) {
                (None, _) => Step::Nothing,
                (Some(cur), Some(b)) if same(cur, b) => Step::Remove(target),
                _ => {
                    conflicts.push(path.to_string());
                    continue;
                }
            }
        } else {
            // The worker's file as it sits in its checkout (its line endings
            // are what this machine's git gives a working tree).
            let theirs = std::fs::read(c.path.join(path)).map_err(|e| format!("{path}: {e}"))?;
            match (&current, &base) {
                (None, None) => Step::Write(target, theirs),
                (Some(cur), _) if same(cur, &theirs) => Step::Nothing,
                (Some(cur), Some(b)) if same(cur, b) => Step::Write(target, theirs),
                (Some(cur), Some(b)) => match merge_text(&scratch.join("merge"), cur, b, &theirs) {
                    Some(merged) => Step::Write(target, merged),
                    None => {
                        conflicts.push(path.to_string());
                        continue;
                    }
                },
                // Deleted in the track folder but changed by the worker, or
                // added on both sides differently.
                _ => {
                    conflicts.push(path.to_string());
                    continue;
                }
            }
        };
        if !matches!(step, Step::Nothing) {
            files.push(path.to_string());
        }
        steps.push(step);
    }
    if !conflicts.is_empty() {
        return Ok(Merged { files: Vec::new(), conflicts });
    }
    for step in steps {
        match step {
            Step::Write(target, bytes) => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            Step::Remove(target) => std::fs::remove_file(&target).map_err(|e| format!("{}: {e}", target.display()))?,
            Step::Nothing => {}
        }
    }
    c.base = tip;
    remember(store, track, worker, &c);
    Ok(Merged { files, conflicts: Vec::new() })
}

/// Throw away what the worker has pending: its checkout goes back to `base`.
pub fn discard(store: &Store, track: &str, worker: &str) -> Result<(), String> {
    let c = record(store, track, worker).ok_or("the worker has no checkout")?;
    run(&c.path, &["reset", "-q", "--hard", &c.base])?;
    run(&c.path, &["clean", "-fdq"])?;
    Ok(())
}

/// Remove every worker checkout of a track (it is being deleted).
pub fn remove_track(store: &Store, track: &str, workers: &[String]) {
    for worker in workers {
        let Some(c) = record(store, track, worker) else { continue };
        if let Err(err) = run(&c.top, &["worktree", "remove", "--force", &c.path.to_string_lossy()]) {
            tracing::warn!(%err, "could not remove a worker checkout");
            let _ = std::fs::remove_dir_all(&c.path);
            let _ = run(&c.top, &["worktree", "prune"]);
        }
        let _ = run(&c.top, &["branch", "-D", &c.branch]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's text with CRLF as LF (git may check files out either way).
    fn text(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
    }

    fn sh(dir: &Path, args: &[&str]) {
        let mut all: Vec<&str> = IDENTITY.to_vec();
        all.extend(args);
        run(dir, &all).unwrap();
    }

    #[test]
    fn a_worker_checkout_from_start_to_merge() {
        let root = std::env::temp_dir().join(format!("divixi-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let track = repo.join("app");
        std::fs::create_dir_all(&track).unwrap();
        sh(&repo, &["init", "-q"]);
        std::fs::write(track.join("a.txt"), "1\n2\n3\n4\n5\n").unwrap();
        std::fs::write(track.join("same.txt"), "x\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "start"]);
        // The human's uncommitted edit is where the worker starts.
        std::fs::write(track.join("same.txt"), "human\n").unwrap();

        let store = Store::in_memory().unwrap();
        let dirs = root.join("worktrees");
        let c = ensure(&store, &dirs, "tr001", "fix", &track).unwrap().expect("a git track gets a checkout");
        assert_eq!(c.sub, "app");
        assert_eq!(text(&c.cwd().join("same.txt")), "human\n");
        assert!(changes(&store, "tr001", "fix").unwrap().files.is_empty());

        // The worker edits the last line and adds a file; the human edits the first line.
        std::fs::write(c.cwd().join("a.txt"), "1\n2\n3\n4\nFIVE\n").unwrap();
        std::fs::write(c.cwd().join("new.txt"), "hi\n").unwrap();
        std::fs::write(track.join("a.txt"), "ONE\n2\n3\n4\n5\n").unwrap();
        let pending = changes(&store, "tr001", "fix").unwrap().files;
        assert_eq!(pending.iter().map(|p| (p.path.as_str(), p.status.as_str())).collect::<Vec<_>>(), [("app/a.txt", "M"), ("app/new.txt", "A")]);
        assert!(file_diff(&store, "tr001", "fix", "app/a.txt").unwrap().contains("+FIVE"));

        let merged = merge(&store, "tr001", "fix", &root.join("scratch")).unwrap();
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        assert_eq!(text(&track.join("a.txt")), "ONE\n2\n3\n4\nFIVE\n");
        assert_eq!(text(&track.join("new.txt")), "hi\n");
        assert!(changes(&store, "tr001", "fix").unwrap().files.is_empty(), "merged means nothing pending");

        // Next turn: the checkout catches up with the track folder.
        std::fs::write(track.join("later.txt"), "later\n").unwrap();
        sh(&repo, &["add", "app/later.txt"]);
        let c = ensure(&store, &dirs, "tr001", "fix", &track).unwrap().unwrap();
        assert!(c.cwd().join("later.txt").exists());

        // Both change the same line: nothing is written.
        std::fs::write(c.cwd().join("a.txt"), "worker\n2\n3\n4\nFIVE\n").unwrap();
        std::fs::write(track.join("a.txt"), "human\n2\n3\n4\nFIVE\n").unwrap();
        let clash = merge(&store, "tr001", "fix", &root.join("scratch")).unwrap();
        assert_eq!(clash.conflicts, ["app/a.txt"]);
        assert_eq!(text(&track.join("a.txt")), "human\n2\n3\n4\nFIVE\n");

        discard(&store, "tr001", "fix").unwrap();
        assert!(changes(&store, "tr001", "fix").unwrap().files.is_empty());

        remove_track(&store, "tr001", &["fix".to_string()]);
        assert!(!c.path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn outside_git_there_is_no_checkout() {
        let dir = std::env::temp_dir().join(format!("divixi-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::in_memory().unwrap();
        // The temp folder may sit inside some repository on a dev machine; only assert when it does not.
        if repository(&dir).is_none() {
            assert!(ensure(&store, &dir.join("wt"), "tr001", "w", &dir).unwrap().is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_endings_do_not_make_a_difference() {
        assert!(same(b"a\r\nb\r\n", b"a\nb\n"));
        assert!(!same(b"a\nb\n", b"a\nc\n"));
        assert_eq!(slug("Fix Parser!"), "fix-parser");
        assert_eq!(slug("///"), "worker");
    }

    #[test]
    fn both_sides_edit_different_lines() {
        let dir = std::env::temp_dir().join(format!("divixi-merge-{}", std::process::id()));
        // Adjacent lines count as one change to git; these are apart.
        let merged = merge_text(&dir, b"ONE\r\n2\r\n3\r\n4\r\nfive\r\n", b"one\n2\n3\n4\nfive\n", b"one\n2\n3\n4\nFIVE\n").expect("clean merge");
        assert_eq!(merged, b"ONE\r\n2\r\n3\r\n4\r\nFIVE\r\n");
        assert!(merge_text(&dir, b"one\nX\n", b"one\ntwo\n", b"one\nY\n").is_none(), "same line both ways conflicts");
    }
}
