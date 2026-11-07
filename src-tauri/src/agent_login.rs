//! Knowing that an agent's login is gone, or about to be.
//!
//! Detection cannot tell: it looks once, and a probe only opens a session,
//! which Claude's adapter does logged out or not. A login that runs out shows
//! as every turn failing within a second, on a machine nobody is watching,
//! with the agent still marked ready. So the first turn refused for want of a
//! login marks the agent and rings the bell once, with the command that brings
//! it back; the login coming back takes both away, and the bell then says what
//! did not run meanwhile, schedules' and routines' turns among it, so whoever
//! comes back to it knows what to run again.
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

/// When a marked agent was first refused, per agent: what failed for want
/// of a login from then on was missed.
const LOST_SINCE: &str = "login_lost_since:";

/// How many of the turns missed on a track a notice names, and how long each
/// name may be; the rest it counts. A window's bell keeps the first 400
/// characters of a notice, and this keeps the whole list inside them.
const MISSED_NAMED: usize = 4;
const MISSED_NAME: usize = 48;

/// The bell's kind for this agent's lost login.
fn lost_kind(kind: AgentKind) -> String {
    format!("login_lost:{}", kind.id())
}

/// The bell's kind for what did not run while this agent was logged out.
fn missed_kind(kind: AgentKind) -> String {
    format!("login_missed:{}", kind.id())
}

fn since_key(kind: AgentKind) -> String {
    format!("{LOST_SINCE}{}", kind.id())
}

/// Marking and unmarking, and taking what was missed: one at a time, so the
/// mark and when it began change together.
static MARKING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// The bell's kind for this agent's login about to end.
fn ending_kind(kind: AgentKind) -> String {
    format!("login_ending:{}", kind.id())
}

/// A run's turn ended: what that says about its agent's login.
pub fn turn_ended(app: &AppHandle, run: &str, event: &AgentEvent) {
    read_ending(app, event, || kind_of_run(app, run));
}

/// The same for a turn with no run of its own, read apart from `pump`: a
/// routine's, or the agent describing files for the knowledge base.
pub fn agent_turn_ended(app: &AppHandle, agent: &str, event: &AgentEvent) {
    read_ending(app, event, || AgentKind::parse(agent));
}

fn read_ending(app: &AppHandle, event: &AgentEvent, kind: impl FnOnce() -> Option<AgentKind>) {
    match event {
        AgentEvent::Failed { error } if orchestra_acp::wants_login(error) => {
            if let Some(kind) = kind() {
                tell_refused(app, kind);
            }
        }
        // A turn the agent finished went through on its login. A cancel may
        // not have got that far.
        AgentEvent::Finished { stop_reason } if !stop_reason.eq_ignore_ascii_case("cancelled") => {
            let any_lost = app.state::<AppState>().load_agents().ok().flatten().is_some_and(|list| list.iter().any(|s| s.login_lost));
            if any_lost {
                if let Some(kind) = kind() {
                    came_through(app, kind);
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

/// One asking at a time. Asking may refresh a token, and CLIs side by side race
/// for its lock, which is how a healthy login comes to fail.
static ASKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn cli_says(kind: AgentKind) -> Option<AuthStatus> {
    let _one = ASKING.lock().await;
    orchestra_agents::auth_status(kind).await
}

fn marked(app: &AppHandle, kind: AgentKind) -> bool {
    app.state::<AppState>().load_agents().ok().flatten().is_some_and(|list| list.iter().any(|s| s.kind == kind && s.login_lost))
}

/// This agent was refused for want of a login. The CLI has the last word when
/// it can say: logged in means this was something passing, and nobody has
/// anything to do. Turns failing together ask once.
fn tell_refused(app: &AppHandle, kind: AgentKind) {
    // Before the refused turn is recorded as failed: it is the first of
    // what was missed.
    let at = crate::now_ms();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _one = ASKING.lock().await;
        if marked(&app, kind) {
            went_by(&app, kind, at);
            return;
        }
        if orchestra_agents::auth_status(kind).await.is_some_and(|auth| auth.logged_in) {
            tracing::info!(agent = kind.id(), "the agent wanted a login, and the CLI says it has one");
            return;
        }
        set_lost(&app, kind, Some(at));
    });
}

/// A marked agent was refused at `at`: the login went by then at the latest.
/// Turns failing together are asked about one at a time, in no set order, so
/// the one that marked the agent need not have been the first.
fn went_by(app: &AppHandle, kind: AgentKind, at: i64) {
    let state = app.state::<AppState>();
    let _one = MARKING.lock();
    if !marked(app, kind) {
        return;
    }
    let key = since_key(kind);
    let since = state.store.get_meta(&key).ok().flatten().and_then(|v| v.parse::<i64>().ok());
    if since.is_none_or(|since| at < since) {
        if let Err(err) = state.store.set_meta(&key, &at.to_string()) {
            tracing::warn!(agent = kind.id(), %err, "could not keep when an agent's login went");
        }
    }
}

/// A turn of a marked agent went through. That is proof the login is back,
/// unless the CLI says otherwise: a session can go on for hours on a token it
/// already holds, and a turn may never have needed one.
fn came_through(app: &AppHandle, kind: AgentKind) {
    if !marked(app, kind) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if cli_says(kind).await.is_some_and(|auth| !auth.logged_in) {
            return;
        }
        set_lost(&app, kind, None);
    });
}

/// Mark an agent as refused since `since`, or with `None` take the mark
/// off, and tell the windows and the bell. Nothing happens when it already
/// is as asked, so a queue of turns failing together rings once.
fn set_lost(app: &AppHandle, kind: AgentKind, since: Option<i64>) {
    let lost = since.is_some();
    let state = app.state::<AppState>();
    let status = {
        let _one = MARKING.lock();
        let Some(mut status) = state.load_agents().ok().flatten().and_then(|list| list.into_iter().find(|s| s.kind == kind)) else { return };
        if status.login_lost == lost {
            return;
        }
        status.login_lost = lost;
        let status = match state.update_agent(status) {
            Ok(status) => status,
            Err(err) => {
                tracing::warn!(agent = kind.id(), %err, "could not keep an agent's login state");
                return;
            }
        };
        // Only with the mark kept: on its own it would read as a login back.
        if let Some(since) = since {
            if let Err(err) = state.store.set_meta(&since_key(kind), &since.to_string()) {
                tracing::warn!(agent = kind.id(), %err, "could not keep when an agent's login went");
            }
        }
        status
    };
    let _ = app.emit("agent_status", &status);
    if lost {
        let (title, body) = lost_notice(&status.name, &status.login_command, crate::ko(app));
        crate::add_notice(app, "", &lost_kind(kind), &title, &body, json!({ "agent": kind.id() }));
        take_off_bell(app, &ending_kind(kind));
    } else {
        tracing::info!(agent = kind.id(), "the agent's login is back");
        take_off_bell(app, &lost_kind(kind));
        tell_missed(app, kind);
    }
}

/// One turn that did not run, for a notice: when it began, and what it was.
struct Missed {
    at: i64,
    what: String,
}

/// The login is back: say, once, what failed for want of it meanwhile, a
/// notice per track (opening it opens the track) and per routine. Nothing
/// when the agent is still marked, or nothing was missed.
fn tell_missed(app: &AppHandle, kind: AgentKind) {
    let state = app.state::<AppState>();
    let since = {
        let _one = MARKING.lock();
        if marked(app, kind) {
            return;
        }
        let key = since_key(kind);
        let Some(since) = state.store.get_meta(&key).ok().flatten() else { return };
        if let Err(err) = state.store.delete_meta(&key) {
            tracing::warn!(agent = kind.id(), %err, "could not let go of when an agent's login went");
            return;
        }
        since.parse::<i64>().ok()
    };
    let Some(since) = since else { return };
    let ko = crate::ko(app);
    let agent = kind.name();

    let runs = state.store.failed_runs_since(kind.id(), since).unwrap_or_default();
    let mut tracks: Vec<(String, Vec<Missed>)> = Vec::new();
    for r in runs.into_iter().filter(|r| r.error.as_deref().is_some_and(orchestra_acp::wants_login)) {
        let what = match crate::schedule::fired_from(&r.prompt) {
            Some(id) => {
                let message = state.store.schedule(id).ok().flatten().map(|s| s.message).unwrap_or_default();
                let label = if ko { format!("예약 #{id}") } else { format!("schedule #{id}") };
                first_line(&format!("{label}: {message}"), MISSED_NAME)
            }
            None => first_line(&r.prompt, MISSED_NAME),
        };
        let missed = Missed { at: r.started_at, what };
        match tracks.iter_mut().find(|(t, _)| *t == r.track) {
            Some((_, list)) => list.push(missed),
            None => tracks.push((r.track, vec![missed])),
        }
    }
    for (track, list) in tracks {
        let name = state.store.track(&track).ok().flatten().map(|t| t.name).unwrap_or_else(|| track.clone());
        let (title, body) = missed_on_track(&name, agent, &list, ko);
        crate::add_notice(app, &track, &missed_kind(kind), &title, &body, json!({ "agent": kind.id() }));
    }

    let routine_runs = state.store.failed_routine_runs_since(kind.id(), since).unwrap_or_default();
    let mut routines: Vec<(String, Vec<(i64, String)>)> = Vec::new();
    for r in routine_runs.into_iter().filter(|r| orchestra_acp::wants_login(&r.error)) {
        match routines.iter_mut().find(|(id, _)| *id == r.routine) {
            Some((_, list)) => list.push((r.started_at, r.id)),
            None => routines.push((r.routine, vec![(r.started_at, r.id)])),
        }
    }
    for (routine, list) in routines {
        let name = state.store.routine(&routine).ok().flatten().map(|r| r.name).unwrap_or_else(|| routine.clone());
        let times: Vec<i64> = list.iter().map(|(at, _)| *at).collect();
        let (title, body) = missed_routine(&name, agent, &times, ko);
        let last = list.last().map(|(_, run)| run.clone()).unwrap_or_default();
        crate::add_notice(app, "", &missed_kind(kind), &title, &body, json!({ "agent": kind.id(), "routine": routine, "run": last }));
    }
}

/// The first line of `text` with something on it, cut to `max` characters.
pub(crate) fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let short: String = line.chars().take(max).collect();
    if short.len() < line.len() { format!("{short}…") } else { short }
}

/// What a notice's list says after the turns it names.
fn and_more(left: usize, ko: bool) -> String {
    if ko { format!("외 {left}건") } else { format!("and {left} more") }
}

fn missed_on_track(track: &str, agent: &str, list: &[Missed], ko: bool) -> (String, String) {
    let n = list.len();
    let (title, head) = if ko {
        (format!("{track}에서 {n}건이 실행되지 못했습니다"), format!("{agent} 로그인이 풀려 있던 동안입니다. 아직 필요하면 다시 실행하세요."))
    } else if n == 1 {
        (format!("1 turn did not run in {track}"), format!("{agent} was logged out. Run it again if it is still needed."))
    } else {
        (format!("{n} turns did not run in {track}"), format!("{agent} was logged out. Run them again if they are still needed."))
    };
    let mut lines = vec![head];
    lines.extend(list.iter().take(MISSED_NAMED).map(|m| format!("· {} {}", crate::when(m.at), m.what)));
    if n > MISSED_NAMED {
        lines.push(and_more(n - MISSED_NAMED, ko));
    }
    (title, lines.join("\n"))
}

fn missed_routine(name: &str, agent: &str, times: &[i64], ko: bool) -> (String, String) {
    let n = times.len();
    let title = match (ko, n) {
        (true, 1) => format!("되풀이 {name}: 실행되지 못했습니다"),
        (true, _) => format!("되풀이 {name}: {n}번 실행되지 못했습니다"),
        (false, 1) => format!("Routine {name} did not run"),
        (false, _) => format!("Routine {name} did not run {n} times"),
    };
    let head = if ko { format!("{agent} 로그인이 풀려 있던 동안입니다. 아직 필요하면 다시 실행하세요.") } else { format!("{agent} was logged out. Run it again if it is still needed.") };
    let mut lines = vec![head];
    lines.extend(times.iter().take(MISSED_NAMED).map(|at| format!("· {}", crate::when(*at))));
    if n > MISSED_NAMED {
        lines.push(and_more(n - MISSED_NAMED, ko));
    }
    (title, lines.join("\n"))
}

/// A status newly detected, by detecting again or after a login or an install.
/// Detection says READY logged out or not, so a mark stays while the CLI says
/// the login is still gone.
pub async fn keep_mark(state: &AppState, mut status: AgentStatus) -> AgentStatus {
    let was_marked = state.load_agents().ok().flatten().is_some_and(|list| list.iter().any(|b| b.kind == status.kind && b.login_lost));
    status.login_lost = was_marked && status.readiness == Readiness::Ready && cli_says(status.kind).await.is_some_and(|auth| !auth.logged_in);
    status
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
    let Some(auth) = cli_says(kind).await else { return };
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
    // Before any waiting below, while `login_lost` still says what it says now.
    for status in statuses.iter().filter(|s| !s.login_lost) {
        // Detected again meanwhile: the mark went with the old status.
        take_off_bell(app, &lost_kind(status.kind));
        tell_missed(app, status.kind);
    }
    for status in statuses.iter().filter(|s| s.login_lost) {
        if cli_says(status.kind).await.is_some_and(|auth| auth.logged_in) {
            set_lost(app, status.kind, None);
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
        assert_ne!(missed_kind(AgentKind::ClaudeCode), missed_kind(AgentKind::Codex));
        // The bell draws these whole, for the list or the command at the end.
        for kind in [lost_kind, ending_kind, missed_kind] {
            assert!(kind(AgentKind::ClaudeCode).starts_with("login_"));
        }
    }

    #[test]
    fn what_did_not_run_on_a_track_is_listed_up_to_a_few() {
        let at = 1_793_854_800_000;
        let one = [Missed { at, what: "예약 #26: 밤사이 실패한 빌드 정리".into() }];
        let (title, body) = missed_on_track("ops", "Claude Code", &one, true);
        assert_eq!(title, "ops에서 1건이 실행되지 못했습니다");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("Claude Code 로그인이 풀려 있던 동안입니다"));
        assert_eq!(lines[1], format!("· {} 예약 #26: 밤사이 실패한 빌드 정리", crate::when(at)));
        let (title, body) = missed_on_track("ops", "Claude Code", &one, false);
        assert_eq!(title, "1 turn did not run in ops");
        assert!(body.starts_with("Claude Code was logged out. Run it again"));

        let many: Vec<Missed> = (0..7).map(|i| Missed { at: at + i * 60_000, what: format!("turn {i}") }).collect();
        let (title, body) = missed_on_track("ops", "Claude Code", &many, false);
        assert_eq!(title, "7 turns did not run in ops");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 1 + MISSED_NAMED + 1);
        assert!(lines[1].ends_with("turn 0") && lines[MISSED_NAMED].ends_with("turn 3"), "the first ones, in order");
        assert_eq!(lines[MISSED_NAMED + 1], "and 3 more");
        let (_, body) = missed_on_track("ops", "Claude Code", &many, true);
        assert_eq!(body.lines().last(), Some("외 3건"));
    }

    /// A window's bell keeps 400 characters of a notice (`notifyServer`): the
    /// count at the end must be inside them, however long the names.
    #[test]
    fn the_longest_list_fits_what_the_bell_keeps() {
        let name = first_line(&format!("schedule #12345: {}", "x".repeat(300)), MISSED_NAME);
        let many: Vec<Missed> = (0..120).map(|i| Missed { at: 1_793_854_800_000 + i, what: name.clone() }).collect();
        let times: Vec<i64> = many.iter().map(|m| m.at).collect();
        for ko in [true, false] {
            let (_, body) = missed_on_track("ops", "GitHub Copilot", &many, ko);
            assert!(body.chars().count() <= 400, "{} characters", body.chars().count());
            let (_, body) = missed_routine("nightly", "GitHub Copilot", &times, ko);
            assert!(body.chars().count() <= 400, "{} characters", body.chars().count());
        }
    }

    #[test]
    fn a_routine_that_did_not_run_says_when() {
        let at = 1_793_854_800_000;
        let (title, body) = missed_routine("nightly", "Claude Code", &[at], true);
        assert_eq!(title, "되풀이 nightly: 실행되지 못했습니다");
        assert_eq!(body.lines().nth(1), Some(format!("· {}", crate::when(at)).as_str()));
        let (title, _) = missed_routine("nightly", "Claude Code", &[at, at + 3_600_000], true);
        assert_eq!(title, "되풀이 nightly: 2번 실행되지 못했습니다");
        let (title, _) = missed_routine("nightly", "Claude Code", &[at], false);
        assert_eq!(title, "Routine nightly did not run");
        let (title, body) = missed_routine("nightly", "Claude Code", &[at, at + 1], false);
        assert_eq!(title, "Routine nightly did not run 2 times");
        assert_eq!(body.lines().count(), 3);
    }

    #[test]
    fn a_turn_is_named_by_its_first_line() {
        assert_eq!(first_line("\n  [decision] #12\nquestion: …", 60), "[decision] #12");
        let long = "가".repeat(80);
        assert_eq!(first_line(&long, 60), format!("{}…", "가".repeat(60)));
        assert_eq!(first_line(&long, 80), long);
        assert_eq!(first_line("", 60), "");
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
