//! Where the log goes, and how much of it.
//!
//! A release build has no console (`windows_subsystem = "windows"`), so a
//! subscriber writing to stdout threw the whole log away on an installed
//! app: every `warn!` and `error!` in the workspace went nowhere and a bug
//! report had nothing to attach. The log now also goes to a file under the
//! app's data folder — a new one each day, the last [`KEEP_FILES`] kept —
//! and stdout stays on in development so `tauri dev` reads as before.
//!
//! Writes go straight to the file rather than through
//! `tracing_appender::non_blocking`: a panic or a hard exit then still
//! leaves the last lines on disk, which are the ones a crash report needs.
//!
//! # One gate for the level
//!
//! The filter is wrapped in a [`reload`] layer and is the **only** thing in
//! the stack that decides whether an event is recorded — neither the file
//! layer nor the stdout layer carries a level of its own. That is what makes
//! [`set_level`] enough: one change reaches every sink at once, and there is
//! no second copy of the level to drift out of step. A sink added later must
//! stay level-free for the same reason (and go through [`Masked`], the
//! writer that keeps secrets out of every sink).
//!
//! The level comes from the environment at startup and from the settings
//! once the store is open ([`apply_saved`]), so a user reproducing a bug can
//! turn the detail up in the settings window without restarting into the
//! state they were trying to keep.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use orchestra_store::Store;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::mask::Masked;

/// Sets the log level for a run, as `RUST_LOG` does (this one wins). Takes
/// tracing's filter syntax: `info`, `debug`, `warn,orchestra_acp=debug`, …
///
/// It is the value the app *starts* at. A level chosen in the settings is
/// kept in the store and takes over from it.
pub const LEVEL_ENV: &str = "DIVIXI_LOG";

/// The setting the chosen level is kept under (`setting:` + this).
pub const LEVEL_SETTING: &str = "log.level";

/// Quiet by default: warnings from everything, and the few lines the app,
/// its agent sessions and its MCP server log about what they are doing.
///
/// A plain `debug` also records what the agents print on stderr (the
/// `agent_stderr` target), which is theirs and may hold anything — which is
/// the other reason the default stays here.
pub const DEFAULT_LEVEL: &str = "warn,orchestra_app=info,orchestra_acp=info,orchestra_mcp=info";

/// The levels the settings page offers, as `(id, filter)`.
///
/// The ids are what the interface names its buttons after; the filters are
/// this module's business. Anything else that parses is still accepted by
/// [`set_level`], so `DIVIXI_LOG`-shaped filters keep working.
pub const PRESETS: [(&str, &str); 4] = [("quiet", DEFAULT_LEVEL), ("info", "info"), ("debug", "debug"), ("trace", "trace")];

/// Log files kept. Older ones go as each new day's file is opened.
const KEEP_FILES: usize = 7;

/// `divixi.2026-09-27.log`
const PREFIX: &str = "divixi";
const SUFFIX: &str = "log";

/// The folder the log files live in.
///
/// Under the app's data folder, beside the store and the adapters, so it is
/// the platform's own place on every OS (Tauri resolves the data folder):
///
/// - Windows: `%APPDATA%\app.divixi\logs`
/// - macOS: `~/Library/Application Support/app.divixi/logs`
/// - Linux: `~/.local/share/app.divixi/logs`
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// The handle that changes the level, and where the level in force came from.
struct Control {
    filter: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
    source: parking_lot::Mutex<&'static str>,
}

static CONTROL: OnceLock<Control> = OnceLock::new();

/// The level in force, for the settings page and the diagnostics report.
#[derive(Clone, serde::Serialize)]
pub struct Level {
    /// The filter itself, in tracing's syntax.
    pub filter: String,
    /// Which of [`PRESETS`] it is, or `custom`.
    pub preset: String,
    /// `env`, `setting` or `default`: what put it there.
    pub source: String,
}

/// Start logging: to the file under `data_dir`, and to stdout as well in
/// development. Returns the folder the files are in.
///
/// Called once, from the app's `setup` — the first point at which Tauri can
/// say where this platform keeps an app's data. Failing to open the file is
/// not fatal: the app runs with whatever layers it could install.
pub fn start(data_dir: &Path) -> PathBuf {
    let logs = dir(data_dir);
    // Made here rather than by the appender, which looks for files to prune
    // before it creates anything and grumbles to stderr about the folder.
    let _ = std::fs::create_dir_all(&logs);
    let file = match appender(&logs) {
        Ok(appender) => Some(appender),
        Err(err) => {
            eprintln!("divixi: no log file in {}: {err}", logs.display());
            None
        }
    };
    let (initial, source) = from_env();
    let (filter, handle) = reload::Layer::new(initial);
    let registry = tracing_subscriber::registry()
        .with(filter)
        // Timestamps and levels only; a file has no terminal to colour.
        .with(file.map(|f| tracing_subscriber::fmt::layer().with_ansi(false).with_writer(Masked(f))))
        .with(to_stdout().then(|| tracing_subscriber::fmt::layer().with_writer(Masked(std::io::stdout))));
    // Only one subscriber can be global. A second call (a test, a second
    // window on the mock runtime) leaves the first one in place.
    if registry.try_init().is_ok() {
        let _ = CONTROL.set(Control { filter: handle, source: parking_lot::Mutex::new(source) });
    } else {
        tracing::debug!("logging was already started");
    }
    logs
}

/// Whether the log also goes to stdout.
///
/// In development, so `tauri dev` reads as it always has. And in
/// `divixi-server`, whose console is the point: it is started from a shell
/// or under a service manager that keeps what it prints. The installed
/// desktop app has no console at all and only writes the file.
fn to_stdout() -> bool {
    cfg!(debug_assertions) || cfg!(feature = "server")
}

/// The level from the environment, or [`DEFAULT_LEVEL`].
fn from_env() -> (EnvFilter, &'static str) {
    match EnvFilter::try_from_env(LEVEL_ENV).or_else(|_| EnvFilter::try_from_default_env()) {
        Ok(filter) => (filter, "env"),
        Err(_) => (EnvFilter::new(DEFAULT_LEVEL), "default"),
    }
}

/// The level in force now.
pub fn level() -> Level {
    let Some(control) = CONTROL.get() else {
        return Level { filter: DEFAULT_LEVEL.to_string(), preset: preset_of(DEFAULT_LEVEL).to_string(), source: "default".to_string() };
    };
    // The filter prints itself as the directives it was built from.
    let filter = control.filter.with_current(|f| f.to_string()).unwrap_or_else(|_| DEFAULT_LEVEL.to_string());
    Level { preset: preset_of(&filter).to_string(), source: control.source.lock().to_string(), filter }
}

/// Change the level for this run and remember it for the next one.
///
/// The only place the level changes. `id` is one of [`PRESETS`] or a filter
/// in tracing's own syntax; anything that does not parse is refused rather
/// than silently ignored, so a typo in the settings cannot quietly turn the
/// log off.
///
/// Only the one setting is written. The store keeps settings a key at a
/// time, so a level chosen here cannot roll back an unrelated preference
/// saved a moment earlier in another window.
pub fn set_level(store: &Store, id: &str) -> Result<Level, String> {
    let wanted = PRESETS.iter().find(|(name, _)| *name == id).map(|(_, filter)| *filter).unwrap_or(id);
    let filter = EnvFilter::try_new(wanted).map_err(|e| format!("{wanted}: {e}"))?;
    let control = CONTROL.get().ok_or("logging has not started")?;
    // Rebuilding the interest cache is the reload layer's own doing, which
    // is what lets callsites already passed over start being recorded.
    control.filter.reload(filter).map_err(|e| e.to_string())?;
    *control.source.lock() = "setting";
    store.set_meta(&format!("{}{LEVEL_SETTING}", crate::SETTING_PREFIX), wanted).map_err(|e| e.to_string())?;
    let level = level();
    tracing::info!(filter = %level.filter, "log level set from the settings");
    Ok(level)
}

/// Put the level from the settings in force, if one was ever chosen there.
///
/// Called once the store is open, which is after [`start`]: the handful of
/// lines logged in between are at the level the environment asked for.
pub fn apply_saved(store: &Store) {
    let Ok(Some(saved)) = store.get_meta(&format!("{}{LEVEL_SETTING}", crate::SETTING_PREFIX)) else { return };
    let saved = saved.trim();
    let Some(control) = CONTROL.get() else { return };
    if saved.is_empty() || normalized(saved) == Some(level().filter) {
        return;
    }
    match EnvFilter::try_new(saved) {
        Ok(filter) => {
            if control.filter.reload(filter).is_ok() {
                *control.source.lock() = "setting";
                tracing::info!(filter = saved, "log level from the settings");
            }
        }
        // A filter the store cannot parse is not worth refusing to start over.
        Err(err) => tracing::warn!(%err, filter = saved, "the saved log level is not a filter; keeping the current one"),
    }
}

/// A filter as the filter itself prints it, which is the only way two of
/// them compare: `EnvFilter` reorders and rewrites what it was given
/// (`warn,orchestra_app=info` comes back out in its own spelling).
fn normalized(filter: &str) -> Option<String> {
    EnvFilter::try_new(filter).ok().map(|f| f.to_string())
}

/// Which preset a filter is, for the settings page's buttons.
fn preset_of(filter: &str) -> &'static str {
    PRESETS
        .iter()
        .find(|(_, f)| *f == filter || normalized(f).as_deref() == Some(filter))
        .map(|(id, _)| *id)
        .unwrap_or("custom")
}

/// The first lines of a run, so a log fragment says which build wrote it.
///
/// Rotation is by day, so several runs share one file; the `====` line is
/// where one run ends and the next begins. A day that turns over while the
/// app is running starts a file without a banner — the run is still named by
/// the one in the file before it.
pub fn banner(logs: &Path) {
    let env = crate::diagnostics::environment();
    let level = level();
    tracing::info!("==== divixi {} ({}) · pid {} · session start ====", env.version, env.build, std::process::id());
    tracing::info!(
        os = %env.os,
        kernel = %env.kernel,
        arch = %env.arch,
        webview = %env.webview,
        level = %level.filter,
        from = %level.source,
        log = %current_file(logs).unwrap_or_else(|| logs.to_path_buf()).display(),
        "this run"
    );
}

/// Today's log file, rotating at midnight, keeping [`KEEP_FILES`] days.
fn appender(logs: &Path) -> Result<tracing_appender::rolling::RollingFileAppender, String> {
    tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(PREFIX)
        .filename_suffix(SUFFIX)
        .max_log_files(KEEP_FILES)
        .build(logs)
        .map_err(|e| e.to_string())
}

/// The log files in `logs`, newest first.
///
/// The name carries the date (`divixi.2026-09-27.log`), so sorting the names
/// sorts by day without asking the filesystem for times.
pub fn files(logs: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(logs) else { return Vec::new() };
    let mut named: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
        .collect();
    named.sort_by(|a, b| b.0.cmp(&a.0));
    named.into_iter().map(|(_, path)| path).collect()
}

/// The file being written to now, if there is one.
pub fn current_file(logs: &Path) -> Option<PathBuf> {
    files(logs).into_iter().next()
}

/// How many log lines a diagnostics report carries.
pub const RECENT_ERRORS: usize = 20;

/// The last warnings and errors logged, oldest first, for a bug report.
///
/// Read from the newest files backwards until there are `limit` of them, so
/// a report made the morning after a crash still shows it.
pub fn recent_errors(logs: &Path, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in files(logs) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let mut found: Vec<String> = text.lines().filter(|l| is_problem(l)).map(|l| l.trim_end().to_string()).collect();
        // This file's last lines come before the ones already found.
        let keep = limit.saturating_sub(out.len());
        found.reverse();
        found.truncate(keep);
        found.reverse();
        found.append(&mut out);
        out = found;
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// A log line worth putting in a bug report: the level is `WARN` or `ERROR`.
///
/// tracing's own format pads the level to five characters and puts a space
/// on either side, so this does not match the words in a message.
fn is_problem(line: &str) -> bool {
    line.contains(" WARN ") || line.contains(" ERROR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("divixi-logging-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_appender_writes_a_dated_file_under_the_data_folder() {
        use std::io::Write;
        let data = temp();
        let logs = dir(&data);
        assert_eq!(logs, data.join("logs"), "logs live beside the store, not in the current folder");

        let mut appender = appender(&logs).expect("an appender in a folder that did not exist yet");
        writeln!(appender, "hello").unwrap();
        appender.flush().unwrap();

        let files = files(&logs);
        assert_eq!(files.len(), 1, "one file for today");
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("divixi.") && name.ends_with(".log"), "{name} carries the date between the two");
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "hello\n");
        assert_eq!(current_file(&logs), Some(files[0].clone()));
        std::fs::remove_dir_all(&data).ok();
    }

    /// The whole path, without an app: start logging into a folder of its
    /// own, log a warning, read it back off the disk, then turn the level up
    /// and see a line that was being dropped arrive.
    ///
    /// One test, because there is one of everything it touches: one global
    /// subscriber (a second [`start`] installs nothing), and one level for
    /// the whole process. Split in two, the halves raced each other over the
    /// level. Other tests logging at the same time add lines of their own,
    /// which is why this looks for its own and not at the whole file, and
    /// the folder is left for the OS to clear: they may still be writing
    /// into it when this one ends.
    #[test]
    fn the_file_is_written_and_the_level_changes_without_a_restart() {
        let data = temp();
        let logs = start(&data);
        assert_eq!(logs, dir(&data));

        // Whatever `DIVIXI_LOG` says in the shell the tests were started
        // from, this test is about the default.
        let control = CONTROL.get().expect("logging started, so the level can be changed");
        control.filter.reload(EnvFilter::new(DEFAULT_LEVEL)).unwrap();
        assert_eq!(level().preset, "quiet", "the filter reads back as the preset it was set to");

        tracing::warn!(marker = "divixi-logging-test", "a warning for the log file");
        tracing::debug!("quiet by default: this line is not recorded");

        let file = current_file(&logs).expect("today's file, made as logging started");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("a warning for the log file"), "the message is in {}: {text}", file.display());
        assert!(text.contains("divixi-logging-test"), "with its fields");
        assert!(text.contains("WARN"), "and its level");
        assert!(!text.contains("this line is not recorded"), "the default level keeps debug out");

        // And it is what a bug report would carry.
        let errors = recent_errors(&logs, RECENT_ERRORS);
        assert!(errors.iter().any(|l| l.contains("a warning for the log file")), "{errors:?}");

        // Turn it up. No store here, so the reload handle is used directly —
        // `set_level` is the same call with the setting written as well.
        control.filter.reload(EnvFilter::new("debug")).unwrap();
        tracing::debug!("turned up: this line is recorded");
        assert_eq!(level().preset, "debug");

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("turned up: this line is recorded"), "a callsite passed over before is recorded now: {text}");

        // ----- and the same change, made the way the settings page makes it -----
        //
        // `set_level` writes the setting as well, so this is also the check
        // that it and `apply_saved` agree about the key and about what a
        // preset means.
        let store = Store::in_memory().unwrap();
        let chosen = set_level(&store, "info").expect("a preset is accepted");
        assert_eq!(chosen.preset, "info");
        assert_eq!(chosen.source, "setting", "it was the settings that put it there");
        assert_eq!(
            store.get_meta(&format!("{}{LEVEL_SETTING}", crate::SETTING_PREFIX)).unwrap().as_deref(),
            Some("info"),
            "the filter is what is kept, not the preset's name"
        );

        // A filter of one's own is allowed; a level that is not a level is
        // refused rather than quietly turning the log off.
        assert!(set_level(&store, "warn,orchestra_app=trace").is_ok());
        assert_eq!(level().preset, "custom");
        assert!(set_level(&store, "orchestra_app=louder").is_err());
        assert_eq!(level().preset, "custom", "a refusal changes nothing");

        // What the next run does with the store it finds.
        set_level(&store, "quiet").unwrap();
        apply_saved(&store);
        assert_eq!(level().preset, "quiet");
        assert_eq!(preset_of(DEFAULT_LEVEL), "quiet");
        assert_eq!(preset_of("warn,orchestra_app=trace"), "custom");

        // Left quiet, so the tests that run alongside are not flooded: this
        // is one process and one level.
        assert_eq!(level().filter, normalized(DEFAULT_LEVEL).unwrap());
    }

    #[test]
    fn recent_errors_read_the_newest_days_last_problems() {
        let logs = temp();
        std::fs::write(
            logs.join("divixi.2026-09-26.log"),
            "2026-09-26T00:00:00Z  WARN a: yesterday warned\n2026-09-26T00:00:01Z  INFO a: yesterday was fine\n",
        )
        .unwrap();
        std::fs::write(
            logs.join("divixi.2026-09-27.log"),
            "2026-09-27T00:00:00Z  INFO a: opening event store\n2026-09-27T00:00:01Z  WARN a: could not move the drafts folder\n2026-09-27T00:00:02Z ERROR a: it broke\n",
        )
        .unwrap();
        // Not ours: a file the user dropped in the folder is left alone.
        std::fs::write(logs.join("notes.txt"), "ERROR not a log of ours\n").unwrap();

        let all = recent_errors(&logs, 10);
        assert_eq!(
            all,
            vec![
                "2026-09-26T00:00:00Z  WARN a: yesterday warned".to_string(),
                "2026-09-27T00:00:01Z  WARN a: could not move the drafts folder".to_string(),
                "2026-09-27T00:00:02Z ERROR a: it broke".to_string(),
            ],
            "warnings and errors, oldest first, info lines left out"
        );
        // A tight limit keeps the newest, and stops before older days.
        assert_eq!(recent_errors(&logs, 1), vec!["2026-09-27T00:00:02Z ERROR a: it broke".to_string()]);
        std::fs::remove_dir_all(&logs).ok();
    }
}
