//! Hours in which a track does not start work.
//!
//! The rules were scattered: a collection every Sunday at 03:20, another
//! every day at 07:10, nothing in production between three and five -- each
//! known to one worker, kept in its own script, and forgotten by the next.
//! A track keeps them in one place now. A worker turn that would start
//! inside a window is refused (or let through with a warning, as the
//! window says), and a schedule due inside one is moved to its end.
//!
//! Times are the machine's local time, as the rules are written.

use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Weekday};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// One window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// `daily`, `weekdays`, `weekend`, or days such as `sun` or `mon,wed`.
    #[serde(default = "daily")]
    pub days: String,
    /// `HH:MM`.
    pub from: String,
    /// `HH:MM`; at or before `from` means the window runs past midnight.
    pub to: String,
    /// What it is for, shown when it refuses.
    #[serde(default)]
    pub label: String,
    /// `block` (refuse to start) or `warn` (start, but say so).
    #[serde(default = "block")]
    pub mode: String,
}

fn daily() -> String {
    "daily".to_string()
}

fn block() -> String {
    "block".to_string()
}

impl Window {
    pub fn blocks(&self) -> bool {
        self.mode != "warn"
    }

    /// One line for a human: "Sun 03:20–04:30 weekly collection (block)".
    pub fn line(&self) -> String {
        let label = if self.label.is_empty() { String::new() } else { format!(" {}", self.label) };
        format!("{} {}–{}{label} ({})", self.days, self.from, self.to, self.mode)
    }
}

fn key(track: &str) -> String {
    format!("quiet:{track}")
}

fn parse_time(s: &str) -> Result<NaiveTime, String> {
    let (h, m) = s.trim().split_once(':').ok_or_else(|| format!("a time is HH:MM, not {s:?}"))?;
    let (h, m): (u32, u32) = (h.parse().map_err(|_| format!("a time is HH:MM, not {s:?}"))?, m.parse().map_err(|_| format!("a time is HH:MM, not {s:?}"))?);
    NaiveTime::from_hms_opt(h, m, 0).ok_or_else(|| format!("a time is HH:MM, not {s:?}"))
}

/// The weekdays a `days` word names.
fn weekdays(days: &str) -> Result<Vec<Weekday>, String> {
    let all = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];
    let d = days.trim().to_ascii_lowercase();
    Ok(match d.as_str() {
        "" | "daily" | "every day" | "매일" => all.to_vec(),
        "weekdays" | "평일" => all[..5].to_vec(),
        "weekend" | "주말" => all[5..].to_vec(),
        _ => {
            let mut out = Vec::new();
            for part in d.split([',', ' ']).filter(|p| !p.is_empty()) {
                out.push(weekday(part).ok_or_else(|| format!("not a day: {part:?}"))?);
            }
            out
        }
    })
}

/// A day's name, in English or Korean, to a weekday.
pub fn weekday(word: &str) -> Option<Weekday> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "mon" | "monday" | "월" | "월요일" => Weekday::Mon,
        "tue" | "tuesday" | "화" | "화요일" => Weekday::Tue,
        "wed" | "wednesday" | "수" | "수요일" => Weekday::Wed,
        "thu" | "thursday" | "목" | "목요일" => Weekday::Thu,
        "fri" | "friday" | "금" | "금요일" => Weekday::Fri,
        "sat" | "saturday" | "토" | "토요일" => Weekday::Sat,
        "sun" | "sunday" | "일" | "일요일" => Weekday::Sun,
        _ => return None,
    })
}

/// Check a list of windows as given.
pub fn check(windows: &[Window]) -> Result<Vec<Window>, String> {
    let mut out = Vec::new();
    for w in windows {
        parse_time(&w.from)?;
        parse_time(&w.to)?;
        weekdays(&w.days)?;
        if !matches!(w.mode.as_str(), "block" | "warn") {
            return Err(format!("mode is block or warn, not {:?}", w.mode));
        }
        out.push(Window { days: w.days.trim().to_string(), from: w.from.trim().to_string(), to: w.to.trim().to_string(), label: w.label.trim().to_string(), mode: w.mode.clone() });
    }
    Ok(out)
}

/// A track's windows as kept.
pub fn get(state: &AppState, track: &str) -> Vec<Window> {
    state.store.get_meta(&key(track)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Keep a track's windows (an empty list forgets them).
pub fn set(state: &AppState, track: &str, windows: &[Window]) -> Result<Vec<Window>, String> {
    let clean = check(windows)?;
    if clean.is_empty() {
        state.store.delete_meta(&key(track)).map_err(|e| e.to_string())?;
    } else {
        state.store.set_meta(&key(track), &serde_json::to_string(&clean).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(clean)
}

/// The window `now` (Unix milliseconds) falls in, if any, with when it
/// ends (Unix milliseconds). A blocking window wins over a warning one.
pub fn active(windows: &[Window], now_ms: i64) -> Option<(Window, i64)> {
    let now = Local.timestamp_millis_opt(now_ms).single()?;
    let mut found: Option<(Window, i64)> = None;
    for w in windows {
        let (Ok(from), Ok(to), Ok(days)) = (parse_time(&w.from), parse_time(&w.to), weekdays(&w.days)) else { continue };
        // The window may have started today, or yesterday if it runs past
        // midnight: look at both starts.
        for back in [0i64, 1] {
            let day = now.date_naive() - chrono::Duration::days(back);
            if !days.contains(&day.weekday()) {
                continue;
            }
            let start = day.and_time(from);
            let mut end = day.and_time(to);
            if to <= from {
                end += chrono::Duration::days(1);
            }
            let here = now.naive_local();
            if start <= here && here < end {
                let end_ms = Local.from_local_datetime(&end).single().map(|t| t.timestamp_millis()).unwrap_or(now_ms);
                let better = match &found {
                    Some((f, _)) => !f.blocks() && w.blocks(),
                    None => true,
                };
                if better {
                    found = Some((w.clone(), end_ms));
                }
            }
        }
    }
    found
}

/// "until 04:30" / "04:30까지", for a refusal.
pub fn until(end_ms: i64, ko: bool) -> String {
    let t = Local.timestamp_millis_opt(end_ms).single().map(|t| format!("{:02}:{:02}", t.hour(), t.minute())).unwrap_or_default();
    if ko {
        format!("{t}까지")
    } else {
        format!("until {t}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local.from_local_datetime(&NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()).single().unwrap().timestamp_millis()
    }

    fn w(days: &str, from: &str, to: &str, mode: &str) -> Window {
        Window { days: days.into(), from: from.into(), to: to.into(), label: "x".into(), mode: mode.into() }
    }

    #[test]
    fn windows_are_checked() {
        assert!(check(&[w("daily", "3:00", "05:00", "block")]).is_ok());
        assert!(check(&[w("sun", "03:20", "04:30", "warn")]).is_ok());
        assert!(check(&[w("mon,wed", "03:20", "04:30", "block")]).is_ok());
        assert!(check(&[w("daily", "25:00", "05:00", "block")]).is_err());
        assert!(check(&[w("someday", "03:00", "05:00", "block")]).is_err());
        assert!(check(&[w("daily", "03:00", "05:00", "maybe")]).is_err());
        assert_eq!(weekdays("평일").unwrap().len(), 5);
        assert_eq!(weekday("일"), Some(Weekday::Sun));
    }

    #[test]
    fn a_window_knows_its_days_its_end_and_midnight() {
        // 2026-10-18 is a Sunday.
        let sunday = [w("sun", "03:20", "04:30", "block")];
        let (hit, end) = active(&sunday, at(2026, 10, 18, 3, 25)).unwrap();
        assert_eq!(hit, sunday[0]);
        assert_eq!(end, at(2026, 10, 18, 4, 30));
        assert!(active(&sunday, at(2026, 10, 19, 3, 25)).is_none(), "Monday");
        assert!(active(&sunday, at(2026, 10, 18, 4, 30)).is_none(), "the end is not inside");
        assert!(active(&sunday, at(2026, 10, 18, 3, 19)).is_none());

        // Past midnight: 23:00–01:00 daily covers 00:30 on the next day.
        let night = [w("daily", "23:00", "01:00", "block")];
        let (_, end) = active(&night, at(2026, 10, 18, 0, 30)).unwrap();
        assert_eq!(end, at(2026, 10, 18, 1, 0));
        let (_, end) = active(&night, at(2026, 10, 18, 23, 30)).unwrap();
        assert_eq!(end, at(2026, 10, 19, 1, 0));

        // A blocking window wins over a warning one at the same time.
        let both = [w("daily", "07:00", "07:30", "warn"), w("daily", "07:05", "07:20", "block")];
        assert!(active(&both, at(2026, 10, 18, 7, 10)).unwrap().0.blocks());
        assert!(!active(&both, at(2026, 10, 18, 7, 2)).unwrap().0.blocks());
        assert!(until(at(2026, 10, 18, 4, 30), true).starts_with("04:30"));
        assert_eq!(until(at(2026, 10, 18, 4, 30), false), "until 04:30");
    }
}
