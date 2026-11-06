//! Knowing that an agent's login is gone.
//!
//! Detection cannot tell: it looks once, and a probe only opens a session,
//! which Claude's adapter does logged out or not. A login that runs out shows
//! as every turn failing within a second, on a machine nobody is watching,
//! with the agent still marked ready. So the first turn refused for want of a
//! login marks the agent and rings the bell once, with the command that brings
//! it back; the login coming back takes both away.
//!
//! The mark does not stop anything from starting. Nothing tells the app when
//! someone logs in from a terminal, and the next turn should simply work.

use std::time::Duration;

use orchestra_agents::AgentKind;
use orchestra_core::AgentEvent;
use serde_json::json;
use tauri::{Emitter, Manager};

use crate::{AppHandle, AppState};

/// How often a marked agent is asked about its login, and how often stray
/// notices are looked for. A minute: someone has just logged in and is
/// looking at the bell.
const TICK: Duration = Duration::from_secs(60);

/// The bell's kind for this agent's lost login.
fn lost_kind(kind: AgentKind) -> String {
    format!("login_lost:{}", kind.id())
}

/// A turn ended: what that says about its agent's login.
pub fn turn_ended(app: &AppHandle, run: &str, event: &AgentEvent) {
    match event {
        AgentEvent::Failed { error } if orchestra_acp::wants_login(error) => {
            let (app, run) = (app.clone(), run.to_string());
            tauri::async_runtime::spawn(async move { refused(&app, &run).await });
        }
        // A turn the agent finished went through on its login. A cancel may
        // not have got that far.
        AgentEvent::Finished { stop_reason } if !stop_reason.eq_ignore_ascii_case("cancelled") => {
            let any_lost = app.state::<AppState>().load_agents().ok().flatten().is_some_and(|list| list.iter().any(|s| s.login_lost));
            if any_lost {
                if let Some(kind) = kind_of_run(app, run) {
                    set_lost(app, kind, false);
                }
            }
        }
        _ => {}
    }
}

fn kind_of_run(app: &AppHandle, run: &str) -> Option<AgentKind> {
    let summary = app.state::<AppState>().store.run(run).ok().flatten()?;
    AgentKind::parse(&summary.agent)
}

/// A turn was refused for want of a login. The CLI has the last word when it
/// can say: logged in means this was something passing, and nobody has
/// anything to do.
async fn refused(app: &AppHandle, run: &str) {
    let Some(kind) = kind_of_run(app, run) else { return };
    if orchestra_agents::auth_status(kind).await.is_some_and(|auth| auth.logged_in) {
        tracing::info!(agent = kind.id(), run, "a turn wanted a login, and the CLI says it has one");
        return;
    }
    set_lost(app, kind, true);
}

/// Mark an agent, or take the mark off, and tell the windows and the bell.
/// Nothing happens when it already is as asked, so a queue of turns failing
/// together rings once.
fn set_lost(app: &AppHandle, kind: AgentKind, lost: bool) {
    static ONE_AT_A_TIME: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let state = app.state::<AppState>();
    let status = {
        let _one = ONE_AT_A_TIME.lock();
        let Some(mut status) = state.load_agents().ok().flatten().and_then(|list| list.into_iter().find(|s| s.kind == kind)) else { return };
        if status.login_lost == lost {
            return;
        }
        status.login_lost = lost;
        match state.update_agent(status) {
            Ok(status) => status,
            Err(err) => {
                tracing::warn!(agent = kind.id(), %err, "could not keep an agent's login state");
                return;
            }
        }
    };
    let _ = app.emit("agent_status", &status);
    if lost {
        let (title, body) = lost_notice(&status.name, &status.login_command, crate::ko(app));
        crate::add_notice(app, "", &lost_kind(kind), &title, &body, json!({ "agent": kind.id() }));
    } else {
        tracing::info!(agent = kind.id(), "the agent's login is back");
        take_off_bell(app, &lost_kind(kind));
    }
}

/// What this kind of notice said is over.
fn take_off_bell(app: &AppHandle, kind: &str) {
    for n in app.state::<AppState>().store.read_unread_notices("", kind).unwrap_or_default() {
        let _ = app.emit("notice", &n);
    }
}

fn lost_notice(name: &str, command: &str, ko: bool) -> (String, String) {
    if ko {
        (format!("{name} 로그인이 풀렸습니다"), format!("다시 로그인할 때까지 이 머신의 {name} 실행은 모두 실패합니다. 이 머신에서: {command}"))
    } else {
        (format!("{name} is logged out"), format!("Every {name} run on this machine fails until you log in again. On this machine: {command}"))
    }
}

/// Look once.
async fn tick(app: &AppHandle) {
    let statuses = app.state::<AppState>().load_agents().ok().flatten().unwrap_or_default();
    for status in statuses {
        if !status.login_lost {
            // Detected again meanwhile: the mark went with the old status.
            take_off_bell(app, &lost_kind(status.kind));
        } else if orchestra_agents::auth_status(status.kind).await.is_some_and(|auth| auth.logged_in) {
            set_lost(app, status.kind, false);
        }
    }
}

/// Look every so often, for as long as the app runs.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            tick(&app).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_gives_the_command_in_either_language() {
        let (title, body) = lost_notice("Claude Code", "/x/claude auth login", true);
        assert!(title.contains("Claude Code"));
        assert!(body.ends_with("/x/claude auth login"));
        let (title, body) = lost_notice("Claude Code", "/x/claude auth login", false);
        assert_eq!(title, "Claude Code is logged out");
        assert!(body.ends_with("/x/claude auth login"));
    }

    #[test]
    fn each_agent_rings_on_its_own() {
        assert_ne!(lost_kind(AgentKind::ClaudeCode), lost_kind(AgentKind::Codex));
    }
}
