//! What the app does when it comes back after going away.
//!
//! A machine rebooted at 23:35 on a Tuesday. The server came back when
//! someone connected on Friday morning, closed the run it found live as
//! "the app closed while the run was live", and nobody was told anything:
//! not what had stopped, not when, not that the worker could be started
//! again with its memory. Fifty-nine hours.
//!
//! So the app now keeps two facts while it runs -- which boot it belongs
//! to (`boot_id`) and when it was last alive (`alive_at`, every twenty
//! seconds) -- and reads them when it starts. A different boot means the
//! machine rebooted; the same boot with runs still live means the app
//! itself went without closing them. Either way each such run is closed
//! for what it is (`interrupted`, not `failed`), ended when the app was
//! last seen rather than when it noticed, and told apart from one whose
//! agent process is still alive (`detached`, watched until it is gone).
//! Then the people who need to know are told: a notice on the bell for
//! the human, a turn for each track's conductor saying what was cut and
//! how to start it again, and runs the conductor had marked safe to
//! repeat are started again by themselves.

use std::time::Duration;

use orchestra_core::{AgentEvent, RunStatus};
use orchestra_store::{ClosedRun, Interruption, RunSummary};
use serde_json::json;
use tauri::Manager;

use crate::conductor::{deliver, Hand};
use crate::{AppHandle, AppState};

/// Meta key of the boot the app last ran under.
pub const BOOT_KEY: &str = "boot_id";
/// Meta key of when the app was last known alive (Unix milliseconds).
pub const ALIVE_KEY: &str = "alive_at";

/// Comebacks found but not yet told: kept until their notices and the
/// conductor's summary have gone out, so an app that goes down again
/// before then (it waits a few seconds for the agents) still tells them
/// next time. The runs they name are no longer live by then, and nothing
/// else would find them again.
const PENDING_KEY: &str = "comeback_pending";

/// How often the app says it is alive.
const HEARTBEAT: Duration = Duration::from_secs(20);

/// How often detached runs are looked at.
const DETACHED_CHECK: Duration = Duration::from_secs(30);

/// Something that tells this boot from the one before. Linux has a random
/// id per boot; macOS and Windows give the boot time instead, to the
/// second, which serves the same purpose. Empty when nothing can be read,
/// which reads as "no reboot seen".
pub fn id() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(read_id).clone()
}

/// Overrides the boot id, to try a reboot without one (scripts/ checks).
const BOOT_ENV: &str = "DIVIXI_BOOT_ID";

fn read_id() -> String {
    if let Some(given) = std::env::var_os(BOOT_ENV).map(|v| v.to_string_lossy().trim().to_string()).filter(|v| !v.is_empty()) {
        return given;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
            return s.trim().to_string();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl").args(["-n", "kern.boottime"]).output() {
            // "{ sec = 1760000000, usec = 123 } Mon Oct 13 23:35:14 2026"
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(sec) = text.split("sec = ").nth(1).and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next()) {
                if !sec.is_empty() {
                    return format!("macos-boot-{sec}");
                }
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows counts its boots in the registry. The boot time worked
        // out from the clock and the uptime would change with the clock,
        // and a clock set right between two runs of the app would then
        // read as a reboot, ending runs whose agents are still alive.
        use winreg::enums::HKEY_LOCAL_MACHINE;
        const KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters";
        if let Ok(n) = winreg::RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(KEY).and_then(|k| k.get_value::<u32, _>("BootId")) {
            return format!("windows-boot-{n}");
        }
        // Without it: the boot time to the nearest ten seconds, from how
        // long the system has been up.
        let up_ms: u64 = unsafe { windows_tick() };
        let now = crate::now_ms() as u64;
        return format!("windows-uptime-{}", (now.saturating_sub(up_ms)) / 10_000);
    }
    #[allow(unreachable_code)]
    String::new()
}

#[cfg(windows)]
unsafe fn windows_tick() -> u64 {
    extern "system" {
        fn GetTickCount64() -> u64;
    }
    GetTickCount64()
}

/// Whether a process with this id is alive. On Unix `kill(pid, 0)`: it is
/// alive when the signal would be delivered, or when it is somebody
/// else's (EPERM). On Windows a handle to it is asked for and its exit
/// code read: alive while that says "still active", or when the handle is
/// refused (the process is there and somebody else's).
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 delivers nothing; kill cannot violate memory safety.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        // SAFETY: the handle asked for allows reading the process's state
        // and nothing else, and is closed before this returns.
        unsafe { windows_alive(pid) }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

#[cfg(windows)]
unsafe fn windows_alive(pid: u32) -> bool {
    use std::ffi::c_void;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_ACCESS_DENIED: u32 = 5;
    const STILL_ACTIVE: u32 = 259;
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return GetLastError() == ERROR_ACCESS_DENIED;
    }
    let mut code: u32 = 0;
    let read = GetExitCodeProcess(handle, &mut code) != 0;
    CloseHandle(handle);
    // A process that chose 259 as its exit code reads as alive here, as it
    // does to Windows itself.
    read && code == STILL_ACTIVE
}

/// How the app found things when it started.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Comeback {
    /// The machine rebooted since the app last ran.
    pub rebooted: bool,
    /// When the app was last known alive, if it had run before.
    pub last_alive: Option<i64>,
    /// What became of the runs it had live, by track.
    pub closed: Vec<ClosedRun>,
    /// Routine runs it had live, now failed.
    pub routines: Vec<orchestra_store::ClosedRoutineRun>,
    /// What happened, for a human, in the interface language.
    pub why: String,
    /// The parts of it told already (`notice:…`, `resume:…`, `summary:…`),
    /// kept with it so a start that replays it does each part once.
    #[serde(default)]
    pub done: Vec<String>,
    /// When it was found (unix ms): what its restart summaries are known by
    /// in the conductor's waiting list, so a replay does not add one twice.
    #[serde(default)]
    pub at: i64,
}

/// Close what the last run of the app left live, now that this one knows
/// what happened. Called once, right after the store opens; the boot id
/// and the heartbeat are written here too, so the next start can tell.
pub fn come_back(state: &AppState, ko: bool) -> Comeback {
    let store = &state.store;
    let before = store.get_meta(BOOT_KEY).ok().flatten().unwrap_or_default();
    let last_alive = store.get_meta(ALIVE_KEY).ok().flatten().and_then(|v| v.trim().parse::<i64>().ok());
    let now_id = id();
    let rebooted = !before.is_empty() && !now_id.is_empty() && before != now_id;
    let seen = last_alive.map(crate::when).unwrap_or_else(|| if ko { "모름".into() } else { "unknown".into() });
    let back = crate::when(crate::now_ms());
    let why = match (rebooted, ko) {
        (true, true) => format!("기계가 재부팅됐습니다 (마지막 확인 {seen}, 다시 켜짐 {back})"),
        (true, false) => format!("the machine rebooted (last seen alive {seen}; back {back})"),
        (false, true) => format!("DIVIXI 가 정상 종료 없이 꺼졌습니다 (마지막 확인 {seen}, 다시 켜짐 {back})"),
        (false, false) => format!("DIVIXI went away without closing its runs (last seen alive {seen}; back {back})"),
    };
    // After a reboot no process of the old boot is left, whatever a pid
    // says: the number may have been given to something else since, and
    // taking that for the agent would hold the run as detached until a
    // stranger exits. The store looks at runs detached on an earlier
    // comeback with the same check, so a reboot ends those too.
    let still_there = agent_check(rebooted);
    let (closed, routines) = match store.close_live_runs_and_routines(&Interruption { why: why.clone(), at: last_alive, alive: still_there }) {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(%err, "could not close the runs a previous process left live");
            (Vec::new(), Vec::new())
        }
    };
    if let Err(err) = store.set_meta(BOOT_KEY, &now_id) {
        tracing::warn!(%err, "could not keep the boot id");
    }
    beat(state);
    if rebooted || !closed.is_empty() {
        tracing::warn!(rebooted, last_alive = ?last_alive, cut = closed.len(), "{why}");
    }
    Comeback { rebooted, last_alive, closed, routines, why, done: Vec::new(), at: crate::now_ms() }
}

/// How to ask whether a lost run's agent is still there: by pid on the
/// same boot, never after a reboot (every process of the old boot is gone,
/// and its number may be somebody else's now).
fn agent_check(rebooted: bool) -> &'static dyn Fn(u32, Option<u64>) -> bool {
    if rebooted {
        &|_, _| false
    } else {
        &agent_alive
    }
}

/// Whether an agent the app started is still there: its own process, or
/// on Unix anything left in the process group it led (`orchestra_acp`
/// starts each agent as the leader of a group of its own). The adapter
/// can exit when its pipes break and leave the agent it started running
/// in that group; the agent is what matters. On Windows the agent's job
/// object goes with the app, and the tree with it.
///
/// With the start time kept for it, a live process of that number that
/// started at another time is somebody else's, given the number after the
/// agent ended -- and a number is not given out while a group of that id
/// still has members, so nothing of the agent's is left either.
pub fn agent_alive(pid: u32, start: Option<u64>) -> bool {
    if alive(pid) {
        return match (start, started(pid)) {
            (Some(kept), Some(now)) => kept == now,
            _ => true,
        };
    }
    #[cfg(unix)]
    {
        if pid <= 1 {
            return false;
        }
        // SAFETY: signal 0 delivers nothing; killpg cannot violate memory safety.
        let rc = unsafe { libc::killpg(pid as libc::pid_t, 0) };
        return rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    #[allow(unreachable_code)]
    false
}

/// When a process started, as the OS keeps it: what tells the process a
/// number was given to from one given it later. Clock ticks since boot on
/// Linux, microseconds since the epoch on macOS, the creation FILETIME on
/// Windows; none where it cannot be read.
pub fn started(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // Field 22 of /proc/<pid>/stat; the name in field 2 may hold
        // spaces and parentheses, so the fields are counted after its last `)`.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = &stat[stat.rfind(')')? + 1..];
        return rest.split_whitespace().nth(19)?.parse().ok();
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: proc_pidinfo writes at most `size` bytes into `info`, a
        // plain C struct for which all-zero is a valid value.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
        if n != size {
            return None;
        }
        return Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec);
    }
    #[cfg(windows)]
    {
        // SAFETY: the handle allows reading the process's times and is
        // closed before this returns.
        return unsafe { windows_started(pid) };
    }
    #[allow(unreachable_code)]
    {
        let _ = pid;
        None
    }
}

#[cfg(windows)]
unsafe fn windows_started(pid: u32) -> Option<u64> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetProcessTimes(process: *mut c_void, creation: *mut FileTime, exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None;
    }
    let (mut creation, mut exit, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
    let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
    CloseHandle(handle);
    ok.then(|| (u64::from(creation.high) << 32) | u64::from(creation.low))
}

/// Whether a process somebody named (a watched job, a process schedule)
/// is still the one named: alive, and, with a start time kept for it,
/// started then. Unlike `agent_alive` its group is not asked: such a
/// process is not one the app started as a group leader.
pub fn same_alive(pid: u32, start: Option<u64>) -> bool {
    alive(pid)
        && match (start, started(pid)) {
            (Some(kept), Some(now)) => kept == now,
            _ => true,
        }
}

/// A process to keep with a run: its id, and when it started.
pub fn identity(pid: Option<u32>) -> (Option<u32>, Option<u64>) {
    (pid, pid.and_then(started))
}

/// How long a run has been going, for the event that ends it.
fn elapsed_ms(started_at: i64, now: i64) -> u64 {
    (now - started_at).max(0) as u64
}

fn beat(state: &AppState) {
    if let Err(err) = state.store.set_meta(ALIVE_KEY, &crate::now_ms().to_string()) {
        tracing::warn!(%err, "could not write the heartbeat");
    }
}

/// Say the app is alive every so often, for as long as it runs.
pub fn start_heartbeat(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(HEARTBEAT).await;
            beat(&app.state::<AppState>());
        }
    });
}

/// Whether a run's key is a document's or a plan's (`artifact:<id>`,
/// `routine:<id>`) rather than a track's: such an agent has no conductor,
/// and its notice has no track to open.
fn not_a_track(key: &str) -> bool {
    key.starts_with("artifact:") || key.starts_with("routine:")
}

/// The name a person knows a document or a plan by, for its run's key.
fn owner_name(store: &orchestra_store::Store, key: &str) -> String {
    if let Some(id) = key.strip_prefix("artifact:") {
        return store.artifact(id).ok().flatten().map(|(a, _)| a.title).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| id.to_string());
    }
    if let Some(id) = key.strip_prefix("routine:") {
        return store.routine(id).ok().flatten().map(|r| r.name).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.to_string());
    }
    key.to_string()
}

/// One line on a closed run for a person or the conductor.
fn line(r: &RunSummary, ko: bool) -> String {
    let started = crate::when(r.started_at);
    let seen = r.ended_at.map(crate::when).unwrap_or_else(|| if ko { "모름".into() } else { "unknown".into() });
    let task: String = r.prompt.lines().next().unwrap_or("").chars().take(80).collect();
    // The conductor reads this too: its own turn is named as such.
    let who = match (r.session == crate::conductor::CONDUCTOR_SESSION, ko) {
        (true, true) => "지휘자(당신 자신의 턴)".to_string(),
        (true, false) => "conductor (your own turn)".to_string(),
        (false, _) => r.session.clone(),
    };
    match (r.status, ko) {
        (RunStatus::Detached, true) => format!("{} {who} — {started} 시작 · 에이전트 프로세스(pid {})가 아직 살아 있음: 끝나면 알립니다 · {task}", r.id, r.pid.unwrap_or(0)),
        (RunStatus::Detached, false) => format!("{} {who} — started {started} · its agent (pid {}) is still alive: you will be told when it ends · {task}", r.id, r.pid.unwrap_or(0)),
        (_, true) => format!("{} {who} — {started} 시작, 마지막 확인 {seen}{} · {task}", r.id, if r.repeatable { " · 되풀이 안전으로 표시됨" } else { "" }),
        (_, false) => format!("{} {who} — started {started}, last seen {seen}{} · {task}", r.id, if r.repeatable { " · marked safe to repeat" } else { "" }),
    }
}

/// Tell the people who need to know what the comeback found: a notice on
/// the bell per track, a turn for the track's conductor, and runs marked
/// safe to repeat started again. Then watch the detached runs.
pub fn keep_pending(store: &orchestra_store::Store, current: Comeback) -> Vec<Comeback> {
    let mut pending: Vec<Comeback> = store.get_meta(PENDING_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if !current.closed.is_empty() || !current.routines.is_empty() {
        pending.push(current);
    }
    keep_left(store, &pending);
    pending
}

/// Keep what is still to be told (none: forget the key).
fn keep_left(store: &orchestra_store::Store, left: &[Comeback]) {
    let kept = if left.is_empty() {
        store.delete_meta(PENDING_KEY)
    } else {
        match serde_json::to_string(left) {
            Ok(s) => store.set_meta(PENDING_KEY, &s),
            Err(err) => {
                tracing::warn!(%err, "could not keep the comebacks still to tell");
                return;
            }
        }
    };
    if let Err(err) = kept {
        tracing::warn!(%err, "could not keep the comebacks still to tell");
    }
}

/// Tell every comeback still to be told, oldest first, each let go once
/// it has been; then watch the runs left detached.
pub async fn tell(app: AppHandle, mut pending: Vec<Comeback>) {
    while !pending.is_empty() {
        tell_one(&app, &mut pending).await;
        pending.remove(0);
        keep_left(&app.state::<AppState>().store, &pending);
    }
    watch_detached(app);
}

/// Whether this part of a comeback was told already (see `Comeback::done`).
fn told(done: &[String], key: &str) -> bool {
    done.iter().any(|d| d == key)
}

/// How a repeatable run's restart went, as kept with its comeback: the
/// new run, or why it could not start.
fn resume_key(run: &str, outcome: &Result<String, String>) -> String {
    match outcome {
        Ok(new) => format!("resume:{run}=ok:{new}"),
        Err(why) => format!("resume:{run}=err:{why}"),
    }
}

/// The restart of `run` kept with its comeback, if it was tried before.
fn resumed_before(done: &[String], run: &str) -> Option<Result<String, String>> {
    let head = format!("resume:{run}=");
    let rest = done.iter().find_map(|d| d.strip_prefix(&head))?;
    Some(match rest.strip_prefix("ok:") {
        Some(new) => Ok(new.to_string()),
        None => Err(rest.strip_prefix("err:").unwrap_or(rest).to_string()),
    })
}

/// Keep that one part of the first pending comeback has been told.
fn mark(app: &AppHandle, pending: &mut [Comeback], key: String) {
    pending[0].done.push(key);
    keep_left(&app.state::<AppState>().store, pending);
}

/// Tell the first pending comeback. Each part (a notice, a restart, a
/// summary) is kept as told as it goes, so a start that replays the
/// comeback after going down part-way does each part once. A notice is
/// marked after it is written (a crash between the two repeats it, the
/// lesser harm); a summary is marked before it is handed on, since the
/// conductor's turn can take minutes and a crash in it is reported on its
/// own as that turn cut.
async fn tell_one(app: &AppHandle, pending: &mut [Comeback]) {
    let ko = crate::ko(app);
    let comeback = pending[0].clone();
    let done = comeback.done.clone();
    // A routine has no conductor: a run of one that was cut is said on the
    // bell, and so is an agent of it that went on, which the routine waits
    // for before it runs again -- as the store has it now, not as it was
    // when this comeback was first found.
    for r in &comeback.routines {
        let key = format!("notice:{}", r.run);
        if told(&done, &key) {
            continue;
        }
        let (name, left) = {
            let store = &app.state::<AppState>().store;
            let left = match store.routine_left_running(&r.routine) {
                Ok(Some((run, pid, start))) if run == r.run && agent_alive(pid, start) => Some(pid),
                _ => None,
            };
            (owner_name(store, &format!("routine:{}", r.routine)), left)
        };
        let title = if ko { format!("끊긴 루틴 실행 · {name}") } else { format!("A routine's run was cut short · {name}") };
        let left = match (left, ko) {
            (Some(pid), true) => format!("\n에이전트(pid {pid})가 아직 돌고 있어, 그것이 끝날 때까지 이 루틴은 다시 시작되지 않습니다."),
            (Some(pid), false) => format!("\nIts agent (pid {pid}) is still running; the routine will not start again until it ends."),
            (None, _) => String::new(),
        };
        crate::add_notice(app, "", "interrupted", &title, &format!("{} · {}{left}", comeback.why, r.run), json!({ "routine": r.routine, "run": r.run }));
        mark(app, pending, key);
    }
    let mut by_track: std::collections::BTreeMap<String, Vec<ClosedRun>> = std::collections::BTreeMap::new();
    for c in comeback.closed {
        if not_a_track(&c.run.track) {
            // A document's or a plan's agent has no conductor to tell: the
            // person hears of it on the bell, and opens it again from there.
            let key = format!("notice:{}", c.run.id);
            if told(&done, &key) {
                continue;
            }
            let name = owner_name(&app.state::<AppState>().store, &c.run.track);
            let title = if ko { format!("끊긴 에이전트 턴 · {name}") } else { format!("An agent's turn was cut short · {name}") };
            let body = format!("{}\n{}", comeback.why, line(&c.run, ko));
            crate::add_notice(app, "", "interrupted", &title, &body, json!({ "runs": [c.run.id], "of": c.run.track }));
            mark(app, pending, key);
            continue;
        }
        by_track.entry(c.run.track.clone()).or_default().push(c);
    }
    for (track, runs) in by_track {
        let name = app.state::<AppState>().store.track(&track).ok().flatten().map(|t| t.name).unwrap_or_else(|| track.clone());
        let lines: Vec<String> = runs.iter().map(|c| line(&c.run, ko)).collect();
        let cut: Vec<&ClosedRun> = runs.iter().filter(|c| !c.detached).collect();
        let key = format!("notice:{track}");
        if !told(&done, &key) {
            let title = if ko { format!("끊긴 작업 {}개 · {name}", runs.len()) } else { format!("{} run(s) cut short · {name}", runs.len()) };
            let body = format!("{}\n{}", comeback.why, lines.join("\n"));
            crate::add_notice(
                app,
                &track,
                "interrupted",
                &title,
                &body,
                json!({ "runs": runs.iter().map(|c| c.run.id.clone()).collect::<Vec<_>>(), "rebooted": comeback.rebooted, "last_alive": comeback.last_alive }),
            );
            mark(app, pending, key);
        }

        // Runs the conductor said were safe to repeat: started again,
        // each with its memory and where it had got to -- once, however
        // often this comeback is replayed. The rest wait for a person or
        // the conductor.
        let mut restarted: Vec<String> = Vec::new();
        let mut not_restarted: Vec<String> = Vec::new();
        for c in &cut {
            if !c.run.repeatable {
                continue;
            }
            let outcome = match resumed_before(&pending[0].done, &c.run.id) {
                Some(before) => before,
                None => {
                    let outcome = match crate::conductor::resume_run(app.clone(), track.clone(), c.run.id.clone(), true).await {
                        Ok(v) => Ok(v.get("run").and_then(|r| r.as_str()).unwrap_or("?").to_string()),
                        Err(err) => {
                            tracing::warn!(%track, run = %c.run.id, %err, "could not start a repeatable run again");
                            Err(err)
                        }
                    };
                    mark(app, pending, resume_key(&c.run.id, &outcome));
                    outcome
                }
            };
            match outcome {
                Ok(new) => restarted.push(format!("{} → {new}", c.run.id)),
                Err(err) => not_restarted.push(format!("{}: {err}", c.run.id)),
            }
        }
        let key = format!("summary:{track}");
        if told(&done, &key) {
            continue;
        }
        let text = restart_text(&comeback.why, &lines, &restarted, &not_restarted, cut.len(), ko);
        // Into the conductor's waiting list first (once, by this comeback's
        // id), then marked, then handed on: a crash before the mark finds it
        // waiting already, one after leaves it waiting to be handed on.
        let hand = Hand { track: track.clone(), text, open: true, what: if ko { "재시작 요약".into() } else { "restart summary".into() }, keep: true, worker: None, run: None };
        crate::conductor::park_once(app, &format!("restart-{}-{track}", comeback.at), hand).await;
        mark(app, pending, key);
        crate::conductor::flush_parked(app.clone(), track.clone()).await;
    }
}

/// The turn a conductor gets after a comeback. `restarted` are the turns
/// started again by themselves; `not_restarted` the ones that were meant
/// to be and could not (quiet hours, an agent that would not open), each
/// with why -- said apart, so a failure is never read as a start.
fn restart_text(why: &str, lines: &[String], restarted: &[String], not_restarted: &[String], cut: usize, ko: bool) -> String {
    let mut out = String::new();
    if ko {
        out.push_str(&format!("[divixi-restart] {why}\n\n끊긴 턴:\n"));
        for l in lines {
            out.push_str(&format!("- {l}\n"));
        }
        if !restarted.is_empty() {
            out.push_str(&format!("\n되풀이해도 안전으로 표시된 턴은 같은 기억으로 자동으로 다시 돌렸습니다: {}\n", restarted.join(", ")));
        }
        if !not_restarted.is_empty() {
            out.push_str(&format!("\n되풀이해도 안전으로 표시됐지만 자동으로 다시 돌리지 못한 턴: {}. 이유가 풀리면 `resume_run` 으로 직접 돌리거나 사람에게 알리세요.\n", not_restarted.join("; ")));
        }
        if cut > restarted.len() + not_restarted.len() {
            out.push_str("\n나머지는 자동으로 다시 돌리지 않았습니다 (저장소나 운영 DB에 쓰던 일은 두 번 돌 수 있습니다). 각 run 이 어디까지 했는지는 read_report 로 읽을 수 있습니다. 같은 기억으로 이어서 돌리려면 `resume_run(run)` 을 부르세요: 끊긴 시점의 마지막 출력과 도구 호출을 작업자에게 함께 넘깁니다. 두 번 돌면 안 되는 일이면 먼저 사람에게 request_decision 으로 물으세요.\n");
        }
        out.push_str("\n사람에게 무엇이 끊겼고 무엇을 이어야 하는지 한두 문단으로 알리세요. 예약(list_schedules)과 지켜보던 작업(list_jobs)도 확인하세요.");
    } else {
        out.push_str(&format!("[divixi-restart] {why}\n\nTurns cut short:\n"));
        for l in lines {
            out.push_str(&format!("- {l}\n"));
        }
        if !restarted.is_empty() {
            out.push_str(&format!("\nTurns marked safe to repeat were started again by themselves, with their memory: {}\n", restarted.join(", ")));
        }
        if !not_restarted.is_empty() {
            out.push_str(&format!("\nTurns marked safe to repeat that could NOT be started again: {}. Start them with `resume_run` once the reason is gone, or tell the human.\n", not_restarted.join("; ")));
        }
        if cut > restarted.len() + not_restarted.len() {
            out.push_str("\nThe rest were NOT started again (a turn that writes to a repository or a database could run twice). read_report says how far each got. To carry one on with its memory, call `resume_run(run)`: the worker is handed its last output and tool calls from before the cut. If running it twice would be harmful, ask the human with request_decision first.\n");
        }
        out.push_str("\nTell the human in a paragraph or two what was cut and what should carry on. Check your schedules (list_schedules) and watched jobs (list_jobs) too.");
    }
    out
}

/// Watch runs whose agent went on alone: when its process ends, the run
/// ends as interrupted, the record says when, and the conductor is told it
/// may be started again now.
pub fn watch_detached(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let detached = app.state::<AppState>().store.detached_runs().unwrap_or_default();
            if detached.is_empty() {
                return;
            }
            for r in detached {
                let Some(pid) = r.pid else { continue };
                if agent_alive(pid, r.pid_start) {
                    continue;
                }
                let ko = crate::ko(&app);
                let when = crate::when(crate::now_ms());
                let why = if ko {
                    format!("앱이 꺼진 뒤에도 에이전트(pid {pid})가 혼자 돌다가 {when}에 끝났습니다; 무엇을 했는지는 앱이 보지 못했습니다")
                } else {
                    format!("its agent (pid {pid}) went on alone after the app closed and ended at {when}; what it did was not seen by the app")
                };
                let state = app.state::<AppState>();
                // Ended now, after however long it ran on its own: a detached
                // run has no duration yet, so the event's time is measured
                // from its start.
                if let Err(err) = state.store.append(&r.id, elapsed_ms(r.started_at, crate::now_ms()), &AgentEvent::Interrupted { why: why.clone() }) {
                    tracing::warn!(run = %r.id, %err, "could not end a detached run");
                    continue;
                }
                tracing::info!(run = %r.id, pid, "a detached run's agent ended");
                let apart = not_a_track(&r.track);
                let who = if apart { owner_name(&state.store, &r.track) } else { r.session.clone() };
                let title = if ko { format!("떨어져 돌던 에이전트가 끝났습니다 · {who}") } else { format!("A detached agent ended · {who}") };
                crate::add_notice(&app, if apart { "" } else { &r.track }, "interrupted", &title, &format!("{} {who} — {why}", r.id), json!({ "runs": [r.id], "of": r.track }));
                // A document's or a plan's agent has no conductor to tell.
                if apart {
                    continue;
                }
                let own = r.session == crate::conductor::CONDUCTOR_SESSION;
                let text = match (own, ko) {
                    (true, true) => format!("[divixi-restart] 당신의 이전 턴 {} — {why}\n\n그 턴이 무엇을 했는지 앱은 모릅니다. 저장소와 작업자 상태를 먼저 확인한 뒤 이어 가세요. 그동안 기다리던 예약·작업 알림이 이어서 옵니다.", r.id),
                    (true, false) => format!("[divixi-restart] your own earlier turn {} — {why}\n\nThe app does not know what that turn did. Check the repository and the workers before going on. What waited meanwhile (schedules, job news) comes next.", r.id),
                    (false, true) => format!("[divixi-restart] {} {} — {why}\n\n이제 같은 기억으로 다시 돌릴 수 있습니다 (`resume_run(\"{}\")`). 두 번 돌면 안 되는 일이면 먼저 사람에게 물으세요.", r.id, r.session, r.id),
                    (false, false) => format!("[divixi-restart] {} {} — {why}\n\nIt may be started again with its memory now (`resume_run(\"{}\")`). If running it twice would be harmful, ask the human first.", r.id, r.session, r.id),
                };
                deliver(app.clone(), Hand { track: r.track.clone(), text, open: true, what: format!("detached {}", r.id), keep: true, worker: None, run: None }).await;
            }
            tokio::time::sleep(DETACHED_CHECK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_boot_has_an_id_and_this_process_is_alive() {
        let a = id();
        assert_eq!(a, id(), "the same for the whole run");
        assert!(!a.is_empty());
        assert!(alive(std::process::id()));
        #[cfg(unix)]
        assert!(alive(1), "init is somebody else's, and alive");
        #[cfg(windows)]
        assert!(alive(4), "System is somebody else's, and alive");
        assert!(!alive(4_000_000), "no such process");
        // A child that has ended is not alive, whatever its number says.
        let mut child = if cfg!(windows) { std::process::Command::new("cmd").args(["/c", "exit"]).spawn().unwrap() } else { std::process::Command::new("true").spawn().unwrap() };
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(pid), "ended, and waited for");
    }

    /// After a reboot nothing is asked of a pid: the old boot's processes
    /// are gone, and the number may be a stranger's. On the same boot the
    /// pid is asked.
    /// The adapter that led the group can be gone while the agent it
    /// started goes on: the agent is still there.
    #[cfg(unix)]
    #[test]
    fn an_agent_outlives_its_adapter_in_its_group() {
        use std::os::unix::process::CommandExt;
        let mut leader = std::process::Command::new("sh").args(["-c", "sleep 30 & exit 0"]).process_group(0).spawn().unwrap();
        let pid = leader.id();
        leader.wait().unwrap();
        assert!(!alive(pid), "the adapter has exited");
        assert!(agent_alive(pid, None), "what it started is still in its group");
        // SAFETY: the group was made above and holds only the sleep.
        unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
        let mut gone = false;
        for _ in 0..100 {
            if !agent_alive(pid, None) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(gone, "and once that goes, nothing is");
    }

    /// What a comeback found waits in the store until it has been told:
    /// an app that goes down again first tells it next time.
    #[test]
    fn a_comeback_is_kept_until_told() {
        let store = orchestra_store::Store::in_memory().unwrap();
        let quiet = Comeback { rebooted: false, last_alive: None, closed: Vec::new(), routines: Vec::new(), why: "nothing".into(), done: Vec::new(), at: 1 };
        assert!(keep_pending(&store, quiet.clone()).is_empty(), "nothing cut, nothing to keep");
        let cut = Comeback { closed: vec![ClosedRun { run: run("t564", RunStatus::Interrupted, false), detached: false }], why: "the machine rebooted".into(), ..quiet.clone() };
        assert_eq!(keep_pending(&store, cut).len(), 1);
        // Down again before telling: the next start has the first still,
        // and its own.
        let routine = Comeback { routines: vec![orchestra_store::ClosedRoutineRun { run: "rr1".into(), routine: "ro001".into(), left: None }], why: "went away".into(), ..quiet };
        let both = keep_pending(&store, routine);
        assert_eq!(both.iter().map(|c| c.why.as_str()).collect::<Vec<_>>(), ["the machine rebooted", "went away"]);
        assert_eq!(both[0].closed[0].run.id, "t564");
        keep_left(&store, &both[1..]);
        assert_eq!(keep_pending(&store, Comeback { closed: Vec::new(), routines: Vec::new(), ..both[1].clone() }).len(), 1, "the told one is gone");
        keep_left(&store, &[]);
        assert!(store.get_meta(PENDING_KEY).unwrap().is_none());
        // What of it was told goes with it, so a replay does each part once.
        let mut part = both[0].clone();
        part.done = vec!["notice:tr001".into(), resume_key("t564", &Ok("t570".into())), resume_key("t565", &Err("quiet hours until 05:00".into()))];
        let kept = keep_pending(&store, part);
        assert!(told(&kept[0].done, "notice:tr001") && !told(&kept[0].done, "summary:tr001"));
        assert_eq!(resumed_before(&kept[0].done, "t564"), Some(Ok("t570".into())), "started again once, as t570");
        assert_eq!(resumed_before(&kept[0].done, "t565"), Some(Err("quiet hours until 05:00".into())));
        assert_eq!(resumed_before(&kept[0].done, "t56"), None, "a run is its own key, not a prefix of another");
        keep_left(&store, &[]);
    }

    #[test]
    fn a_documents_or_a_plans_run_is_not_a_tracks() {
        assert!(not_a_track("artifact:a01") && not_a_track("routine:r01"));
        assert!(!not_a_track("tr001"));
        let store = orchestra_store::Store::in_memory().unwrap();
        assert_eq!(owner_name(&store, "artifact:gone"), "gone", "its id when it is not there");
        assert_eq!(owner_name(&store, "tr001"), "tr001");
    }

    #[test]
    fn a_reboot_leaves_no_agent_to_find() {
        let me = std::process::id();
        assert!(!agent_check(true)(me, None), "alive here, but not from the boot before");
        assert!(agent_check(false)(me, None));
        // A number kept with its start: the same process only if it
        // started then; another start is a stranger given the number.
        let start = started(me).expect("this process's start can be read");
        assert_eq!(started(me), Some(start), "the same every time it is asked");
        assert!(agent_alive(me, Some(start)));
        assert!(!agent_alive(me, Some(start + 1)), "a process of that number that started at another time is not the agent");
        assert!(agent_check(false)(me, Some(start)));
        assert_eq!(identity(Some(me)), (Some(me), Some(start)));
        assert_eq!(identity(None), (None, None));
        assert!(same_alive(me, Some(start)) && same_alive(me, None));
        assert!(!same_alive(me, Some(start + 1)), "the number, given to another process");
        assert_eq!(elapsed_ms(1_000, 61_000), 60_000);
        assert_eq!(elapsed_ms(5_000, 1_000), 0, "a clock set back is not a negative run");
    }

    fn run(id: &str, status: RunStatus, repeatable: bool) -> RunSummary {
        RunSummary {
            id: id.into(),
            track: "tr001".into(),
            session: "review-core".into(),
            agent: "claude_code".into(),
            prompt: "patch the repo\nmore".into(),
            cwd: ".".into(),
            status,
            started_at: 1_760_000_000_000,
            ended_at: Some(1_760_000_600_000),
            duration_ms: Some(600_000),
            session_id: None,
            stop_reason: None,
            error: None,
            output: String::new(),
            plan: Vec::new(),
            tool_count: 0,
            pid: Some(42),
            pid_start: None,
            repeatable,
        }
    }

    #[test]
    fn the_conductor_is_told_what_was_cut_and_how_to_go_on() {
        let lines = vec![line(&run("t564", RunStatus::Interrupted, false), false), line(&run("t565", RunStatus::Detached, false), false)];
        assert!(lines[0].starts_with("t564 review-core — started "), "{}", lines[0]);
        assert!(lines[0].contains("last seen") && lines[0].ends_with("patch the repo"));
        assert!(lines[1].contains("pid 42") && lines[1].contains("still alive"));
        let text = restart_text("the machine rebooted", &lines, &[], &[], 1, false);
        assert!(text.starts_with("[divixi-restart] the machine rebooted"));
        assert!(text.contains("NOT started again") && text.contains("resume_run(run)"), "{text}");
        let auto = restart_text("x", &lines, &["t564 → t570".into()], &[], 1, true);
        assert!(auto.contains("자동으로 다시 돌렸습니다: t564 → t570"));
        assert!(!auto.contains("나머지는"), "nothing else to start by hand");
        // A start that failed is not a start: said apart, and not counted
        // as one, so the conductor is told to try again or escalate.
        let failed = restart_text("x", &lines, &[], &["t564: quiet hours until 05:00".into()], 1, false);
        assert!(failed.contains("could NOT be started again: t564: quiet hours until 05:00"), "{failed}");
        assert!(!failed.contains("were started again by themselves"), "{failed}");
        assert!(!failed.contains("The rest were NOT"), "the one cut turn is accounted for: {failed}");
        assert!(line(&run("t1", RunStatus::Interrupted, true), true).contains("되풀이 안전"));
        // The conductor reads its own turn named as its own.
        let mut own = run("t9", RunStatus::Detached, false);
        own.session = crate::conductor::CONDUCTOR_SESSION.into();
        assert!(line(&own, false).starts_with("t9 conductor (your own turn) — started"), "{}", line(&own, false));
        assert!(line(&own, true).starts_with("t9 지휘자(당신 자신의 턴) — "));
    }
}
