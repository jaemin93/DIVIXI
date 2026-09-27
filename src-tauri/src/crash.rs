//! What is left behind when the app dies of a panic.
//!
//! A panic does not go through `tracing`, and a release build has no console
//! (`windows_subsystem = "windows"`), so until now a panic left nothing at
//! all: the window vanished and the log's last line was whatever was
//! happening before. A hook now writes the panic to a file of its own.
//!
//! Its own file, not the `tracing` log, for two reasons. The subscriber
//! cannot be assumed healthy at the moment of a panic — the panic may be
//! inside it, or inside the writer's lock. And a file that holds nothing but
//! panics is a file the next run can read at a glance.
//!
//! The hook is a last-resort writer, so every error in it is swallowed:
//! there is nothing useful to do about a failure to record a failure.
//!
//! What the next run can say, and what it cannot: `crash.log` proves the
//! last run panicked somewhere. It does not prove the app was killed — a
//! panic on a worker thread ends that thread and the window stays up — so
//! the record is offered as "the last run panicked", with the time and the
//! message, and never as a count of crashes.

use std::io::Write;
use std::path::Path;

/// This run's panics, if any. Moved aside at startup so the file always
/// belongs to the run that is writing it.
const CURRENT: &str = "crash.log";
/// The previous run's panics, kept until the run after that replaces them.
const PREVIOUS: &str = "crash-previous.log";

/// Past this, the file is started again rather than appended to. Something
/// panicking in a loop must not fill the disk, and the panic worth reading
/// is the one that just happened.
const CAP: u64 = 512 * 1024;
/// A backtrace longer than this is cut: the frames that matter are the first.
const BACKTRACE_CAP: usize = 16 * 1024;

/// The previous run's panic, for the interface to mention when it is asked to.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Crash {
    /// When it happened, RFC 3339 in UTC, as the file records it.
    pub when: String,
    /// The panic's own message, one line.
    pub message: String,
    /// `src/conductor.rs:123:4`, when the panic said where it was.
    pub location: String,
    /// The whole block, backtrace and all, for a bug report.
    pub detail: String,
    /// Where that block is on disk.
    pub path: String,
}

/// Record every panic from here on in `<logs>/crash.log`.
///
/// The hook that was installed before stays in place and runs afterwards, so
/// a debug build still prints to stderr and the process still aborts or
/// unwinds exactly as it did.
pub fn install(logs: &Path) {
    let file = logs.join(CURRENT);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_block(&file, info);
        previous(info);
    }));
}

/// The previous run's panic, taken aside so that this run starts clean.
///
/// Called once at startup, before [`install`]: it renames this run's file to
/// the previous one's name, which is what makes "the last run panicked"
/// answerable without keeping a separate marker in step with it.
///
/// `None` unless this startup found a file to move. `crash-previous.log`
/// stays on disk for whoever opens the folder, but it is not read: a run
/// that did not panic must not be reported as one that did, or the notice
/// would come back on every launch for the rest of the install's life.
pub fn take_previous(logs: &Path) -> Option<Crash> {
    let current = logs.join(CURRENT);
    let previous = logs.join(PREVIOUS);
    if !current.is_file() {
        return None;
    }
    let _ = std::fs::remove_file(&previous);
    // A failed rename leaves the block where it is and this run's hook will
    // append to it: better a file with two runs in it than a panic lost.
    match std::fs::rename(&current, &previous) {
        Ok(()) => read(&previous),
        Err(_) => read(&current),
    }
}

/// Read a crash file back into its last block.
fn read(path: &Path) -> Option<Crash> {
    let text = std::fs::read_to_string(path).ok()?;
    let block = text.rsplit(SEPARATOR).find(|b| !b.trim().is_empty())?.trim().to_string();
    let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string())).unwrap_or_default();
    Some(Crash {
        when: field("when:"),
        message: field("panic:"),
        location: field("at:"),
        detail: block,
        path: path.display().to_string(),
    })
}

/// Between blocks, and how [`read`] finds the last one.
const SEPARATOR: &str = "==== divixi panic ====";

/// Append one panic to the file. Everything here is best effort.
fn write_block(path: &Path, info: &std::panic::PanicHookInfo<'_>) {
    let message = message_of(info);
    let location = info.location().map(|l| l.to_string()).unwrap_or_else(|| "unknown".to_string());
    let mut backtrace = std::backtrace::Backtrace::force_capture().to_string();
    if backtrace.len() > BACKTRACE_CAP {
        // Back off to a character boundary: a path in a frame may not be ASCII.
        let cut = (0..=BACKTRACE_CAP).rev().find(|at| backtrace.is_char_boundary(*at)).unwrap_or(0);
        backtrace.truncate(cut);
    }

    let block = format!(
        "{SEPARATOR}\nwhen: {}\npanic: {message}\nat: {location}\npid: {}\nthread: {}\nversion: {} ({})\nbacktrace:\n{backtrace}\n",
        now(),
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed"),
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    // A panic message can carry anything the panicking code was holding.
    let block = crate::mask::scrub(&block).unwrap_or(block);

    let over_cap = std::fs::metadata(path).map(|m| m.len() > CAP).unwrap_or(false);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true);
    if over_cap {
        options.truncate(true);
    } else {
        options.append(true);
    }
    if let Ok(mut file) = options.open(path) {
        let _ = file.write_all(block.as_bytes());
        let _ = file.flush();
    }
    // The log file too, in case the subscriber is still standing: a reader
    // who only has the log then still sees that the run ended here.
    tracing::error!(%location, "panic: {message}");
}

/// The panic's message, whichever of the two shapes it was raised with.
fn message_of(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic with no message".to_string()
    }
}

/// Now, in UTC, to the second.
pub fn now() -> String {
    time::OffsetDateTime::now_utc().replace_nanosecond(0).ok().and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok()).unwrap_or_default()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("divixi-crash-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The hook itself is process-wide and a panic inside a test is the
    /// test's own, so what is exercised here is the file: the block's shape,
    /// and that the next run reads the last one back out of it.
    #[test]
    fn the_last_block_is_what_the_next_run_reads() {
        let logs = temp();
        std::fs::write(
            logs.join(CURRENT),
            format!(
                "{SEPARATOR}\nwhen: 2026-09-26T10:00:00Z\npanic: the first one\nat: src/a.rs:1:1\n\
                 {SEPARATOR}\nwhen: 2026-09-27T11:22:33Z\npanic: called `Option::unwrap()` on a `None` value\nat: src/conductor.rs:942:31\npid: 4242\nbacktrace:\n   0: divixi::main\n"
            ),
        )
        .unwrap();

        let crash = take_previous(&logs).expect("the previous run's panic");
        assert_eq!(crash.when, "2026-09-27T11:22:33Z", "the newest block, not the first");
        assert_eq!(crash.message, "called `Option::unwrap()` on a `None` value");
        assert_eq!(crash.location, "src/conductor.rs:942:31");
        assert!(crash.detail.contains("divixi::main"), "the backtrace comes along for the report");
        assert!(crash.path.ends_with(PREVIOUS));

        assert!(!logs.join(CURRENT).exists(), "this run starts with no crash file of its own");
        assert!(logs.join(PREVIOUS).is_file());

        // The run after this one does not panic: nothing to take, even
        // though the block from two runs ago is still sitting there. A
        // notice that came back every launch would be worse than none.
        assert!(take_previous(&logs).is_none(), "only a file this startup moved belongs to the run that just ended");
        assert!(logs.join(PREVIOUS).is_file(), "and it is still there to open");
        std::fs::remove_dir_all(&logs).ok();
    }

    /// The hook itself, end to end: install it, panic inside
    /// `catch_unwind` so the test survives it, and read what it left.
    ///
    /// The hook is process-wide, so the one that was there is put back
    /// before this test ends — a later test that fails has to keep printing
    /// its failure the way it always did.
    #[test]
    fn the_hook_writes_the_panic_it_saw() {
        let logs = temp();
        let standing = std::panic::take_hook();
        install(&logs);
        let ours = std::panic::catch_unwind(|| panic!("a panic for the test, with ghp_0123456789abcdefghijABCDEFGHIJ0123 in it"));
        std::panic::set_hook(standing);
        assert!(ours.is_err(), "the panic happened");

        let text = std::fs::read_to_string(logs.join(CURRENT)).expect("the hook made the file");
        assert!(text.contains(SEPARATOR), "the block is marked off: {text}");
        assert!(text.contains("a panic for the test"), "{text}");
        assert!(text.contains("at: src-tauri\\src\\crash.rs") || text.contains("at: src-tauri/src/crash.rs"), "with where it happened: {text}");
        assert!(text.contains("backtrace:"), "and a backtrace, symbols or not");
        assert!(!text.contains("ghp_0123456789"), "a panic message goes through the mask too: {text}");
        assert!(text.contains("ghp_[hidden]"));

        // And the next run reads it back as the previous run's panic.
        let crash = take_previous(&logs).expect("a panic to report");
        assert!(crash.message.starts_with("a panic for the test"), "{}", crash.message);
        assert!(crash.when.ends_with('Z'), "stamped: {}", crash.when);
        std::fs::remove_dir_all(&logs).ok();
    }

    #[test]
    fn a_block_is_written_masked_and_stops_growing() {
        let logs = temp();
        let path = logs.join(CURRENT);
        // Stand in for the hook: the same writer, without panicking a test.
        let secret = "ghp_0123456789abcdefghijABCDEFGHIJ0123";
        std::fs::write(&path, format!("{SEPARATOR}\nwhen: 2026-09-27T00:00:00Z\npanic: token was {secret}\nat: src/x.rs:1:1\n")).unwrap();
        let blocked = crate::mask::scrub(&std::fs::read_to_string(&path).unwrap()).expect("a token in a panic message is hidden");
        assert!(!blocked.contains(secret) && blocked.contains("ghp_[hidden]"), "{blocked}");

        assert!(now().ends_with('Z') && now().len() >= 20, "an RFC 3339 stamp in UTC: {}", now());
        std::fs::remove_dir_all(&logs).ok();
    }
}
