//! Knowing that an agent's login is gone, or about to be.
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
//!
//! Better still is hearing beforehand. A login has a fixed end, and the CLI's
//! own warning about it appears in a terminal, which a server does not have
//! open. So the app asks the CLI, and the bell says so a few days ahead.

use std::time::{Duration, Instant};

use orchestra_acp::{AgentSession, AgentSpec, SessionOptions};
use orchestra_agents::{AgentKind, AgentStatus, AuthStatus, Readiness};
use orchestra_core::AgentEvent;
use serde_json::json;
use tauri::{Emitter, Manager};

use crate::{AppHandle, AppState};

/// How often a marked agent is asked about its login, and how often stray
/// notices are looked for. A minute: someone has just logged in and is
/// looking at the bell.
const TICK: Duration = Duration::from_secs(60);

/// How often the end of a login is looked at. It only moves with a new login,
/// and asking runs the CLI, which may refresh a token.
const ENDING_EVERY: Duration = Duration::from_secs(6 * 3600);

const DAY_MS: i64 = 24 * 3_600_000;

/// How long before a login's end the bell says so: as long as the CLI's own
/// warning gives.
const WARN_BEFORE_MS: i64 = 3 * DAY_MS;

/// The login end already told about, per agent, so one end rings once.
const WARNED: &str = "login_warned:";

/// The bell's kind for this agent's lost login.
fn lost_kind(kind: AgentKind) -> String {
    format!("login_lost:{}", kind.id())
}

/// The bell's kind for this agent's login about to end.
fn ending_kind(kind: AgentKind) -> String {
    format!("login_ending:{}", kind.id())
}

/// A turn ended: what that says about its agent's login.
pub fn turn_ended(app: &AppHandle, run: &str, event: &AgentEvent) {
    match event {
        AgentEvent::Failed { error } if orchestra_acp::wants_login(error) => {
            if let Some(kind) = kind_of_run(app, run) {
                tell_refused(app, kind);
            }
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

/// Open a session for this agent. Some adapters refuse a missing login here,
/// before there is any turn to fail, and that is told the same way.
pub async fn open_session(app: &AppHandle, agent: &str, spec: &AgentSpec, opts: SessionOptions) -> anyhow::Result<AgentSession> {
    let opened = AgentSession::open(spec, opts).await;
    if let Err(err) = &opened {
        if orchestra_acp::wants_login(&err.to_string()) {
            if let Some(kind) = AgentKind::parse(agent) {
                tell_refused(app, kind);
            }
        }
    }
    opened
}

/// This agent was refused for want of a login. The CLI has the last word when
/// it can say: logged in means this was something passing, and nobody has
/// anything to do.
fn tell_refused(app: &AppHandle, kind: AgentKind) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if orchestra_agents::auth_status(kind).await.is_some_and(|auth| auth.logged_in) {
            tracing::info!(agent = kind.id(), "the agent wanted a login, and the CLI says it has one");
            return;
        }
        set_lost(&app, kind, true);
    });
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
        take_off_bell(app, &ending_kind(kind));
    } else {
        tracing::info!(agent = kind.id(), "the agent's login is back");
        take_off_bell(app, &lost_kind(kind));
    }
}

/// Someone asked to detect again. That says READY logged out or not, so a mark
/// stays while the CLI says the login is still gone.
pub async fn keep_marks(state: &AppState, mut statuses: Vec<AgentStatus>) -> Vec<AgentStatus> {
    let before = state.load_agents().ok().flatten().unwrap_or_default();
    for status in &mut statuses {
        let marked = before.iter().any(|b| b.kind == status.kind && b.login_lost);
        if marked && status.readiness == Readiness::Ready {
            status.login_lost = orchestra_agents::auth_status(status.kind).await.is_some_and(|auth| !auth.logged_in);
        }
    }
    statuses
}

/// What this kind of notice said is over.
fn take_off_bell(app: &AppHandle, kind: &str) {
    for n in app.state::<AppState>().store.read_unread_notices("", kind).unwrap_or_default() {
        let _ = app.emit("notice", &n);
    }
}

fn lost_notice(name: &str, command: &str, ko: bool) -> (String, String) {
    if ko {
        (format!("{name} 로그인이 풀렸습니다"), format!("이 머신에서 다시 로그인: {command}"))
    } else {
        (format!("{name} is logged out"), format!("Log in again on this machine: {command}"))
    }
}

/// When this login ends, if that is close enough to say so.
fn ends_soon(auth: &AuthStatus, now: i64) -> Option<i64> {
    if !auth.logged_in {
        return None;
    }
    let ends = chrono::DateTime::parse_from_rfc3339(auth.login_ends.as_deref()?).ok()?.timestamp_millis();
    (ends - now <= WARN_BEFORE_MS).then_some(ends)
}

fn ending_notice(name: &str, command: &str, ends: i64, now: i64, when: &str, ko: bool) -> (String, String) {
    let days = ((ends - now) as f64 / DAY_MS as f64).ceil() as i64;
    if ko {
        let title = if days > 1 { format!("{name} 로그인이 {days}일 뒤 끝납니다") } else { format!("{name} 로그인이 곧 끝납니다") };
        (title, format!("{when} 전에 이 머신에서 다시 로그인: {command}"))
    } else {
        let title = if days > 1 { format!("{name}'s login ends in {days} days") } else { format!("{name}'s login is about to end") };
        (title, format!("Before {when}, log in again on this machine: {command}"))
    }
}

/// Say that a login is about to end, once per end; take the saying back when
/// a new login moved the end away.
async fn look_at_ending(app: &AppHandle, kind: AgentKind) {
    let state = app.state::<AppState>();
    let found = state.load_agents().ok().flatten().and_then(|list| list.into_iter().find(|s| s.kind == kind));
    let Some(status) = found.filter(|s| s.readiness == Readiness::Ready) else { return };
    let Some(auth) = orchestra_agents::auth_status(kind).await else { return };
    let now = crate::now_ms();
    let Some(ends) = ends_soon(&auth, now) else {
        take_off_bell(app, &ending_kind(kind));
        return;
    };
    let key = format!("{WARNED}{}", kind.id());
    if state.store.get_meta(&key).ok().flatten().as_deref() == Some(ends.to_string().as_str()) {
        return;
    }
    let (title, body) = ending_notice(&status.name, &status.login_command, ends, now, &crate::when(ends), crate::ko(app));
    if crate::add_notice(app, "", &ending_kind(kind), &title, &body, json!({ "agent": kind.id(), "ends": ends })).is_some() {
        if let Err(err) = state.store.set_meta(&key, &ends.to_string()) {
            tracing::warn!(agent = kind.id(), %err, "could not keep that a login's end was told");
        }
    }
}

/// Look once. `ending` is when the end of a login was last looked at.
async fn tick(app: &AppHandle, ending: &mut Option<Instant>) {
    let statuses = app.state::<AppState>().load_agents().ok().flatten().unwrap_or_default();
    for status in statuses {
        if !status.login_lost {
            // Detected again meanwhile: the mark went with the old status.
            take_off_bell(app, &lost_kind(status.kind));
        } else if orchestra_agents::auth_status(status.kind).await.is_some_and(|auth| auth.logged_in) {
            set_lost(app, status.kind, false);
            // A new login: its end is worth a look now.
            *ending = None;
        }
    }
    if ending.is_none_or(|at| at.elapsed() >= ENDING_EVERY) {
        *ending = Some(Instant::now());
        look_at_ending(app, AgentKind::ClaudeCode).await;
    }
}

/// Look every so often, for as long as the app runs.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ending = None;
        loop {
            tokio::time::sleep(TICK).await;
            tick(&app, &mut ending).await;
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
        assert_ne!(ending_kind(AgentKind::ClaudeCode), ending_kind(AgentKind::Codex));
    }

    fn auth(logged_in: bool, ends: Option<&str>) -> AuthStatus {
        AuthStatus { logged_in, login_ends: ends.map(str::to_owned) }
    }

    #[test]
    fn a_login_is_about_to_end_only_within_the_warning() {
        let ends = chrono::DateTime::parse_from_rfc3339("2026-11-25T20:30:33.671Z").unwrap().timestamp_millis();
        let on = auth(true, Some("2026-11-25T20:30:33.671Z"));
        assert_eq!(ends_soon(&on, ends - 4 * DAY_MS), None, "four days out");
        assert_eq!(ends_soon(&on, ends - 3 * DAY_MS), Some(ends), "three days out");
        assert_eq!(ends_soon(&on, ends + 1000), Some(ends), "past it, while an access token still lasts");
    }

    #[test]
    fn nothing_is_about_to_end_without_a_login_that_ends() {
        assert_eq!(ends_soon(&auth(false, Some("2026-11-25T20:30:33.671Z")), 0), None, "logged out");
        assert_eq!(ends_soon(&auth(true, None), 0), None, "an API key, or a CLI too old to say");
        assert_eq!(ends_soon(&auth(true, Some("soon")), 0), None, "not a time");
    }

    #[test]
    fn the_warning_counts_whole_days_and_gives_the_command() {
        let (title, body) = ending_notice("Claude Code", "/x/claude auth login", 3 * DAY_MS, 0, "11-26 05:30", true);
        assert_eq!(title, "Claude Code 로그인이 3일 뒤 끝납니다");
        assert!(body.contains("11-26 05:30") && body.ends_with("/x/claude auth login"));
        let (title, _) = ending_notice("Claude Code", "c", 2 * DAY_MS + 1, 0, "w", true);
        assert_eq!(title, "Claude Code 로그인이 3일 뒤 끝납니다", "part of a day counts");
        let (title, _) = ending_notice("Claude Code", "c", DAY_MS, 0, "w", true);
        assert_eq!(title, "Claude Code 로그인이 곧 끝납니다");
        let (title, _) = ending_notice("Claude Code", "c", -1000, 0, "w", false);
        assert_eq!(title, "Claude Code's login is about to end", "past its end, an access token may still last");
        let (title, body) = ending_notice("Claude Code", "/x/claude auth login", 2 * DAY_MS, 0, "11-26 05:30", false);
        assert_eq!(title, "Claude Code's login ends in 2 days");
        assert!(body.contains("11-26 05:30") && body.ends_with("/x/claude auth login"));
    }
}
