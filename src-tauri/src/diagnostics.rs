//! What a bug report needs attached.
//!
//! One command builds one block of Markdown the reporter can paste straight
//! into an issue: the version, the OS, the agent CLIs that are installed and
//! which version of each, where the log is, and the last warnings and errors
//! from it. The settings page shows the same text before it is copied, so
//! nobody pastes something they have not read.
//!
//! What it must never carry: a key, a token, or anything else secret. So it
//! reads no settings (the embedding key lives there) and no environment, and
//! it prints an agent's program and version but never the environment a
//! session is launched with. Personal paths do go in — a report without them
//! cannot be followed — which is the other reason the text is shown first.
//!
//! The report itself is English: it is read by whoever answers the issue,
//! not by the person filing it. The buttons and notes around it go through
//! i18n like the rest of the interface.

use orchestra_agents::{Adapter, AgentStatus};
use tauri::Manager;

use crate::{logging, AppHandle, AppState};

/// What machine this is. Shared with the log's boot banner, so a log
/// fragment and a report name the same build the same way.
pub struct Environment {
    pub version: String,
    pub build: &'static str,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub webview: String,
}

/// This machine, as far as it can be asked without an app.
pub fn environment() -> Environment {
    Environment {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: if cfg!(debug_assertions) { "debug" } else { "release" },
        os: sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string()),
        kernel: sysinfo::System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
        arch: sysinfo::System::cpu_arch(),
        webview: webview(),
    }
}

/// Everything the report says, gathered before any of it is written out.
///
/// A plain struct with no app in it, so the writing can be tested.
struct Facts {
    machine: Environment,
    /// The filter the log is being written at, and what put it there.
    level: String,
    workspace: String,
    data_dir: String,
    store: String,
    log_file: String,
    agents: Vec<Agent>,
    errors: Vec<String>,
    /// The previous run's panic, when it left one behind.
    crash: Option<crate::crash::Crash>,
}

/// One agent CLI as this machine has it.
struct Agent {
    name: String,
    readiness: String,
    /// Where the CLI is, and what `--version` said, when it was found.
    cli: Option<(String, String)>,
    adapter: String,
}

/// The report for this install, as Markdown.
pub fn report(app: &AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let logs = logging::dir(&state.data_dir);
    let level = logging::level();
    let facts = Facts {
        machine: environment(),
        level: format!("{} (from the {})", level.filter, level.source),
        workspace: crate::workspace_root().display().to_string(),
        data_dir: state.data_dir.display().to_string(),
        store: state.db_path.clone(),
        log_file: logging::current_file(&logs).unwrap_or_else(|| logs.clone()).display().to_string(),
        agents: state.load_agents()?.unwrap_or_default().iter().map(agent).collect(),
        errors: logging::recent_errors(&logs, logging::RECENT_ERRORS),
        crash: state.last_crash.clone(),
    };
    Ok(render(&facts))
}

/// Which webview the window runs on: the most useful single fact about a
/// rendering bug, and the one nobody can read off their own machine.
fn webview() -> String {
    tauri::webview_version().unwrap_or_else(|_| "unknown".to_string())
}

/// One detected agent, without the environment its sessions are launched
/// with ([`orchestra_acp::AgentSpec::env`]) — that is where a key would be.
fn agent(status: &AgentStatus) -> Agent {
    Agent {
        name: status.name.clone(),
        readiness: format!("{:?}", status.readiness),
        cli: status.cli.as_ref().map(|c| (c.path.clone(), c.version.clone().unwrap_or_else(|| "version unknown".to_string()))),
        adapter: match &status.adapter {
            Adapter::LocalScript { path } => format!("local adapter {path}"),
            Adapter::Npx { package } => format!("npx {package}"),
            Adapter::Cli { path } => format!("the CLI speaks ACP: {path}"),
            Adapter::Binary { path } => format!("server binary {path}"),
            Adapter::NeedsDownload { .. } => "server binary not downloaded yet".to_string(),
            Adapter::Missing { reason } => format!("none: {reason}"),
        },
    }
}

/// The facts as a fenced block, ready to paste into an issue.
fn render(f: &Facts) -> String {
    let mut out = String::from("### divixi diagnostics\n\n```text\n");
    let mut row = |k: &str, v: &str| out.push_str(&format!("{k:<10}{v}\n"));
    row("divixi", &format!("{} ({})", f.machine.version, f.machine.build));
    row("os", &format!("{} · {} · kernel {}", f.machine.os, std::env::consts::OS, f.machine.kernel));
    row("arch", &f.machine.arch);
    row("webview", &f.machine.webview);
    row("workspace", &f.workspace);
    row("data", &f.data_dir);
    row("store", &f.store);
    row("log", &f.log_file);
    row("level", &f.level);

    out.push_str("\nagents\n");
    if f.agents.is_empty() {
        out.push_str("  none detected yet\n");
    }
    for a in &f.agents {
        out.push_str(&format!("  {} — {}\n", a.name, a.readiness));
        match &a.cli {
            Some((path, version)) => out.push_str(&format!("    cli      {path} ({version})\n")),
            None => out.push_str("    cli      not found\n"),
        }
        out.push_str(&format!("    adapter  {}\n", a.adapter));
    }

    if let Some(crash) = &f.crash {
        // The previous run panicked: that block is the report.
        out.push_str(&format!("\nthe run before this one panicked\n  when     {}\n  panic    {}\n  at       {}\n", crash.when, crash.message, crash.location));
    }

    out.push_str(&format!("\nlast warnings and errors ({})\n", f.errors.len()));
    if f.errors.is_empty() {
        out.push_str("  none in the log\n");
    }
    for line in &f.errors {
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str("```\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            machine: Environment {
                version: "0.1.0".into(),
                build: "release",
                os: "Windows 11 Home".into(),
                kernel: "26200".into(),
                arch: "x86_64".into(),
                webview: "141.0.3537.85".into(),
            },
            level: "warn,orchestra_app=info (from the default)".into(),
            workspace: r"C:\Users\someone\projects".into(),
            data_dir: r"C:\Users\someone\AppData\Roaming\app.divixi".into(),
            store: r"C:\Users\someone\AppData\Roaming\app.divixi\divixi.db".into(),
            log_file: r"C:\Users\someone\AppData\Roaming\app.divixi\logs\divixi.2026-09-27.log".into(),
            agents: vec![
                Agent {
                    name: "Claude Code".into(),
                    readiness: "Ready".into(),
                    cli: Some((r"C:\Program Files\nodejs\claude.cmd".into(), "2.1.34".into())),
                    adapter: "local adapter node_modules/x/dist/index.js".into(),
                },
                Agent { name: "Codex".into(), readiness: "NotInstalled".into(), cli: None, adapter: "none: no CLI".into() },
            ],
            errors: vec!["2026-09-27T09:00:00Z  WARN orchestra_app: could not move the drafts folder".into()],
            crash: None,
        }
    }

    #[test]
    fn the_report_pastes_into_an_issue() {
        let md = render(&facts());
        assert!(md.starts_with("### divixi diagnostics\n\n```text\n"));
        assert!(md.ends_with("```\n"), "the fence is closed, so the block is one paste");
        assert_eq!(md.matches("```").count(), 2, "no fence inside the fence");
        for expected in [
            "0.1.0 (release)",
            "Windows 11 Home",
            "x86_64",
            "141.0.3537.85",
            "divixi.2026-09-27.log",
            "Claude Code — Ready",
            r"claude.cmd (2.1.34)",
            "Codex — NotInstalled",
            "not found",
            "could not move the drafts folder",
            "warn,orchestra_app=info (from the default)",
        ] {
            assert!(md.contains(expected), "the report says {expected:?}:\n{md}");
        }
    }

    #[test]
    fn an_empty_install_still_reports() {
        let md = render(&Facts { agents: Vec::new(), errors: Vec::new(), ..facts() });
        assert!(md.contains("none detected yet"));
        assert!(md.contains("none in the log"));
    }

    /// A panic in the run before is the most useful thing a report can
    /// carry, and the reader has to see it without opening a file.
    #[test]
    fn the_previous_runs_panic_is_in_the_report() {
        let crash = crate::crash::Crash {
            when: "2026-09-27T11:22:33Z".into(),
            message: "called `Option::unwrap()` on a `None` value".into(),
            location: "src/conductor.rs:942:31".into(),
            detail: "==== divixi panic ====\nbacktrace:\n   0: divixi::main".into(),
            path: r"C:\Users\someone\AppData\Roaming\app.divixi\logs\crash-previous.log".into(),
        };
        let md = render(&Facts { crash: Some(crash), ..facts() });
        assert!(md.contains("the run before this one panicked"), "{md}");
        assert!(md.contains("src/conductor.rs:942:31") && md.contains("2026-09-27T11:22:33Z"), "{md}");
        assert_eq!(md.matches("```").count(), 2, "still one block to paste");
        assert!(!render(&facts()).contains("panicked"), "a run that did not panic says nothing about panics");
    }
}
