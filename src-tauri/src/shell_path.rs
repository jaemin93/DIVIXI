//! The PATH an app opened from Finder does not get.
//!
//! macOS starts an app opened from Finder, the Dock or Spotlight through
//! launchd, and launchd gives it `PATH=/usr/bin:/bin:/usr/sbin:/sbin` and
//! nothing more; opened from a launcher such as Raycast, it has
//! `/usr/local/bin` in front. `node` is in none of those — Homebrew puts it in
//! `/opt/homebrew/bin`, nvm, fnm and volta under the home folder — and the
//! Claude Code and Codex adapters run under `node`, so every agent failed to
//! start with "failed to launch node: No such file or directory". `npm run
//! app` and `open` from a terminal pass the shell's PATH through, which is
//! why a development build never showed it.
//!
//! So when the PATH is launchd's and nothing more, the app asks the user's
//! login shell for the PATH it sets up, and puts that in front of its own.
//! It is the macOS side of what `registry_path_dirs` does on Windows
//! (crates/agents): the PATH as the user set it, which a process started
//! some other way never sees. A PATH with anything more in it was set by
//! someone on purpose — a terminal, or `launchctl config user path` — and
//! is left alone.
//!
//! It changes this process's environment, so [`adopt`] runs at the top of
//! `run`, before any thread that could read it exists. The one thread it
//! starts itself reads the shell's output and nothing else. Logging has not
//! started yet, so what it did waits in [`OUTCOME`] until [`log`] can say.

use std::ffi::{OsStr, OsString};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The directories a PATH nobody set is made of. launchd's own is the last
/// four; an app opened from a launcher such as Raycast gets `/usr/local/bin`
/// in front of them, which on Apple Silicon holds no Homebrew and so no
/// `node` either.
const LAUNCHD_DIRS: [&str; 5] = ["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// Printed either side of the PATH, on its line. A startup file may print
/// anything — a greeting, a terminal title, a shell integration's escape
/// codes, a background job's own output at any moment — so the PATH is what
/// lies between these two on one line, and nothing else is taken for it.
const START: &str = "__DIVIXI_PATH_START__";
const END: &str = "__DIVIXI_PATH_END__";

/// How long the login shell may take. A heavy `.zshrc` takes a second; a
/// shell that is still going after this is stuck, and the window is waiting.
const TIMEOUT: Duration = Duration::from_secs(5);

/// When neither `SHELL` nor the account says: the default login shell since
/// macOS 10.15.
const FALLBACK_SHELL: &str = "/bin/zsh";

/// What [`adopt`] did, for the log.
#[derive(Debug)]
enum Outcome {
    /// The PATH already had more than launchd's directories.
    Kept,
    /// The login shell's PATH is in front; `added` directories are new.
    Adopted { shell: String, added: usize },
    /// The login shell did not say, so the PATH is still launchd's.
    Failed { shell: String, error: String },
}

static OUTCOME: OnceLock<Outcome> = OnceLock::new();

/// Take the login shell's PATH when this process has only launchd's.
///
/// Call once, before any other thread exists: it sets `PATH`.
pub fn adopt() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let outcome = if !is_launchd_default(&current) {
        Outcome::Kept
    } else {
        let shell = login_shell();
        match read(&shell, &[], TIMEOUT) {
            Ok(found) => {
                let (path, added) = merge(&found, &current);
                std::env::set_var("PATH", path);
                Outcome::Adopted { shell: shell.display().to_string(), added }
            }
            Err(error) => Outcome::Failed { shell: shell.display().to_string(), error },
        }
    };
    let _ = OUTCOME.set(outcome);
}

/// Say what [`adopt`] did. Call once logging has started.
pub fn log() {
    match OUTCOME.get() {
        Some(Outcome::Adopted { shell, added }) => tracing::info!(%shell, added, "PATH taken from the login shell"),
        Some(Outcome::Failed { shell, error }) => {
            tracing::warn!(%shell, %error, "the PATH is launchd's and the login shell's could not be read: agents that run under node may not start")
        }
        Some(Outcome::Kept) | None => {}
    }
}

/// The user's login shell: `SHELL`, which launchd sets from the account, or
/// else the account's own record, or else [`FALLBACK_SHELL`]. Running zsh for
/// someone whose shell is bash or fish would miss the lines in their files.
fn login_shell() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
        return PathBuf::from(shell);
    }
    let user = std::env::var("USER").unwrap_or_default();
    let mut dscl = Command::new("/usr/bin/dscl");
    dscl.args([".", "-read", &format!("/Users/{user}"), "UserShell"]);
    bounded_output(dscl, DSCL_TIMEOUT)
        .and_then(|out| account_shell(&out))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}

/// How long `dscl` may take. It asks opendirectoryd, which answers in
/// milliseconds; one that does not is wedged, and nothing else of the app,
/// not even the window, has started yet.
const DSCL_TIMEOUT: Duration = Duration::from_secs(2);

/// A command's stdout, if it finishes within `timeout`; killed if not. Its
/// output is a line or two, so it cannot fill the pipe while it runs.
fn bounded_output(mut cmd: Command, timeout: Duration) -> Option<String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out)
}

/// The shell out of `dscl . -read /Users/<name> UserShell`: `UserShell: /bin/bash`.
fn account_shell(dscl: &str) -> Option<PathBuf> {
    let shell = dscl.lines().find_map(|l| l.strip_prefix("UserShell:"))?.trim();
    shell.starts_with('/').then(|| PathBuf::from(shell))
}

/// Whether `path` is one nobody set: nothing in it but [`LAUNCHD_DIRS`]. An
/// empty PATH counts, since it has even less.
fn is_launchd_default(path: &OsStr) -> bool {
    std::env::split_paths(path).all(|dir| dir.as_os_str().is_empty() || LAUNCHD_DIRS.iter().any(|d| dir == Path::new(d)))
}

/// Ask `shell`, as an interactive login shell, for the PATH it sets up.
///
/// Interactive as well as login: Homebrew's installer writes to `.zprofile`,
/// which a login shell reads, but nvm's and most people's own lines are in
/// `.zshrc`, which only an interactive one does. stdin is empty, so a startup
/// file that asks a question reads end-of-file instead of waiting for an
/// answer.
///
/// The output is read line by line on a thread of its own, and only until
/// the PATH has come: something a startup file starts in the background can
/// keep the pipe open long after the shell is gone, and waiting for the end
/// of it would wait for that. Then, or when `timeout` runs out, the shell's
/// whole process group is killed: whatever a startup file started or was
/// stuck in would otherwise be left running, once for every launch. It has a
/// group of its own for that reason.
fn read(shell: &Path, env: &[(&str, &str)], timeout: Duration) -> Result<String, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(shell);
    let (args, script) = shell_args(shell);
    cmd.args(args)
        .envs(env.iter().copied())
        .process_group(0)
        .stdin(if script.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("could not start {}: {e}", shell.display()))?;
    if let (Some(script), Some(mut stdin)) = (script, child.stdin.take()) {
        // Dropped at the end of this block: the shell reads end-of-file after the
        // script, and exits instead of waiting for more.
        let _ = std::io::Write::write_all(&mut stdin, script.as_bytes());
    }
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut lines = Vec::new();
    let found = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                lines.push(line);
                if let Some(path) = parse(&lines) {
                    break Ok(path);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break Err(format!("{} gave no PATH within {}s", shell.display(), timeout.as_secs())),
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(format!("{} ended without printing a PATH", shell.display())),
        }
    };
    // The whole group, answered or not. Whatever a startup file started is no
    // use to an app that only wanted a PATH, and anything left holding the
    // pipe would keep the reader thread blocked for the life of the app. The
    // shell leads its group, so the group's id is its pid.
    let _ = Command::new("/bin/kill").args(["-KILL", "--", &format!("-{}", child.id())]).stderr(Stdio::null()).status();
    let _ = child.wait();
    found
}

/// How to ask `shell` for its PATH: its arguments, and what to write to its
/// stdin, if anything. The same startup files a terminal's new window reads,
/// so the PATH is the one the user sees there.
///
/// csh and tcsh take `-l` only as their sole option, so they cannot be told
/// a command with `-c` and still be login shells, which read `~/.login`.
/// They get `-l` alone and the command on stdin instead.
fn shell_args(shell: &Path) -> (Vec<String>, Option<String>) {
    let command = awk_command();
    let name = shell.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if matches!(name, "csh" | "tcsh") {
        (vec!["-l".into()], Some(format!("{command}\n")))
    } else {
        (vec!["-l".into(), "-i".into(), "-c".into(), command], None)
    }
}

/// The command that prints the framed PATH.
///
/// awk, not the shell, prints it: one line, framed, in a single write, and
/// the same in zsh, bash, fish or tcsh, whose own quoting differs. Each
/// marker is written as two strings awk joins, so the command's own text
/// never holds a whole one: a startup hook that echoes the command it is
/// about to run (a terminal-title `preexec`) cannot put a frame in the output.
fn awk_command() -> String {
    let split = |marker: &str| {
        let (a, b) = marker.split_at(marker.len() / 2);
        format!("\"{a}\" \"{b}\"")
    };
    format!("/usr/bin/awk 'BEGIN {{ print {} ENVIRON[\"PATH\"] {} }}'", split(START), split(END))
}

/// The PATH out of a login shell's output: what lies between [`START`] and
/// [`END`] on one line. Escape codes may come before it on that line, printed
/// without a newline of their own. Entries may be relative (`./bin`); the
/// framing, not the look of the line, is what says it is the PATH.
fn parse(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|line| {
        let (_, rest) = line.split_once(START)?;
        let (path, _) = rest.split_once(END)?;
        (!path.is_empty() && !path.chars().any(char::is_control)).then(|| path.to_string())
    })
}

/// The login shell's directories first, then this process's that it did not
/// have, each once. Also how many are new to this process. `current` stays
/// an `OsStr`, so a directory in it that is not UTF-8 is kept, not dropped.
fn merge(shell: &str, current: &OsStr) -> (OsString, usize) {
    let before: Vec<PathBuf> = std::env::split_paths(current).collect();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(shell).chain(before.iter().cloned()) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let added = dirs.iter().filter(|d| !before.contains(d)).count();
    // Every entry came out of a split on ':', so none of them holds one and
    // joining cannot fail.
    let path = std::env::join_paths(&dirs).unwrap_or_else(|_| OsString::from(shell));
    (path, added)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `sleep` no other process on the machine is running, so `pgrep` finds
    /// this test's and no other: another test run at the same time has its
    /// own pid, and so its own fraction.
    fn unique_sleep(seconds: u32) -> String {
        format!("sleep {seconds}.{}", std::process::id())
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// A folder to stand in for the home one, holding a `.zshrc`: zsh reads
    /// its startup files from `ZDOTDIR` when it is set.
    fn zdotdir(zshrc: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("divixi-shell-path-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".zshrc"), zshrc).unwrap();
        dir
    }

    #[test]
    fn only_launchds_own_path_is_replaced() {
        let launchd = |p: &str| is_launchd_default(OsStr::new(p));
        assert!(launchd("/usr/bin:/bin:/usr/sbin:/sbin"));
        assert!(launchd("/bin:/usr/bin"));
        assert!(launchd(""));
        assert!(!launchd("/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"));
        assert!(launchd("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"), "what Raycast hands an app it opens");
    }

    #[test]
    fn the_path_is_what_lies_between_the_markers_whatever_came_first() {
        let output = format!("Welcome back!\n\x1b]7;file://mac/Users/me\x07{START}/Users/me/.nvm/versions/node/v24/bin:/opt/homebrew/bin:/usr/bin{END}\n");
        assert_eq!(parse(&lines(&output)).as_deref(), Some("/Users/me/.nvm/versions/node/v24/bin:/opt/homebrew/bin:/usr/bin"));
    }

    #[test]
    fn nothing_is_taken_until_both_markers_have_come() {
        assert_eq!(parse(&lines("/opt/homebrew/bin:/usr/bin\n")), None);
        assert_eq!(parse(&lines(&format!("{START}/opt/homebrew/bin\n"))), None, "no end: not all of it has come");
        assert_eq!(parse(&lines(&format!("{START}{END}\n"))), None);
    }

    #[test]
    fn the_shells_directories_go_first_and_none_twice() {
        let (path, added) = merge("/opt/homebrew/bin:/usr/bin:/bin", OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin"));
        assert_eq!(path, "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin");
        assert_eq!(added, 1);
    }

    #[test]
    fn a_directory_that_is_not_utf8_survives_the_merge() {
        use std::os::unix::ffi::OsStrExt;
        let current = OsStr::from_bytes(b"/usr/bin:/Users/me/\xffbin");
        let (path, _) = merge("/opt/homebrew/bin", current);
        assert_eq!(path.as_bytes(), b"/opt/homebrew/bin:/usr/bin:/Users/me/\xffbin");
    }

    #[test]
    fn the_accounts_shell_is_read_from_dscl() {
        assert_eq!(account_shell("UserShell: /opt/homebrew/bin/fish\n"), Some(PathBuf::from("/opt/homebrew/bin/fish")));
        assert_eq!(account_shell("No such key: UserShell\n"), None);
        assert_eq!(account_shell(""), None);
    }

    /// The whole of it against a real zsh, with a `.zshrc` that greets and
    /// sets a terminal title before it touches the PATH, as real ones do.
    #[test]
    fn a_zshrcs_path_comes_through_its_noise() {
        let home = zdotdir("echo 'Welcome back!'\nprintf '\\033]0;title\\007'\nexport PATH=/opt/divixi-test/bin:$PATH\n");
        let path = read(Path::new("/bin/zsh"), &[("ZDOTDIR", home.to_str().unwrap())], Duration::from_secs(20)).unwrap();
        assert!(path.starts_with("/opt/divixi-test/bin:"), "{path}");
    }

    /// The command, echoed back by a hook before it runs, holds no frame for
    /// the PATH to be read out of; awk's own line does.
    #[test]
    fn an_echoed_command_is_not_taken_for_the_path() {
        let command = awk_command();
        assert!(!command.contains(START) && !command.contains(END), "{command}");
        let output = format!("\x1b]0;{command}\x07\n{START}/opt/homebrew/bin:/usr/bin{END}\n");
        assert_eq!(parse(&lines(&output)).as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
    }

    /// Only the framed line is the PATH: a background job's output, however
    /// much it looks like one, is not taken for it, and a PATH with a
    /// relative entry is taken as it is.
    #[test]
    fn only_the_framed_line_is_the_path() {
        let output = format!("[1] 4242\n/Users/me/project\n{START}/opt/homebrew/bin:./bin:/usr/bin{END}\n/Users/me/other\n");
        assert_eq!(parse(&lines(&output)).as_deref(), Some("/opt/homebrew/bin:./bin:/usr/bin"));
    }

    /// A shell that answers still does not leave behind what its startup file
    /// started: an app that only wanted a PATH has no use for it.
    #[test]
    fn a_shell_that_answers_takes_what_it_started_with_it() {
        let sleep = unique_sleep(4721);
        let home = zdotdir(&format!("{sleep} &\nexport PATH=/opt/divixi-test/bin:$PATH\n"));
        let path = read(Path::new("/bin/zsh"), &[("ZDOTDIR", home.to_str().unwrap())], Duration::from_secs(20)).unwrap();
        assert!(path.starts_with("/opt/divixi-test/bin:"), "{path}");
        std::thread::sleep(Duration::from_millis(200));
        let left = Command::new("/usr/bin/pgrep").args(["-f", &sleep]).output().unwrap();
        assert!(left.stdout.is_empty(), "the startup file's background job outlived the shell");
    }

    #[test]
    fn a_bounded_command_that_hangs_is_killed_and_gives_nothing() {
        let started = Instant::now();
        let mut hang = Command::new("/bin/sleep");
        hang.arg("5");
        assert_eq!(bounded_output(hang, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        let mut echo = Command::new("/bin/echo");
        echo.arg("UserShell: /bin/zsh");
        assert_eq!(bounded_output(echo, Duration::from_secs(2)).as_deref(), Some("UserShell: /bin/zsh\n"));
    }

    #[test]
    fn a_shell_that_never_finishes_starting_is_given_up_on_with_what_it_started() {
        let sleep = unique_sleep(4613);
        let home = zdotdir(&format!("{sleep}\n"));
        let started = Instant::now();
        let err = read(Path::new("/bin/zsh"), &[("ZDOTDIR", home.to_str().unwrap())], Duration::from_millis(500)).unwrap_err();
        assert!(err.contains("gave no PATH"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(200));
        let left = Command::new("/usr/bin/pgrep").args(["-f", &sleep]).output().unwrap();
        assert!(left.stdout.is_empty(), "the startup file's sleep outlived the shell");
    }

    #[test]
    fn tcsh_is_asked_the_way_it_accepts() {
        let home = zdotdir("");
        std::fs::write(home.join(".cshrc"), "echo 'Welcome back!'\nsetenv PATH /opt/divixi-cshrc/bin:${PATH}\n").unwrap();
        std::fs::write(home.join(".login"), "setenv PATH /opt/divixi-login/bin:${PATH}\n").unwrap();
        let path = read(Path::new("/bin/tcsh"), &[("HOME", home.to_str().unwrap())], Duration::from_secs(20)).unwrap();
        assert!(path.contains("/opt/divixi-cshrc/bin"), "{path}");
        assert!(path.contains("/opt/divixi-login/bin"), "~/.login was not read: {path}");
    }
}
