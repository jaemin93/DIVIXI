//! Noticing that nothing is happening.
//!
//! Open work -- a decision nobody answered, a schedule waiting, a job being
//! watched, a report parked for a conductor -- and no turn on the track for
//! hours: that is the shape of the fifty-nine hours nobody noticed, and of
//! the quieter version where a card sits unanswered for a week. The bell
//! says so once per quiet stretch; activity clears it, and the next stretch
//! rings it again.

use std::time::Duration;

use serde_json::json;
use tauri::Manager;

use crate::{AppHandle, AppState};

/// How often the tracks are looked at. A minute: the alarm is about hours,
/// but a check that fires after one is one a person can watch happen.
const TICK: Duration = Duration::from_secs(60);

/// The setting that says how many hours of quiet are too many; 0 turns
/// the alarm off.
pub const SETTING: &str = "stall.hours";

/// Hours of quiet before the bell, when the setting says nothing.
pub const DEFAULT_HOURS: f64 = 6.0;

/// What a track has open, and when it last did anything. `None` when it
/// has nothing open.
#[derive(Debug, Clone, PartialEq)]
pub struct Quiet {
    pub open: Vec<String>,
    /// Unix milliseconds of the last activity.
    pub last: i64,
}

/// A track's open work and last activity, from the record.
fn quiet_of(state: &AppState, track: &str, ko: bool) -> Option<Quiet> {
    let store = &state.store;
    let mut open = Vec::new();
    let mut last: i64 = 0;
    let decisions = store.decisions(Some(track)).unwrap_or_default();
    let waiting: Vec<_> = decisions.iter().filter(|d| d.status == orchestra_store::DecisionStatus::Open && d.permission.is_none()).collect();
    if !waiting.is_empty() {
        open.push(if ko { format!("답 없는 결정 {}개 (#{})", waiting.len(), waiting.iter().map(|d| d.id.to_string()).collect::<Vec<_>>().join(", #")) } else { format!("{} unanswered decision(s) (#{})", waiting.len(), waiting.iter().map(|d| d.id.to_string()).collect::<Vec<_>>().join(", #")) });
    }
    for d in &decisions {
        last = last.max(d.decided_at.unwrap_or(d.created_at));
    }
    let schedules = store.schedules(Some(track), true).unwrap_or_default();
    if !schedules.is_empty() {
        open.push(if ko { format!("예약 {}개", schedules.len()) } else { format!("{} schedule(s)", schedules.len()) });
        for s in &schedules {
            last = last.max(s.last_at.unwrap_or(s.created_at));
        }
    }
    let jobs = store.jobs(Some(track), true).unwrap_or_default();
    if !jobs.is_empty() {
        open.push(if ko { format!("지켜보는 작업 {}개", jobs.len()) } else { format!("{} watched job(s)", jobs.len()) });
        for j in &jobs {
            last = last.max(j.changed_at);
        }
    }
    let parked = crate::conductor::parked_all(state).into_iter().filter(|p| p.track == track).count();
    if parked > 0 {
        open.push(if ko { format!("지휘자에게 전달 못 한 보고 {}개", parked) } else { format!("{parked} report(s) waiting for the conductor") });
    }
    if open.is_empty() {
        return None;
    }
    for r in store.runs().unwrap_or_default().into_iter().filter(|r| r.track == track) {
        last = last.max(r.ended_at.unwrap_or(r.started_at));
    }
    Some(Quiet { open, last })
}

/// Whether a quiet stretch is long enough to ring for.
pub fn too_quiet(last: i64, now: i64, hours: f64) -> bool {
    hours > 0.0 && (now - last) as f64 > hours * 3_600_000.0
}

/// Look at every track once.
pub async fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    let ko = crate::ko(app);
    let hours = state.store.get_meta(&format!("{}{SETTING}", crate::SETTING_PREFIX)).ok().flatten().and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(DEFAULT_HOURS);
    if hours <= 0.0 {
        return;
    }
    let now = crate::now_ms();
    for track in state.store.tracks().unwrap_or_default() {
        // A track with a turn in flight is not quiet, whatever the record's times say.
        if state.sessions.is_active(&track.id).await {
            continue;
        }
        let Some(q) = quiet_of(&state, &track.id, ko) else { continue };
        if !too_quiet(q.last, now, hours) {
            continue;
        }
        // Once per stretch: an unread stall notice from after the last activity is this one.
        let already = state.store.unread_notices().unwrap_or_default().into_iter().any(|n| n.track == track.id && n.kind == "stall" && n.created_at > q.last);
        if already {
            continue;
        }
        let since = crate::when(q.last);
        let quiet_ms = now - q.last;
        let span = if quiet_ms >= 3_600_000 {
            if ko { format!("{}시간째", quiet_ms / 3_600_000) } else { format!("{} h", quiet_ms / 3_600_000) }
        } else if ko {
            format!("{}분째", quiet_ms / 60_000)
        } else {
            format!("{} min", quiet_ms / 60_000)
        };
        let title = if ko { format!("{span} 아무것도 돌지 않습니다 · {}", track.name) } else { format!("Nothing has run for {span} · {}", track.name) };
        let body = if ko { format!("마지막 활동 {since}. 열린 일: {}", q.open.join(", ")) } else { format!("Last activity {since}. Open: {}", q.open.join(", ")) };
        tracing::info!(track = %track.id, quiet_minutes = quiet_ms / 60_000, "nothing has run on a track with open work");
        crate::add_notice(app, &track.id, "stall", &title, &body, json!({ "since": q.last }));
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
    fn a_stretch_is_too_quiet_past_the_hours_set() {
        let h = 3_600_000;
        assert!(!too_quiet(0, 5 * h, 6.0));
        assert!(too_quiet(0, 7 * h, 6.0));
        assert!(!too_quiet(0, 100 * h, 0.0), "off");
        assert!(too_quiet(0, h / 2 + 1, 0.5), "a fraction of an hour, for a test");
    }
}
