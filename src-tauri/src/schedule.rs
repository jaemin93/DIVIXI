//! Schedules that outlive sessions.
//!
//! A conductor that wants to look at something again at ten past seven
//! has, as an agent, only the schedule its own session offers -- and that
//! goes when the session is closed as idle, when the app restarts, when
//! the machine reboots. Five of them went that way in one week. These are
//! kept in the record instead (`schedules` in crates/store) and fired by
//! the app: a time, a repetition, or a condition (a file saying something,
//! a process ending), each with the message the conductor is to be woken
//! with. The conductor's session is opened for it when it is closed.
//!
//! A schedule whose time passed while the app was off fires once when the
//! app is back, says it is late and by how much, and a repeating one then
//! moves on to its next time from now rather than catching up on every
//! firing it missed.
//!
//! Times are the machine's local time, as a person writes them.

use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Weekday};
use orchestra_store::{NewSchedule, Schedule, ScheduleStatus};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::conductor::{deliver, Hand};
use crate::{AppHandle, AppState};

/// How often schedules are looked at.
const TICK: Duration = Duration::from_secs(30);

/// Past this much after its time, a firing is said to be late (the app or
/// the machine was off). Schedules are looked at every half minute, so
/// anything over two is not the tick.
const LATE: i64 = 2 * 60 * 1000;

/// The least a repetition may be.
const MIN_EVERY: i64 = 60;

/// The event the window reloads a track's schedule list on.
pub const EVENT: &str = "schedules";

/// A schedule as it was asked for, read into what the record keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub kind: String,
    pub spec: Value,
    /// The first firing, for a timed schedule.
    pub next_at: Option<i64>,
}

/// Read a schedule out of the words it was given (see the tool's
/// description for the shapes). `now` is Unix milliseconds.
pub fn parse(when: &Value, now: i64) -> Result<Parsed, String> {
    let Some(o) = when.as_object() else { return Err("`when` is an object such as {\"at\": \"2026-10-17 07:30\"}, {\"in\": \"30m\"}, {\"every\": \"2h\"}, {\"daily\": \"07:10\"}, {\"weekly\": \"sun 03:20\"}, {\"file\": \"/path/log\", \"contains\": \"DONE\"} or {\"pid\": 1234}".to_string()) };
    if let Some(at) = o.get("at") {
        let ms = parse_at(at)?;
        if ms <= now {
            return Err(format!("that time is already past ({})", crate::when(ms)));
        }
        return Ok(Parsed { kind: "at".into(), spec: json!({ "at": ms }), next_at: Some(ms) });
    }
    if let Some(v) = o.get("in").or_else(|| o.get("in_minutes")) {
        let secs = match v {
            Value::Number(n) if o.contains_key("in_minutes") => n.as_i64().unwrap_or(0) * 60,
            Value::Number(n) => n.as_i64().unwrap_or(0),
            Value::String(s) => parse_duration(s)?,
            _ => return Err("`in` is a span such as \"30m\", \"2h\" or \"90s\"".to_string()),
        };
        if secs <= 0 {
            return Err("`in` is a span in the future".to_string());
        }
        let ms = now + secs * 1000;
        return Ok(Parsed { kind: "at".into(), spec: json!({ "at": ms, "in": v }), next_at: Some(ms) });
    }
    if let Some(v) = o.get("every") {
        let secs = match v {
            Value::Number(n) => n.as_i64().unwrap_or(0),
            Value::String(s) => parse_duration(s)?,
            _ => return Err("`every` is a span such as \"30m\", \"2h\" or \"1d\"".to_string()),
        };
        if secs < MIN_EVERY {
            return Err(format!("`every` is at least {MIN_EVERY} seconds"));
        }
        let until = match o.get("until") {
            Some(u) => Some(parse_at(u)?),
            None => None,
        };
        let first = now + secs * 1000;
        if until.is_some_and(|u| u < first) {
            return Err(format!("`until` ({}) comes before the first firing ({}): it would never fire", crate::when(until.unwrap_or(0)), crate::when(first)));
        }
        let mut spec = json!({ "every": secs });
        if let Some(u) = until {
            spec["until"] = json!(u);
        }
        return Ok(Parsed { kind: "every".into(), spec, next_at: Some(first) });
    }
    if let Some(v) = o.get("daily") {
        let t = parse_hhmm(v.as_str().unwrap_or_default())?;
        let next = next_daily(now, t);
        return Ok(Parsed { kind: "daily".into(), spec: json!({ "daily": format!("{:02}:{:02}", t.hour(), t.minute()) }), next_at: Some(next) });
    }
    if let Some(v) = o.get("weekly") {
        let s = v.as_str().unwrap_or_default().trim();
        let (day, time) = s.split_once(' ').ok_or_else(|| format!("`weekly` is a day and a time, such as \"sun 03:20\", not {s:?}"))?;
        let day = crate::quiet::weekday(day).ok_or_else(|| format!("not a day: {day:?}"))?;
        let t = parse_hhmm(time)?;
        let next = next_weekly(now, day, t);
        return Ok(Parsed { kind: "weekly".into(), spec: json!({ "weekly": format!("{} {:02}:{:02}", day_name(day), t.hour(), t.minute()) }), next_at: Some(next) });
    }
    if let Some(f) = o.get("file") {
        let path = f.as_str().map(str::trim).filter(|p| !p.is_empty()).ok_or("`file` is a path")?;
        // Kept and looked at later by the app, whose own folder is not the
        // track's: a relative path would name some other file.
        if !std::path::Path::new(path).is_absolute() {
            return Err(format!("`file` is an absolute path, not {path:?}: the app looks at it from its own folder, not the track's"));
        }
        let contains = o.get("contains").and_then(Value::as_str).unwrap_or_default().to_string();
        let pattern = o.get("pattern").and_then(Value::as_str).unwrap_or_default().to_string();
        if contains.is_empty() && pattern.is_empty() {
            return Err("a `file` schedule says what to look for: `contains` (words) or `pattern` (a regular expression)".to_string());
        }
        if !pattern.is_empty() {
            regex::Regex::new(&pattern).map_err(|e| format!("not a regular expression: {e}"))?;
        }
        return Ok(Parsed { kind: "file".into(), spec: json!({ "file": path, "contains": contains, "pattern": pattern }), next_at: None });
    }
    if let Some(p) = o.get("pid") {
        let pid = p.as_u64().and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0).ok_or("`pid` is a process id")?;
        if !crate::boot::alive(pid) {
            return Err(format!("no process {pid} is running now"));
        }
        // The boot the process belongs to: after a reboot it is gone, and
        // its number may be somebody else's.
        // And when it started, so the number given to another process
        // later on the same boot is not taken for it.
        return Ok(Parsed { kind: "process".into(), spec: json!({ "pid": pid, "boot": crate::boot::id(), "start": crate::boot::started(pid) }), next_at: None });
    }
    Err("`when` names one of: at, in, every, daily, weekly, file, pid".to_string())
}

fn day_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

/// "30m", "2h", "90s", "1d", "1h30m" to seconds.
pub fn parse_duration(s: &str) -> Result<i64, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Err("an empty span".to_string());
    }
    let mut total = 0i64;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().map_err(|_| format!("not a span: {s:?}"))?;
        num.clear();
        total += match c {
            's' => n,
            'm' => n * 60,
            'h' => n * 3600,
            'd' => n * 86_400,
            ' ' => continue,
            _ => return Err(format!("not a span: {s:?} (use s, m, h or d)")),
        };
    }
    if !num.is_empty() {
        // A bare number is minutes.
        total += num.parse::<i64>().map_err(|_| format!("not a span: {s:?}"))? * 60;
    }
    Ok(total)
}

fn parse_hhmm(s: &str) -> Result<NaiveTime, String> {
    let (h, m) = s.trim().split_once(':').ok_or_else(|| format!("a time is HH:MM, not {s:?}"))?;
    let (h, m): (u32, u32) = (h.trim().parse().map_err(|_| format!("a time is HH:MM, not {s:?}"))?, m.trim().parse().map_err(|_| format!("a time is HH:MM, not {s:?}"))?);
    NaiveTime::from_hms_opt(h, m, 0).ok_or_else(|| format!("a time is HH:MM, not {s:?}"))
}

/// A moment: Unix milliseconds as a number, RFC 3339 ("2026-10-17T07:30:00+09:00"),
/// or local "2026-10-17 07:30" / "2026-10-17T07:30".
pub fn parse_at(v: &Value) -> Result<i64, String> {
    match v {
        Value::Number(n) => n.as_i64().ok_or_else(|| "not a time".to_string()),
        Value::String(s) => {
            let s = s.trim();
            if let Ok(t) = DateTime::parse_from_rfc3339(s) {
                return Ok(t.timestamp_millis());
            }
            for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
                if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                    return Local.from_local_datetime(&naive).single().map(|t| t.timestamp_millis()).ok_or_else(|| format!("{s:?} is not a time that exists here"));
                }
            }
            Err(format!("not a time: {s:?} (use \"YYYY-MM-DD HH:MM\" in local time, or RFC 3339 with an offset)"))
        }
        _ => Err("not a time".to_string()),
    }
}

fn local(ms: i64) -> DateTime<Local> {
    Local.timestamp_millis_opt(ms).single().unwrap_or_else(Local::now)
}

fn to_ms(date: NaiveDate, time: NaiveTime) -> Option<i64> {
    Local.from_local_datetime(&date.and_time(time)).single().map(|t| t.timestamp_millis())
}

/// The next `time` strictly after `now`, today or tomorrow.
pub fn next_daily(now: i64, time: NaiveTime) -> i64 {
    let here = local(now);
    let today = here.date_naive();
    for days in 0..3 {
        let date = today + chrono::Duration::days(days);
        if let Some(ms) = to_ms(date, time) {
            if ms > now {
                return ms;
            }
        }
    }
    now + 86_400_000
}

/// The next `day` at `time` strictly after `now`.
pub fn next_weekly(now: i64, day: Weekday, time: NaiveTime) -> i64 {
    let here = local(now);
    let today = here.date_naive();
    for days in 0..9 {
        let date = today + chrono::Duration::days(days);
        if date.weekday() != day {
            continue;
        }
        if let Some(ms) = to_ms(date, time) {
            if ms > now {
                return ms;
            }
        }
    }
    now + 7 * 86_400_000
}

/// When a repeating schedule fires next after `now`; `None` ends it.
pub fn next_after(s: &Schedule, now: i64) -> Option<i64> {
    match s.kind.as_str() {
        "every" => {
            let secs = s.spec.get("every").and_then(Value::as_i64)?.max(MIN_EVERY);
            let next = now + secs * 1000;
            match s.spec.get("until").and_then(Value::as_i64) {
                Some(until) if next > until => None,
                _ => Some(next),
            }
        }
        "daily" => {
            let t = parse_hhmm(s.spec.get("daily").and_then(Value::as_str)?).ok()?;
            Some(next_daily(now, t))
        }
        "weekly" => {
            let (d, t) = s.spec.get("weekly").and_then(Value::as_str)?.split_once(' ')?;
            Some(next_weekly(now, crate::quiet::weekday(d)?, parse_hhmm(t).ok()?))
        }
        _ => None,
    }
}

/// One line on a schedule for a person or the conductor.
pub fn describe(s: &Schedule, ko: bool) -> String {
    let when = match s.kind.as_str() {
        "at" => s.next_at.map(crate::when).unwrap_or_default(),
        "every" => {
            let secs = s.spec.get("every").and_then(Value::as_i64).unwrap_or(0);
            let span = span(secs);
            match s.next_at {
                Some(n) => format!("{} {span} · {} {}", if ko { "매" } else { "every" }, if ko { "다음" } else { "next" }, crate::when(n)),
                None => format!("{} {span}", if ko { "매" } else { "every" }),
            }
        }
        "daily" => format!("{} {} · {} {}", if ko { "매일" } else { "daily" }, s.spec.get("daily").and_then(Value::as_str).unwrap_or(""), if ko { "다음" } else { "next" }, s.next_at.map(crate::when).unwrap_or_default()),
        "weekly" => format!("{} {} · {} {}", if ko { "매주" } else { "weekly" }, s.spec.get("weekly").and_then(Value::as_str).unwrap_or(""), if ko { "다음" } else { "next" }, s.next_at.map(crate::when).unwrap_or_default()),
        "file" => format!(
            "{} {} {} {:?}",
            if ko { "파일" } else { "file" },
            s.spec.get("file").and_then(Value::as_str).unwrap_or(""),
            if ko { "에 생기면:" } else { "says" },
            s.spec.get("pattern").and_then(Value::as_str).filter(|p| !p.is_empty()).or_else(|| s.spec.get("contains").and_then(Value::as_str)).unwrap_or("")
        ),
        "process" => format!("{} {} {}", if ko { "프로세스" } else { "process" }, s.spec.get("pid").and_then(Value::as_u64).unwrap_or(0), if ko { "가 끝나면" } else { "ends" }),
        other => other.to_string(),
    };
    let status = match s.status {
        ScheduleStatus::Active => "",
        ScheduleStatus::Done => if ko { " · 끝남" } else { " · done" },
        ScheduleStatus::Cancelled => if ko { " · 취소됨" } else { " · cancelled" },
    };
    format!("#{} {when}{status}: {}", s.id, s.message)
}

fn span(secs: i64) -> String {
    if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// A schedule as the conductor's tools and the window read it.
pub fn view(s: &Schedule, ko: bool) -> Value {
    json!({
        "id": s.id,
        "track": s.track,
        "kind": s.kind,
        "spec": s.spec,
        "message": s.message,
        "next_at": s.next_at,
        "last_at": s.last_at,
        "fired": s.fired,
        "status": s.status,
        "note": s.note,
        "line": describe(s, ko),
    })
}

/// Set a schedule on a track. `created_in` is the conductor run that asked.
pub fn create(app: &AppHandle, track: &str, when: &Value, message: &str, created_in: Option<String>) -> Result<Schedule, String> {
    let state = app.state::<AppState>();
    let message = message.trim();
    if message.is_empty() {
        return Err("say what the conductor should be told when it fires (`message`)".to_string());
    }
    let parsed = parse(when, crate::now_ms())?;
    let s = state
        .store
        .create_schedule(&NewSchedule { track: track.to_string(), kind: parsed.kind, spec: parsed.spec, message: message.to_string(), next_at: parsed.next_at, created_in })
        .map_err(|e| e.to_string())?;
    tracing::info!(%track, schedule = s.id, kind = %s.kind, next_at = ?s.next_at, "schedule set");
    let _ = app.emit(EVENT, track);
    Ok(s)
}

pub fn cancel(app: &AppHandle, id: i64) -> Result<Schedule, String> {
    let state = app.state::<AppState>();
    let s = state.store.cancel_schedule(id).map_err(|e| e.to_string())?;
    let _ = app.emit(EVENT, &s.track);
    Ok(s)
}

/// Look at every schedule once: fire what is due, check each condition.
pub async fn tick(app: &AppHandle) {
    let now = crate::now_ms();
    let (due, conditions) = {
        let state = app.state::<AppState>();
        let due = state.store.due_schedules(now).unwrap_or_default();
        let conditions: Vec<Schedule> = state
            .store
            .schedules(None, true)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.next_at.is_none() && matches!(s.kind.as_str(), "file" | "process"))
            .collect();
        (due, conditions)
    };
    for s in due {
        fire(app, s, now).await;
    }
    for s in conditions {
        // Inside the track's blocking quiet hours a condition is not even
        // looked at: what it waits for is still there (the line in the
        // file, past the read mark; the process gone) when they end, and
        // fires then, as a timed schedule is moved past them.
        let held = {
            let state = app.state::<AppState>();
            crate::quiet::active(&crate::quiet::get(&state, &s.track), now).is_some_and(|(w, _)| w.blocks())
        };
        if held {
            continue;
        }
        if s.kind == "file" {
            // What was read is kept even when nothing was found, so the
            // next look goes on from there and a restart does too.
            let Some(looked) = file_says(&s) else { continue };
            if Some(looked.seen) != s.seen || looked.sum != s.seen_sum {
                let _ = app.state::<AppState>().store.schedule_seen(s.id, looked.seen, looked.sum);
            }
            if let Some(found) = looked.found {
                fire_condition(app, s, found).await;
            }
        } else if let Some(found) = condition_met(&s) {
            fire_condition(app, s, found).await;
        }
    }
}

/// Whether a condition holds now, and what was seen.
fn condition_met(s: &Schedule) -> Option<String> {
    match s.kind.as_str() {
        "file" => file_says(s)?.found,
        "process" => {
            let pid = u32::try_from(s.spec.get("pid").and_then(Value::as_u64)?).ok()?;
            let boot = s.spec.get("boot").and_then(Value::as_str).unwrap_or("");
            let start = s.spec.get("start").and_then(Value::as_u64);
            process_ended(pid, boot, &crate::boot::id(), |p| crate::boot::same_alive(p, start))
        }
        _ => None,
    }
}

/// Whether the process a schedule waits on has ended: on another boot it
/// has, whatever its number says now; on the same boot the number is asked.
fn process_ended(pid: u32, boot: &str, now_boot: &str, alive: impl Fn(u32) -> bool) -> Option<String> {
    if !boot.is_empty() && !now_boot.is_empty() && boot != now_boot {
        return Some(format!("process {pid} ended: the machine rebooted"));
    }
    (!alive(pid)).then(|| format!("process {pid} ended"))
}

/// What one look at a watched file found, and how far it has now read.
struct Looked {
    found: Option<String>,
    seen: u64,
    /// The fingerprint at `seen` (see `jobs::mark_sum`).
    sum: Option<i64>,
}

/// About this much per look, to the end of the piece that reaches it: a
/// file that grew by more is caught up with over the next looks, nothing
/// skipped. The pieces are read as a watched job's log is
/// (`jobs::read_lines`): whole lines, a long line read on to its end, a
/// line written across two looks seen whole.
const FILE_LOOK: u64 = 16 * 1024 * 1024;
/// A file nobody has looked at yet is read from this far before its end:
/// what it says now, not its whole history.
const FILE_FIRST: u64 = 1 << 20;

/// One look at a `file` schedule's file: everything written since the
/// last look (from about the last MiB on the first one, from the top when
/// the file was cut shorter than what was read), piece by piece, for the
/// words or the pattern. Stops at the line that says it. `None` when the
/// file cannot be read or the schedule has nothing to look for.
fn file_says(s: &Schedule) -> Option<Looked> {
    let path = s.spec.get("file").and_then(Value::as_str)?;
    let contains = s.spec.get("contains").and_then(Value::as_str).unwrap_or("");
    let pattern = s.spec.get("pattern").and_then(Value::as_str).unwrap_or("");
    // Line by line, as a person writes `^done$`.
    let re = (!pattern.is_empty()).then(|| regex::Regex::new(&format!("(?m){pattern}")).ok()).flatten();
    if re.is_none() && contains.is_empty() {
        return None;
    }
    let find = |text: &str| match &re {
        Some(re) => re.find(text).map(|m| line_of(text, m.start())),
        None => text.find(contains).map(|at| line_of(text, at)),
    };
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut from = match s.seen {
        // The same file grown on, or else (shorter, or other bytes before
        // the mark: replaced) read again from its start.
        Some(seen) => crate::jobs::resume_at(&mut f, len, seen, s.seen_sum),
        // The first piece goes back to the start of the line this falls in.
        None => len.saturating_sub(FILE_FIRST),
    };
    let stop = len.min(from.saturating_add(FILE_LOOK));
    while from < stop {
        let (buf, end) = crate::jobs::read_lines(&mut f, from, len)?;
        let text = String::from_utf8_lossy(&buf);
        if let Some(line) = find(&text) {
            return Some(Looked { found: Some(line), seen: end, sum: crate::jobs::mark_sum(&mut f, end) });
        }
        from = end;
    }
    Some(Looked { found: None, seen: from, sum: crate::jobs::mark_sum(&mut f, from) })
}

/// The line `at` falls in.
fn line_of(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[at..].find('\n').map(|i| at + i).unwrap_or(text.len());
    text[start..end].trim().to_string()
}

/// Fire a timed schedule: wake the conductor with its message, say when
/// it was due and whether it is late, and move it on or end it. Inside the
/// track's quiet hours it is moved to their end instead.
async fn fire(app: &AppHandle, s: Schedule, now: i64) {
    let ko = crate::ko(app);
    let due_at = s.next_at.unwrap_or(now);
    let late = now - due_at;
    {
        let state = app.state::<AppState>();
        if let Some((w, end)) = crate::quiet::active(&crate::quiet::get(&state, &s.track), now) {
            if w.blocks() {
                match past_quiet(&s, end) {
                    Some(later) => {
                        let note = if ko { format!("금지 시간대({})라 {}로 미룸", w.line(), crate::when(later)) } else { format!("moved to {} past quiet hours ({})", crate::when(later), w.line()) };
                        tracing::info!(schedule = s.id, %note, "schedule moved");
                        let _ = state.store.schedule_moved(s.id, later, &note);
                    }
                    None => {
                        let note = if ko { format!("금지 시간대({})가 끝나기 전에 until 이 지나 끝남", w.line()) } else { format!("over: its until comes before quiet hours ({}) end", w.line()) };
                        tracing::info!(schedule = s.id, %note, "schedule ended inside quiet hours");
                        let _ = state.store.end_schedule(s.id, &note);
                    }
                }
                let _ = app.emit(EVENT, &s.track);
                return;
            }
        }
    }
    let late_note = late_note(late, due_at, ko);
    let head = if ko { format!("[schedule] #{} · {} 예정{late_note}", s.id, crate::when(due_at)) } else { format!("[schedule] #{} · due {}{late_note}", s.id, crate::when(due_at)) };
    let text = format!(
        "{head}\n{}\n\n({})",
        s.message,
        if ko {
            "당신이 schedule 로 걸어 둔 예약이 때가 되어 앱이 깨웠습니다. 위 message 가 그때 적어 둔 할 일입니다. 하고 나서 사람에게 한두 문장으로 알리세요. 늦게 실행된 예약은 그 사이 상황이 달라졌을 수 있으니 먼저 확인합니다."
        } else {
            "A schedule you set with `schedule` came due and the app woke you. The message above is what you wrote down to do. Do it, then tell the human in a sentence or two. A late one may find things changed meanwhile: check first."
        }
    );
    let next = next_after(&s, now);
    let note = late_note.trim_start_matches(" · ").to_string();
    {
        let state = app.state::<AppState>();
        if let Err(err) = state.store.schedule_fired(s.id, next, &note) {
            tracing::warn!(schedule = s.id, %err, "could not mark a schedule fired");
            return;
        }
    }
    tracing::info!(schedule = s.id, track = %s.track, late_ms = late, next = ?next, "schedule fired");
    let _ = app.emit(EVENT, &s.track);
    deliver(app.clone(), Hand { track: s.track.clone(), text, open: true, what: format!("schedule #{}", s.id), keep: true, worker: None, run: None }).await;
}

/// Where a schedule due inside quiet hours goes: just past their end --
/// unless that is past its own `until`, in which case nowhere: a
/// repetition asked for until four runs no later than four, quiet hours
/// or not.
fn past_quiet(s: &Schedule, quiet_end: i64) -> Option<i64> {
    let later = quiet_end + 1000;
    match s.spec.get("until").and_then(Value::as_i64) {
        Some(until) if later > until => None,
        _ => Some(later),
    }
}

/// What a firing says about being late: nothing within the tick's own
/// slack, and past that how long past when, with why (the app or the
/// machine was off -- nothing else holds a schedule back).
fn late_note(late_ms: i64, due_at: i64, ko: bool) -> String {
    if late_ms <= LATE {
        return String::new();
    }
    let span = span(late_ms / 1000);
    if ko {
        format!(" · 늦게 실행됨: 예정 {}에서 {span} 지남 (앱이나 기계가 꺼져 있었음)", crate::when(due_at))
    } else {
        format!(" · late: {span} past {} (the app or the machine was off)", crate::when(due_at))
    }
}

/// A condition came true: wake the conductor once and end the schedule.
async fn fire_condition(app: &AppHandle, s: Schedule, found: String) {
    let ko = crate::ko(app);
    let head = if ko { format!("[schedule] #{} · 조건 충족: {found}", s.id) } else { format!("[schedule] #{} · condition met: {found}", s.id) };
    let text = format!(
        "{head}\n{}\n\n({})",
        s.message,
        if ko {
            "당신이 schedule 로 걸어 둔 조건이 충족되어 앱이 깨웠습니다. 위 message 가 그때 적어 둔 할 일입니다. 이 예약은 한 번 울리고 끝납니다."
        } else {
            "A condition you set with `schedule` came true and the app woke you. The message above is what you wrote down to do. This schedule fires once and is over."
        }
    );
    {
        let state = app.state::<AppState>();
        if let Err(err) = state.store.schedule_fired(s.id, None, &found) {
            tracing::warn!(schedule = s.id, %err, "could not mark a schedule fired");
            return;
        }
    }
    tracing::info!(schedule = s.id, track = %s.track, %found, "schedule condition met");
    let _ = app.emit(EVENT, &s.track);
    deliver(app.clone(), Hand { track: s.track.clone(), text, open: true, what: format!("schedule #{}", s.id), keep: true, worker: None, run: None }).await;
}

/// Look at the schedules every so often, for as long as the app runs.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tick(&app).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        to_ms(NaiveDate::from_ymd_opt(y, mo, d).unwrap(), NaiveTime::from_hms_opt(h, mi, 0).unwrap()).unwrap()
    }

    #[test]
    fn spans_are_read() {
        assert_eq!(parse_duration("30m").unwrap(), 1800);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("90s").unwrap(), 90);
        assert_eq!(parse_duration("1d").unwrap(), 86_400);
        assert_eq!(parse_duration("1h30m").unwrap(), 5400);
        assert_eq!(parse_duration("15").unwrap(), 900, "a bare number is minutes");
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("").is_err());
        assert_eq!(span(7200), "2h");
        assert_eq!(span(90), "90s");
    }

    #[test]
    fn times_are_read_in_local_time_or_with_an_offset() {
        let ms = parse_at(&json!("2026-10-17 07:30")).unwrap();
        assert_eq!(ms, at_local(2026, 10, 17, 7, 30));
        assert_eq!(parse_at(&json!("2026-10-17T07:30")).unwrap(), ms);
        assert_eq!(parse_at(&json!(1_000_000)).unwrap(), 1_000_000);
        // An offset is honoured: 07:30 in Seoul is 22:30 UTC the day before.
        let z = parse_at(&json!("2026-10-17T07:30:00+09:00")).unwrap();
        assert_eq!(z, DateTime::parse_from_rfc3339("2026-10-16T22:30:00Z").unwrap().timestamp_millis());
        assert!(parse_at(&json!("tomorrow")).is_err());
    }

    #[test]
    fn the_shapes_a_schedule_takes() {
        let now = at_local(2026, 10, 16, 12, 0);
        let p = parse(&json!({ "in": "30m" }), now).unwrap();
        assert_eq!((p.kind.as_str(), p.next_at), ("at", Some(now + 1800 * 1000)));
        let p = parse(&json!({ "in_minutes": 5 }), now).unwrap();
        assert_eq!(p.next_at, Some(now + 300 * 1000));
        assert!(parse(&json!({ "at": "2026-10-16 11:00" }), now).is_err(), "already past");
        let p = parse(&json!({ "every": "2h", "until": "2026-10-17 00:00" }), now).unwrap();
        assert_eq!((p.kind.as_str(), p.next_at), ("every", Some(now + 7200 * 1000)));
        assert_eq!(p.spec["until"], json!(at_local(2026, 10, 17, 0, 0)));
        assert!(parse(&json!({ "every": "10s" }), now).is_err(), "too often");
        assert!(parse(&json!({ "every": "2h", "until": "2026-10-16 13:00" }), now).is_err(), "over before its first firing");
        assert!(parse(&json!({ "every": "1h", "until": "2026-10-16 13:00" }), now).is_ok(), "right at its first firing is one firing");
        let p = parse(&json!({ "daily": "07:10" }), now).unwrap();
        assert_eq!((p.kind.as_str(), p.next_at), ("daily", Some(at_local(2026, 10, 17, 7, 10))), "tomorrow: today's is past");
        let p = parse(&json!({ "daily": "13:00" }), now).unwrap();
        assert_eq!(p.next_at, Some(at_local(2026, 10, 16, 13, 0)), "today: still ahead");
        // 2026-10-16 is a Friday; the next Sunday is the 18th.
        let p = parse(&json!({ "weekly": "sun 03:20" }), now).unwrap();
        assert_eq!((p.kind.as_str(), p.next_at), ("weekly", Some(at_local(2026, 10, 18, 3, 20))));
        assert_eq!(p.spec["weekly"], "sun 03:20");
        let p = parse(&json!({ "weekly": "일 03:20" }), now).unwrap();
        assert_eq!(p.next_at, Some(at_local(2026, 10, 18, 3, 20)), "a Korean day name");
        let p = parse(&json!({ "weekly": "fri 11:00" }), now).unwrap();
        assert_eq!(p.next_at, Some(at_local(2026, 10, 23, 11, 0)), "this Friday's time is past: next week");
        let log = std::env::temp_dir().join("x.log").to_string_lossy().into_owned();
        assert!(parse(&json!({ "file": "build.log", "contains": "DONE" }), now).is_err(), "relative to what");
        let p = parse(&json!({ "file": log, "contains": "DONE" }), now).unwrap();
        assert_eq!((p.kind.as_str(), p.next_at), ("file", None));
        assert!(parse(&json!({ "file": log }), now).is_err(), "nothing to look for");
        assert!(parse(&json!({ "file": log, "pattern": "(" }), now).is_err(), "not a regex");
        assert!(parse(&json!({ "pid": 0 }), now).is_err());
        assert!(parse(&json!({ "pid": 4_294_967_297u64 }), now).is_err(), "not a pid, whatever its low bits say");
        let p = parse(&json!({ "pid": std::process::id() }), now).unwrap();
        assert_eq!(p.spec["boot"], json!(crate::boot::id()), "this process is alive, on this boot");
        assert_eq!(p.spec["start"], json!(crate::boot::started(std::process::id())), "and started when it did");
        assert!(parse(&json!({ "soon": true }), now).is_err());
        assert!(parse(&json!("07:10"), now).is_err());
    }

    fn sched(kind: &str, spec: Value) -> Schedule {
        Schedule { id: 1, track: "tr001".into(), kind: kind.into(), spec, message: "m".into(), next_at: None, last_at: None, fired: 0, status: ScheduleStatus::Active, note: String::new(), created_at: 0, created_in: None, seen: None, seen_sum: None }
    }

    #[test]
    fn a_repeating_schedule_moves_on_from_now_not_from_when_it_was_due() {
        let now = at_local(2026, 10, 16, 12, 0);
        let every = sched("every", json!({ "every": 3600 }));
        assert_eq!(next_after(&every, now), Some(now + 3_600_000));
        let until = sched("every", json!({ "every": 3600, "until": now + 1000 }));
        assert_eq!(next_after(&until, now), None, "past its end");
        let daily = sched("daily", json!({ "daily": "07:10" }));
        assert_eq!(next_after(&daily, now), Some(at_local(2026, 10, 17, 7, 10)));
        // Fired three days late at noon: the next is tomorrow morning, not
        // the two mornings it missed.
        assert_eq!(next_after(&daily, at_local(2026, 10, 19, 12, 0)), Some(at_local(2026, 10, 20, 7, 10)));
        let weekly = sched("weekly", json!({ "weekly": "sun 03:20" }));
        assert_eq!(next_after(&weekly, at_local(2026, 10, 18, 3, 25)), Some(at_local(2026, 10, 25, 3, 20)));
        assert_eq!(next_after(&sched("at", json!({ "at": 5 })), now), None, "once is once");
        assert_eq!(next_after(&sched("file", json!({})), now), None);
    }

    /// A firing within the tick's slack says nothing; one the app slept
    /// through says how late and why, in either language.
    #[test]
    fn a_late_firing_says_so() {
        let due = at_local(2026, 10, 13, 23, 40);
        assert_eq!(late_note(0, due, false), "");
        assert_eq!(late_note(LATE, due, true), "", "right at the line is not late");
        let en = late_note(59 * 3_600_000, due, false);
        assert_eq!(en, " · late: 59h past 10-13 23:40 (the app or the machine was off)");
        let ko = late_note(LATE + 30_000, due, true);
        assert!(ko.starts_with(" · 늦게 실행됨: 예정 10-13 23:40에서 150s 지남"), "{ko}");
    }

    #[test]
    fn a_schedule_describes_itself() {
        let mut s = sched("daily", json!({ "daily": "07:10" }));
        s.next_at = Some(at_local(2026, 10, 17, 7, 10));
        assert!(describe(&s, false).starts_with("#1 daily 07:10 · next 10-17 07:10: m"), "{}", describe(&s, false));
        assert!(describe(&s, true).contains("매일 07:10"));
        let f = sched("file", json!({ "file": "/tmp/w.log", "contains": "끝" }));
        assert!(describe(&f, false).contains("file /tmp/w.log says \"끝\""), "{}", describe(&f, false));
        let mut c = sched("at", json!({ "at": 1 }));
        c.status = ScheduleStatus::Cancelled;
        assert!(describe(&c, false).ends_with("· cancelled: m"));
    }

    /// Quiet hours move a schedule past their end, but never past the
    /// schedule's own `until`.
    #[test]
    fn quiet_hours_do_not_carry_a_repetition_past_its_until() {
        let end = at_local(2026, 10, 18, 5, 0);
        let open = sched("every", json!({ "every": 3600 }));
        assert_eq!(past_quiet(&open, end), Some(end + 1000));
        let bounded = sched("every", json!({ "every": 3600, "until": at_local(2026, 10, 18, 4, 0) }));
        assert_eq!(past_quiet(&bounded, end), None, "04:00 is before the hours end");
        let roomy = sched("every", json!({ "every": 3600, "until": at_local(2026, 10, 18, 6, 0) }));
        assert_eq!(past_quiet(&roomy, end), Some(end + 1000));
        let once = sched("at", json!({ "at": 1 }));
        assert_eq!(past_quiet(&once, end), Some(end + 1000), "a one-off fires once it may");
    }

    /// A process schedule set before a reboot is met by the reboot: the
    /// process is gone, whoever has its number now.
    #[test]
    fn a_process_schedule_knows_its_boot() {
        let alive = |_: u32| true;
        assert_eq!(process_ended(42, "boot-one", "boot-one", alive), None, "same boot, still running");
        assert_eq!(process_ended(42, "boot-one", "boot-two", alive).as_deref(), Some("process 42 ended: the machine rebooted"), "another boot: gone, whatever pid 42 is now");
        assert_eq!(process_ended(42, "", "boot-two", alive), None, "set before boots were kept: the pid alone decides");
        assert_eq!(process_ended(42, "boot-one", "boot-one", |_| false).as_deref(), Some("process 42 ended"));
    }

    #[test]
    fn a_file_condition_is_met_by_a_line() {
        let dir = std::env::temp_dir().join(format!("divixi-sched-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.log");
        std::fs::write(&path, "step 1\nstep 2\n").unwrap();
        let s = sched("file", json!({ "file": path.to_string_lossy(), "contains": "DONE" }));
        assert_eq!(condition_met(&s), None);
        std::fs::write(&path, "step 1\nstep 2\nall DONE here\n").unwrap();
        assert_eq!(condition_met(&s).as_deref(), Some("all DONE here"));
        let p = sched("file", json!({ "file": path.to_string_lossy(), "contains": "", "pattern": "^step (\\d)$" }));
        assert_eq!(condition_met(&p).as_deref(), Some("step 1"));
        let gone = sched("file", json!({ "file": dir.join("nope").to_string_lossy(), "contains": "x" }));
        assert_eq!(condition_met(&gone), None, "a file that is not there is not a condition met");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing written to a watched file is skipped, however much arrives
    /// between two looks: each look goes on from where the last stopped.
    #[test]
    fn everything_written_to_a_watched_file_is_looked_at() {
        let dir = std::env::temp_dir().join(format!("divixi-sched-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.log");
        std::fs::write(&path, "").unwrap();
        let mut s = sched("file", json!({ "file": path.to_string_lossy(), "contains": "DONE" }));
        let first = file_says(&s).unwrap();
        assert_eq!((first.found.as_deref(), first.seen), (None, 0));
        s.seen = Some(first.seen);
        // Three MiB arrive before the next look, the marker among them.
        let mut filler = String::new();
        while filler.len() < (1 << 20) {
            filler.push_str("a line of filler that says nothing of note\n");
        }
        let whole = format!("{filler}all DONE here\n{filler}{filler}");
        std::fs::write(&path, &whole).unwrap();
        let second = file_says(&s).unwrap();
        assert_eq!(second.found.as_deref(), Some("all DONE here"));
        assert!(second.seen > (1 << 20) && second.seen < whole.len() as u64, "read up to the line that said so: {}", second.seen);
        // A file that was there before the schedule is read from its last
        // MiB, from a whole line; what comes after that is seen.
        let fresh = sched("file", json!({ "file": path.to_string_lossy(), "contains": "DONE" }));
        let late = file_says(&fresh).unwrap();
        assert_eq!((late.found.as_deref(), late.seen), (None, whole.len() as u64), "the marker is more than a MiB back");
        s.seen = Some(late.seen);
        std::fs::write(&path, format!("{whole}DONE again\n")).unwrap();
        assert_eq!(file_says(&s).unwrap().found.as_deref(), Some("DONE again"));
        // Cut shorter than what was read: read again from the top.
        std::fs::write(&path, "short\nDONE now\n").unwrap();
        assert_eq!(file_says(&s).unwrap().found.as_deref(), Some("DONE now"));
        // A line longer than a piece is read whole, so a marker on the
        // edge of a piece is not cut in two.
        let piece = 256 * 1024;
        let edge = format!("{}DONE edge\n{filler}", "x".repeat(piece - 2));
        std::fs::write(&path, &edge).unwrap();
        s.seen = Some(0);
        let long_line = file_says(&s).unwrap();
        assert!(long_line.found.as_deref().is_some_and(|l| l.ends_with("DONE edge")), "found on the long line");
        assert_eq!(long_line.seen as usize, piece - 2 + "DONE edge\n".len(), "read on past the piece, to that line's end");
        // Replaced by a file already longer than the mark: read from its top.
        std::fs::write(&path, "one\ntwo\n").unwrap();
        s.seen = None;
        s.seen_sum = None;
        let at = file_says(&s).unwrap();
        s.seen = Some(at.seen);
        s.seen_sum = at.sum;
        std::fs::write(&path, "DONE rotated\nplenty more than before\n").unwrap();
        assert_eq!(file_says(&s).unwrap().found.as_deref(), Some("DONE rotated"));
        s.seen_sum = None;
        // Written in two parts with a look between them: seen whole.
        std::fs::write(&path, "step 1\nall DO").unwrap();
        s.seen = Some(0);
        let part = file_says(&s).unwrap();
        assert_eq!((part.found.as_deref(), part.seen), (None, 13));
        s.seen = Some(part.seen);
        std::fs::write(&path, "step 1\nall DONE\n").unwrap();
        assert_eq!(file_says(&s).unwrap().found.as_deref(), Some("all DONE"));
        // More than one look reads at once is caught up with over looks.
        let mut long = String::new();
        while (long.len() as u64) < FILE_LOOK + (1 << 20) {
            long.push_str(&filler);
        }
        long.push_str("DONE last\n");
        std::fs::write(&path, &long).unwrap();
        s.seen = Some(0);
        let one = file_says(&s).unwrap();
        assert!(one.found.is_none() && one.seen >= FILE_LOOK - 1024 && one.seen < FILE_LOOK + 256 * 1024, "one look stops at its limit, on a line: {}", one.seen);
        s.seen = Some(one.seen);
        assert_eq!(file_says(&s).unwrap().found.as_deref(), Some("DONE last"), "the next look goes on from there");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
