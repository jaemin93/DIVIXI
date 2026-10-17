//! Watching background work nobody else would.
//!
//! A worker starts a two-hour collection with `setsid nohup …`, ends its
//! turn, and that is the last anyone hears of it: the conductor has no way
//! to be told when it ends or dies, so it tries to come back by a schedule
//! inside its own session, which is gone by then. A job registered here is
//! looked at every half minute -- is the process alive, has the log moved,
//! does its newest line say it finished -- and the conductor is woken when
//! something changes: done, died, quiet for longer than it should be, or
//! moving again. After a reboot every job of the old boot is reported dead
//! with the last line it wrote.
//!
//! Registration comes from the worker that started the process (its one
//! tool, `watch_job`) or from the conductor on its behalf.

use std::path::Path;
use std::time::Duration;

use orchestra_store::{Job, JobStatus, NewJob};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::conductor::{deliver, Hand};
use crate::{AppHandle, AppState};

const TICK: Duration = Duration::from_secs(30);

/// The most of a log read in one look.
const CHUNK: u64 = 256 * 1024;

/// How long a job's log may be still before it is reported stalled, when
/// the registrant said nothing.
pub const DEFAULT_STALL_MINUTES: u32 = 30;

/// The event the window reloads a track's job list on.
pub const EVENT: &str = "jobs";

/// Register a job. `worker` is empty for the conductor's own.
pub fn watch(app: &AppHandle, track: &str, worker: &str, args: &Value) -> Result<Job, String> {
    let pid = match args.get("pid").and_then(Value::as_u64) {
        Some(0) | None => None,
        Some(p) => Some(u32::try_from(p).map_err(|_| format!("{p} is not a process id"))?),
    };
    let log = args.get("log").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let what = args.get("what").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let done_pattern = args.get("done_pattern").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let stall_minutes = args.get("stall_minutes").and_then(Value::as_u64).map(|m| m as u32).unwrap_or(DEFAULT_STALL_MINUTES).max(1);
    watchable(pid, &log, &done_pattern)?;
    if let Some(p) = pid {
        if !crate::boot::alive(p) {
            return Err(format!("no process {p} is running now: it may have ended already, or the pid is wrong"));
        }
    }
    if !log.is_empty() && !Path::new(&log).is_absolute() {
        return Err("the log path is absolute".to_string());
    }
    if !done_pattern.is_empty() {
        regex::Regex::new(&done_pattern).map_err(|e| format!("done_pattern is not a regular expression: {e}"))?;
    }
    let state = app.state::<AppState>();
    let job = state
        .store
        .add_job(&NewJob { track: track.to_string(), worker: worker.to_string(), what, pid, pid_start: pid.and_then(crate::boot::started), log, done_pattern, stall_minutes, boot: crate::boot::id() })
        .map_err(|e| e.to_string())?;
    tracing::info!(%track, %worker, job = job.id, pid = ?job.pid, log = %job.log, "watching a job");
    let _ = app.emit(EVENT, track);
    Ok(job)
}

/// Whether a registration gives the app a way to see the job end: a pid
/// (its exit), or a log with a pattern its finishing line matches. A log
/// alone could only ever be still or moving, never done, and the job
/// would stay watched after its process was long gone.
fn watchable(pid: Option<u32>, log: &str, done_pattern: &str) -> Result<(), String> {
    if pid.is_none() && log.is_empty() {
        return Err("give a pid, a log path, or both".to_string());
    }
    if pid.is_none() && done_pattern.is_empty() {
        return Err("a job watched by its log alone needs a done_pattern (the line the log ends with when it is done), or give its pid as well: a log by itself can only ever be still or moving, never finished".to_string());
    }
    Ok(())
}

/// Stop watching a job that is still watched.
pub fn unwatch(app: &AppHandle, id: i64) -> Result<Job, String> {
    let state = app.state::<AppState>();
    let job = state.store.job(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no job {id}"))?;
    if !job.status.is_open() {
        return Err(format!("job {id} is already {}", job.status.as_str()));
    }
    let ko = crate::ko(app);
    let job = state.store.set_job_status(id, JobStatus::Lost, if ko { "지켜보기를 그만둠" } else { "no longer watched" }).map_err(|e| e.to_string())?;
    let _ = app.emit(EVENT, &job.track);
    Ok(job)
}

/// A job as the tools and the window read it.
pub fn view(j: &Job, ko: bool) -> Value {
    json!({
        "id": j.id,
        "track": j.track,
        "worker": j.worker,
        "what": j.what,
        "pid": j.pid,
        "log": j.log,
        "done_pattern": j.done_pattern,
        "stall_minutes": j.stall_minutes,
        "status": j.status,
        "registered_at": j.registered_at,
        "changed_at": j.changed_at,
        "ended_at": j.ended_at,
        "last_line": j.last_line,
        "note": j.note,
        "line": describe(j, ko),
    })
}

/// One line on a job for a person or the conductor.
pub fn describe(j: &Job, ko: bool) -> String {
    let who = if j.worker.is_empty() { if ko { "지휘자" } else { "conductor" } } else { j.worker.as_str() };
    let what = if j.what.is_empty() { String::new() } else { format!(" {}", j.what) };
    let pid = j.pid.map(|p| format!(" pid {p}")).unwrap_or_default();
    let log = if j.log.is_empty() { String::new() } else { format!(" log {}", j.log) };
    let status = match (j.status, ko) {
        (JobStatus::Running, true) => "진행 중",
        (JobStatus::Running, false) => "running",
        (JobStatus::Stalled, true) => "멈춤",
        (JobStatus::Stalled, false) => "stalled",
        (JobStatus::Done, true) => "끝남",
        (JobStatus::Done, false) => "done",
        (JobStatus::Died, true) => "죽음",
        (JobStatus::Died, false) => "died",
        (JobStatus::Lost, true) => "알 수 없음",
        (JobStatus::Lost, false) => "lost",
    };
    let last = if j.last_line.is_empty() { String::new() } else { format!(" · {}: {}", if ko { "마지막 줄" } else { "last line" }, j.last_line) };
    let note = if j.note.is_empty() { String::new() } else { format!(" · {}", j.note) };
    format!("#{} {who}{what}{pid}{log} · {status}{last}{note}", j.id)
}

/// What a look at a job's log found.
struct Read {
    read_bytes: u64,
    /// The fingerprint at `read_bytes` (see `mark_sum`).
    sum: Option<i64>,
    last_line: String,
    moved: bool,
    /// The line the done pattern matched, when it did.
    done: Option<String>,
}

/// A fingerprint of a file as read up to `at`, kept with a read mark: the
/// file's identity on disk (device and inode on Unix, creation time on
/// Windows), its first 4 KiB and the 64 bytes just before `at`, FNV-1a.
/// A file replaced by another (renamed over, or rewritten in place) that
/// has already grown past the mark reads otherwise in one of them, and is
/// read again from its start instead of from the middle of a different
/// file. `None` at the start of a file, where there is nothing to compare.
pub(crate) fn mark_sum(f: &mut std::fs::File, at: u64) -> Option<i64> {
    use std::io::{Read as _, Seek, SeekFrom};
    if at == 0 {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let meta = f.metadata().ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        eat(&meta.dev().to_le_bytes());
        eat(&meta.ino().to_le_bytes());
    }
    #[cfg(windows)]
    {
        if let Some(made) = meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
            eat(&made.as_nanos().to_le_bytes());
        }
    }
    let _ = &meta;
    let head = at.min(4096);
    let mut buf = vec![0u8; head as usize];
    f.seek(SeekFrom::Start(0)).ok()?;
    f.read_exact(&mut buf).ok()?;
    eat(&buf);
    let back = at.min(64);
    let mut buf = vec![0u8; back as usize];
    f.seek(SeekFrom::Start(at - back)).ok()?;
    f.read_exact(&mut buf).ok()?;
    eat(&buf);
    Some(h as i64)
}

/// Where to read a file from: the mark when the file is still the one it
/// was taken in (no shorter, the same bytes before it), else its start.
pub(crate) fn resume_at(f: &mut std::fs::File, len: u64, mark: u64, sum: Option<i64>) -> u64 {
    if mark > len {
        return 0;
    }
    if sum.is_some() && mark_sum(f, mark) != sum {
        return 0;
    }
    mark
}

/// A line longer than a chunk is read on to its end, up to this much;
/// past that it is cut, and words on the cut are missed.
const LINE_LIMIT: u64 = 64 * 1024 * 1024;

/// Whole lines of a file that is being written, for looking for words in,
/// starting at the read mark `from`. Returns the text and the new mark.
///
/// Forward: one chunk, cut back to its last line end; a chunk with no
/// line end in it is read on to the end of its line (up to `LINE_LIMIT`),
/// so a long line is never split between two reads with the words on the
/// cut. At the end of the file what is there is taken, the line being
/// written included, so a last line with no line end is still seen.
/// Backward: when the mark is inside a line (that line was being written
/// at the last look), what was read of it before (up to a chunk) is put in
/// front again, so words written across two looks are seen whole.
pub(crate) fn read_lines(f: &mut std::fs::File, from: u64, len: u64) -> Option<(Vec<u8>, u64)> {
    use std::io::{Read as _, Seek, SeekFrom};
    let mut buf = Vec::new();
    if from > 0 && from <= len {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(from - 1)).ok()?;
        f.read_exact(&mut last).ok()?;
        if last[0] != b'\n' {
            let back = from.min(CHUNK);
            let mut before = vec![0u8; back as usize];
            f.seek(SeekFrom::Start(from - back)).ok()?;
            f.read_exact(&mut before).ok()?;
            let keep = before.iter().rposition(|b| *b == b'\n').map(|nl| nl + 1).unwrap_or(0);
            buf.extend_from_slice(&before[keep..]);
        }
    }
    let carried = buf.len();
    let take = len.saturating_sub(from).min(CHUNK);
    buf.resize(carried + take as usize, 0);
    f.seek(SeekFrom::Start(from)).ok()?;
    f.read_exact(&mut buf[carried..]).ok()?;
    let mut end = from + take;
    if end < len {
        match buf[carried..].iter().rposition(|b| *b == b'\n') {
            Some(nl) => {
                buf.truncate(carried + nl + 1);
                end = from + nl as u64 + 1;
            }
            None => {
                while end < len && end - from < LINE_LIMIT {
                    let more = (len - end).min(CHUNK);
                    let at = buf.len();
                    buf.resize(at + more as usize, 0);
                    f.read_exact(&mut buf[at..]).ok()?;
                    end += more;
                    if let Some(pos) = buf[at..].iter().position(|b| *b == b'\n') {
                        buf.truncate(at + pos + 1);
                        end = from + (buf.len() - carried) as u64;
                        break;
                    }
                }
            }
        }
    }
    Some((buf, end))
}

/// Read the log from where it was last read (see `read_lines`); a file
/// shorter than that, or one whose bytes before the mark are not the ones
/// read there (`sum`), was truncated or replaced and is read from its
/// start. `read_bytes` says where this read started.
fn read_chunk(job: &Job, read_bytes: u64, sum: Option<i64>) -> Option<Read> {
    if job.log.is_empty() {
        return None;
    }
    let mut f = std::fs::File::open(&job.log).ok()?;
    let len = f.metadata().ok()?.len();
    let from = resume_at(&mut f, len, read_bytes, sum);
    let (buf, end) = read_lines(&mut f, from, len)?;
    let sum = mark_sum(&mut f, end);
    let text = String::from_utf8_lossy(&buf);
    let moved = end > from || from < read_bytes;
    let last_line = text.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(str::to_string).unwrap_or_else(|| job.last_line.clone());
    let done = (!job.done_pattern.is_empty())
        .then(|| regex::Regex::new(&job.done_pattern).ok())
        .flatten()
        .and_then(|re| text.lines().find(|l| re.is_match(l)).map(|l| l.trim().to_string()));
    Some(Read { read_bytes: end, sum, last_line: last_line.chars().take(300).collect(), moved, done })
}

/// One look at the log: the next chunk after what was read.
fn read_log(job: &Job) -> Option<Read> {
    read_chunk(job, job.read_bytes, job.read_sum)
}

/// The whole of what is left of the log, chunk by chunk: for a process
/// that is gone, whose finishing line may be anywhere in what it wrote
/// since the last look, not only in the first chunk of it.
fn drain_log(job: &Job) -> Option<Read> {
    let mut whole = read_chunk(job, job.read_bytes, job.read_sum)?;
    // At most the log's size in chunks; a file growing under a dead
    // process is not expected, but a bound is cheaper than a surprise.
    for _ in 0..4096 {
        let next = read_chunk(job, whole.read_bytes, whole.sum)?;
        if next.read_bytes <= whole.read_bytes {
            break;
        }
        whole.read_bytes = next.read_bytes;
        whole.sum = next.sum;
        whole.moved |= next.moved;
        if !next.last_line.is_empty() {
            whole.last_line = next.last_line;
        }
        if whole.done.is_none() {
            whole.done = next.done;
        }
    }
    Some(whole)
}

/// Look at every open job once.
pub async fn tick(app: &AppHandle) {
    let open = app.state::<AppState>().jobs(None).unwrap_or_default();
    for job in open {
        look(app, job).await;
    }
}

impl AppState {
    fn jobs(&self, track: Option<&str>) -> anyhow::Result<Vec<Job>> {
        self.store.jobs(track, true)
    }
}

/// One look at one job, and what it changes.
async fn look(app: &AppHandle, job: Job) {
    let ko = crate::ko(app);
    let now = crate::now_ms();
    let state = app.state::<AppState>();
    let boot = crate::boot::id();

    // Its boot is over: so is the process. What it wrote before is read to
    // the end first, since it may have finished before the machine went.
    if !job.boot.is_empty() && job.boot != boot {
        let read = drain_log(&job);
        if let Some(r) = &read {
            let _ = state.store.job_progress(job.id, r.read_bytes, r.sum, &r.last_line, r.moved);
        }
        match read.as_ref().and_then(|r| r.done.clone()) {
            Some(line) => {
                let note = if ko { format!("로그에 끝 표시: {line} (그 뒤 기계가 재부팅됨)") } else { format!("the log said so: {line} (the machine rebooted after)") };
                end(app, job, JobStatus::Done, note).await;
            }
            None => {
                let note = if ko { "기계가 재부팅되어 죽음".to_string() } else { "the machine rebooted".to_string() };
                end(app, job, JobStatus::Died, note).await;
            }
        }
        return;
    }

    // A process that is gone has written its last: all of it is read
    // before it is said to have died without finishing.
    // The process registered, not whatever has its number now.
    let alive = job.pid.map(|p| crate::boot::same_alive(p, job.pid_start));
    let read = if alive == Some(false) { drain_log(&job) } else { read_log(&job) };
    if let Some(r) = &read {
        let _ = state.store.job_progress(job.id, r.read_bytes, r.sum, &r.last_line, r.moved);
    }

    if let Some(line) = read.as_ref().and_then(|r| r.done.clone()) {
        let note = if ko { format!("로그에 끝 표시: {line}") } else { format!("the log said so: {line}") };
        end(app, job, JobStatus::Done, note).await;
        return;
    }
    if alive == Some(false) {
        // The last line is already on the record (job_progress above); the
        // note says what the line does not.
        let note = if ko { "프로세스가 끝났지만 로그에 끝 표시가 없음".to_string() } else { "the process ended without the log saying it finished".to_string() };
        end(app, job, JobStatus::Died, note).await;
        return;
    }
    let moved = read.as_ref().is_some_and(|r| r.moved);
    let changed_at = if moved { now } else { job.changed_at };
    match job.status {
        JobStatus::Running if stalls(&job, moved, now - changed_at) => {
            let still = now - changed_at;
            let note = if ko { format!("{}분 동안 로그가 멈춤", still / 60_000) } else { format!("the log has not moved for {} minutes", still / 60_000) };
            // Unwatched while this look was under way: nobody is told.
            let Ok(fresh) = state.store.set_job_status(job.id, JobStatus::Stalled, &note) else { return };
            let _ = app.emit(EVENT, &job.track);
            tell(app, &fresh, true).await;
        }
        JobStatus::Stalled if moved => {
            // `job_progress` already put it back to running; say so --
            // unless it is not watched any more.
            let Some(fresh) = state.store.job(job.id).ok().flatten().filter(|j| j.status == JobStatus::Running) else { return };
            let _ = app.emit(EVENT, &fresh.track);
            tell(app, &fresh, false).await;
        }
        _ => {}
    }
}

/// Whether a running job counts as stalled: its log has a position to
/// watch, it did not move this time, and it has been still longer than
/// the job allows. A job watched by pid alone has no log to be still, so
/// it runs until the process ends.
fn stalls(job: &Job, moved: bool, still_ms: i64) -> bool {
    !job.log.is_empty() && !moved && still_ms > i64::from(job.stall_minutes) * 60_000
}

/// A job ended: record it, tell the conductor, and put a death on the bell.
async fn end(app: &AppHandle, job: Job, status: JobStatus, note: String) {
    let state = app.state::<AppState>();
    let fresh = match state.store.set_job_status(job.id, status, &note) {
        Ok(j) => j,
        Err(err) => {
            tracing::warn!(job = job.id, %err, "could not close a job");
            return;
        }
    };
    tracing::info!(job = fresh.id, track = %fresh.track, status = fresh.status.as_str(), %note, "job ended");
    let _ = app.emit(EVENT, &fresh.track);
    tell(app, &fresh, status == JobStatus::Died).await;
}

/// Wake the conductor about a job, and ring the bell when a person should
/// know too (it died, or it stalled).
async fn tell(app: &AppHandle, job: &Job, bell: bool) {
    let ko = crate::ko(app);
    let line = describe(job, ko);
    if bell {
        let title = match (job.status, ko) {
            (JobStatus::Died, true) => format!("백그라운드 작업이 죽었습니다 · #{}", job.id),
            (JobStatus::Died, false) => format!("A background job died · #{}", job.id),
            (JobStatus::Stalled, true) => format!("백그라운드 작업이 멈춘 듯합니다 · #{}", job.id),
            (JobStatus::Stalled, false) => format!("A background job looks stalled · #{}", job.id),
            (_, true) => format!("백그라운드 작업 · #{}", job.id),
            (_, false) => format!("Background job · #{}", job.id),
        };
        crate::add_notice(app, &job.track, "job", &title, &line, json!({ "job": job.id, "worker": job.worker }));
    }
    let advice = match (job.status, ko) {
        (JobStatus::Done, true) => "끝났습니다. 결과를 확인하고 다음 일을 정하세요; 사람에게 한두 문장으로 알리세요.",
        (JobStatus::Done, false) => "It finished. Read its result and decide what comes next; tell the human in a sentence or two.",
        (JobStatus::Died, true) => "끝 표시 없이 죽었습니다. 로그의 마지막 줄로 어디까지 됐는지 보고, 다시 돌릴지는 사람에게 묻거나 되풀이해도 안전한 일일 때만 스스로 정하세요. 자동으로 다시 돌리지 않았습니다.",
        (JobStatus::Died, false) => "It died without saying it finished. See how far it got from the log's last line; ask the human whether to run it again unless it is safe to repeat. Nothing was started again by itself.",
        (JobStatus::Stalled, true) => "로그가 멈췄습니다. 살아 있지만 진행이 없습니다: 작업자에게 확인시키거나 사람에게 알리세요.",
        (JobStatus::Stalled, false) => "Its log stopped moving. It is alive but not progressing: have the worker check, or tell the human.",
        (JobStatus::Running, true) => "다시 진행 중입니다.",
        (JobStatus::Running, false) => "It is moving again.",
        (JobStatus::Lost, true) => "더 지켜볼 수 없습니다.",
        (JobStatus::Lost, false) => "It can no longer be watched.",
    };
    let text = format!("[job] {line}\n\n({advice})");
    deliver(app.clone(), Hand { track: job.track.clone(), text, open: true, what: format!("job #{}", job.id), keep: true, worker: None, run: None }).await;
}

/// Jobs of a boot that is over, found when the app starts: each is dead,
/// and said so with the last line its log holds.
pub async fn after_restart(app: &AppHandle) {
    tick(app).await;
}

/// Look at the jobs every so often, for as long as the app runs.
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

    fn job(log: &str, pattern: &str, read_bytes: u64) -> Job {
        Job {
            id: 1,
            track: "tr001".into(),
            worker: "review-data".into(),
            what: "collect".into(),
            pid: None,
            pid_start: None,
            log: log.into(),
            done_pattern: pattern.into(),
            stall_minutes: 30,
            status: JobStatus::Running,
            boot: "b".into(),
            registered_at: 0,
            changed_at: 0,
            ended_at: None,
            read_bytes,
            read_sum: None,
            last_line: String::new(),
            note: String::new(),
        }
    }

    #[test]
    fn the_log_is_read_from_where_it_was_left() {
        let dir = std::env::temp_dir().join(format!("divixi-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.log");
        std::fs::write(&path, "1\n2\n").unwrap();
        let log = path.to_string_lossy().into_owned();
        let first = read_log(&job(&log, "^끝$", 0)).unwrap();
        assert_eq!((first.read_bytes, first.last_line.as_str(), first.moved, first.done.is_none()), (4, "2", true, true));
        let same = read_log(&job(&log, "^끝$", 4)).unwrap();
        assert!(!same.moved, "nothing new");
        assert_eq!(same.last_line, "", "no new line; the caller keeps the one it has");
        std::fs::write(&path, "1\n2\n3\n끝\n").unwrap();
        let more = read_log(&job(&log, "^끝$", 4)).unwrap();
        assert!(more.moved);
        assert_eq!(more.done.as_deref(), Some("끝"));
        assert_eq!(more.last_line, "끝");
        // Truncated under it: read again from the start.
        std::fs::write(&path, "x\n").unwrap();
        let again = read_log(&job(&log, "", 100)).unwrap();
        assert_eq!((again.read_bytes, again.last_line.as_str(), again.moved), (2, "x", true));
        assert!(read_log(&job(&dir.join("nope").to_string_lossy(), "", 0)).is_none());
        assert!(read_log(&job("", "", 0)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A finishing line is found whatever its length and however it was
    /// written: a line longer than a chunk with the words on the chunk's
    /// edge, and a line written in two parts with a look between them.
    #[test]
    fn a_finishing_line_is_seen_whole() {
        let dir = std::env::temp_dir().join(format!("divixi-jobs-long-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.log");
        let log = path.to_string_lossy().into_owned();
        let edge = format!("{}DONE edge\nmore\n{}", "x".repeat(CHUNK as usize - 2), "y\n".repeat(CHUNK as usize));
        std::fs::write(&path, &edge).unwrap();
        let one = read_log(&job(&log, "DONE", 0)).unwrap();
        assert!(one.done.as_deref().is_some_and(|l| l.ends_with("DONE edge")), "the words on the edge of the chunk");
        assert_eq!(one.read_bytes as usize, CHUNK as usize - 2 + "DONE edge\n".len(), "read to the end of that line and no further");
        // Written in two parts: "progress\nAL" at one look, "L DONE\n" by the next.
        std::fs::write(&path, "progress\nAL").unwrap();
        let before = read_log(&job(&log, "ALL DONE", 0)).unwrap();
        assert!(before.done.is_none());
        assert_eq!(before.read_bytes, 11, "what is there is read, the line being written included");
        assert_eq!(before.last_line, "AL");
        std::fs::write(&path, "progress\nALL DONE\n").unwrap();
        let after = read_log(&job(&log, "ALL DONE", before.read_bytes)).unwrap();
        assert_eq!(after.done.as_deref(), Some("ALL DONE"), "the line is seen whole across the two looks");
        assert!(after.moved);
        // Nothing new after a line being written: not moved, nothing found again.
        let still = read_log(&job(&log, "", 18)).unwrap();
        assert!(!still.moved);
        std::fs::write(&path, "progress\nhalf").unwrap();
        let half = read_log(&job(&log, "", 13)).unwrap();
        assert!(!half.moved, "the part read before is read again for words, but nothing moved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A log replaced by another that is already longer than what was read
    /// is read again from its start, not from the middle of a new file.
    #[test]
    fn a_replaced_log_is_read_from_its_start() {
        let dir = std::env::temp_dir().join(format!("divixi-jobs-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.log");
        let log = path.to_string_lossy().into_owned();
        std::fs::write(&path, "step 1\nstep 2\n").unwrap();
        let first = read_log(&job(&log, "^DONE$", 0)).unwrap();
        assert_eq!(first.read_bytes, 14);
        let mut seen = job(&log, "^DONE$", first.read_bytes);
        seen.read_sum = first.sum;
        // The same file grown on: read from the mark.
        std::fs::write(&path, "step 1\nstep 2\nstep 3\n").unwrap();
        let on = read_log(&seen).unwrap();
        assert_eq!((on.last_line.as_str(), on.read_bytes), ("step 3", 21), "read on from the mark");
        // Rewritten in place with the same bytes before the mark, its end
        // line near the top: the head says it is another file.
        let filler = "b".repeat(80);
        std::fs::write(&path, format!("step\n{filler}\n")).unwrap();
        let mark = read_log(&job(&log, "^DONE$", 0)).unwrap();
        let mut at = job(&log, "^DONE$", mark.read_bytes);
        at.read_sum = mark.sum;
        std::fs::write(&path, format!("DONE\n{filler}\nmore\n")).unwrap();
        assert_eq!(read_log(&at).unwrap().done.as_deref(), Some("DONE"), "the 64 bytes before the mark agree; the start does not");
        // Replaced by a new log already past the mark, its end line near the top.
        std::fs::write(&path, "fresh\nDONE\nand a good deal more after it\n").unwrap();
        let again = read_log(&seen).unwrap();
        assert_eq!(again.done.as_deref(), Some("DONE"), "read from the start of the new file");
        assert!(again.moved);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A process that wrote more than one chunk and then ended: one look
    /// sees the first chunk and would call it dead; the drain reads to the
    /// end and finds the finishing line. Chunks end on whole lines, so the
    /// line is never split between two.
    #[test]
    fn a_log_longer_than_a_chunk_is_read_to_its_end_before_a_death_is_declared() {
        let dir = std::env::temp_dir().join(format!("divixi-jobs-drain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.log");
        let mut text = String::new();
        let mut i = 0;
        while text.len() < 3 * CHUNK as usize {
            text.push_str(&format!("line {i} {}\n", "x".repeat(70)));
            i += 1;
        }
        text.push_str("끝\n");
        std::fs::write(&path, &text).unwrap();
        let log = path.to_string_lossy().into_owned();
        let one = read_log(&job(&log, "^끝$", 0)).unwrap();
        assert!(one.done.is_none() && one.read_bytes <= CHUNK, "one look stops within a chunk");
        assert!(text.as_bytes()[one.read_bytes as usize - 1] == b'\n', "and on a whole line");
        let all = drain_log(&job(&log, "^끝$", 0)).unwrap();
        assert_eq!(all.read_bytes as usize, text.len(), "read to the end");
        assert_eq!(all.done.as_deref(), Some("끝"));
        assert_eq!(all.last_line, "끝");
        assert!(all.moved);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A job is watched only when its end can be seen: a pid, or a log
    /// with the line that ends it.
    #[test]
    fn a_registration_needs_a_way_to_see_the_end() {
        assert!(watchable(Some(1), "", "").is_ok(), "a pid alone: its exit");
        assert!(watchable(Some(1), "/tmp/w.log", "").is_ok(), "a pid and a log");
        assert!(watchable(None, "/tmp/w.log", "^done$").is_ok(), "a log and its finishing line");
        assert!(watchable(None, "/tmp/w.log", "").is_err(), "a log alone can never be done");
        assert!(watchable(None, "", "^done$").is_err(), "nothing to watch");
    }

    /// Only a log can be still. A job watched by pid alone never moves a
    /// log, and is not stalled for it.
    #[test]
    fn a_job_without_a_log_does_not_stall() {
        let with_log = job("/tmp/w.log", "", 0);
        assert!(stalls(&with_log, false, 31 * 60_000));
        assert!(!stalls(&with_log, false, 29 * 60_000), "within its minutes");
        assert!(!stalls(&with_log, true, 31 * 60_000), "it moved");
        let mut pid_only = job("", "", 0);
        pid_only.pid = Some(777);
        assert!(!stalls(&pid_only, false, 10 * 60 * 60_000), "ten hours of nothing to watch is not a stall");
    }

    #[test]
    fn a_job_describes_itself() {
        let mut j = job("/tmp/w.log", "", 0);
        j.pid = Some(777);
        j.last_line = "step 3".into();
        assert_eq!(describe(&j, false), "#1 review-data collect pid 777 log /tmp/w.log · running · last line: step 3");
        j.status = JobStatus::Died;
        j.note = "the machine rebooted".into();
        assert!(describe(&j, true).contains("죽음") && describe(&j, true).ends_with("the machine rebooted"));
        j.worker.clear();
        assert!(describe(&j, false).starts_with("#1 conductor"));
    }
}
