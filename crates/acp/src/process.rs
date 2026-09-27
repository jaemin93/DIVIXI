//! Agent subprocess lifecycle.
//!
//! The protocol crate can spawn agents itself, but it hides the child and
//! its teardown only kills the direct child, after a grace period, and only
//! when the protocol ends cleanly. Agents that ignore stdin EOF (Antigravity's
//! ACP server does) or that launch a real worker behind a wrapper outlive
//! the session. Orchestra spawns agents here instead, so that:
//!
//! - the child is killed on drop, always;
//! - the tree it started goes with it, by the means each platform offers:
//!   - **Windows**: a Job Object with kill-on-close. This covers every
//!     descendant, including one started as a detached process.
//!   - **Unix**: the agent gets a process group of its own, and teardown
//!     signals the group (SIGTERM, a grace period, then SIGKILL). This covers
//!     descendants that stay in the group, which is what the adapters' own
//!     `npx` -> `node` -> agent chain does. A descendant that calls `setsid()`
//!     starts a new session and escapes; there is no portable answer to that
//!     on Unix, so the two platforms are not equally strict and the tests say
//!     so explicitly;
//! - stderr is drained into tracing and a short tail is kept for error
//!   reports (an agent's last words are usually the only diagnostic).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use agent_client_protocol::Lines;
use futures::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::AgentSpec;

/// How many trailing stderr lines to keep for diagnostics.
const STDERR_TAIL: usize = 40;

/// How long the agent's process group has to act on SIGTERM before it is sent
/// SIGKILL. Long enough for a Node process to run its exit handlers, short
/// enough that closing a track does not feel stuck.
#[cfg(unix)]
const TERM_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// How often to ask whether the agent's process group has emptied out, while
/// the SIGTERM grace runs.
#[cfg(unix)]
const POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// A running agent. Dropping it kills the process and the tree it started.
pub struct AgentProcess {
    child: tokio::process::Child,
    pid: Option<u32>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    #[cfg(windows)]
    _job: Option<job::Job>,
    /// The agent's process group id, which equals its pid: `spawn` puts it in a
    /// group of its own. `None` once teardown has signalled and reaped it, so
    /// nothing signals a pgid the kernel may since have reused.
    #[cfg(unix)]
    group: Option<i32>,
}

/// Has every process in `pgid` gone?
///
/// Signal 0 is the POSIX existence check: it delivers nothing and only reports
/// whether the target could be signalled. ESRCH means the group has no members
/// left. Anything else (EPERM, say) is treated as "still there", because the
/// safe reading of an unclear answer is that something is still running.
///
/// A zombie counts as present: a process exists until it is reaped. The caller
/// therefore reaps the leader before relying on this.
#[cfg(unix)]
fn group_gone(pgid: i32) -> bool {
    if pgid <= 1 {
        return true;
    }
    // SAFETY: signal 0 delivers nothing; killpg cannot violate memory safety.
    if unsafe { libc::killpg(pgid, 0) } == 0 {
        return false;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Send `sig` to every process in `pgid`.
///
/// **What this signals.** `pgid` is only ever the id of a group this crate
/// created in [`spawn`], which is the agent and every process it started that
/// stayed in that group. It is never a group we did not make.
///
/// Two things it deliberately refuses. `killpg(0, ..)` means "my own process
/// group" and would signal Divixi itself, so 0 is rejected; 1 is rejected for
/// the same class of mistake. And the caller clears [`AgentProcess::group`]
/// after reaping, because a pgid whose leader has been reaped is free for the
/// kernel to reuse, and signalling it then could hit an unrelated process.
///
/// A child that calls `setsid()` leaves the group and so escapes this. That is
/// a property of process groups, not a bug here: see the note on [`spawn`].
#[cfg(unix)]
fn signal_group(pgid: i32, sig: i32, pid: Option<u32>) {
    if pgid <= 1 {
        tracing::warn!(pgid, "refusing to signal process group 0 or 1");
        return;
    }
    // SAFETY: killpg takes a pgid and a signal number and cannot violate
    // memory safety. ESRCH (nothing left in the group) is the expected result
    // of the second call and is not an error worth reporting.
    let rc = unsafe { libc::killpg(pgid, sig) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            tracing::debug!(?pid, pgid, sig, %err, "killpg failed");
        }
    }
}

impl AgentProcess {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The last lines the agent wrote to stderr, oldest first.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail.lock().map(|t| t.iter().cloned().collect()).unwrap_or_default()
    }

    /// Terminate the agent and everything it started. Idempotent.
    ///
    /// On Unix the order is SIGTERM to the whole process group, then up to
    /// [`TERM_GRACE`] for it to go, then SIGKILL to whatever is left. The
    /// polite signal goes first so an agent can flush and remove its
    /// temporary files; the deadline is there because the module's promise is
    /// that the tree dies, not that it is asked nicely. The direct child is
    /// then reaped either way, so it cannot linger as a zombie.
    ///
    /// On Windows none of that applies: the job object kills the tree when its
    /// handle closes, which is already atomic and immediate.
    pub async fn shutdown(mut self) {
        let exited = matches!(self.child.try_wait(), Ok(Some(_)));
        if exited {
            tracing::debug!(pid = ?self.pid, "agent already exited");
        }

        // Signalled even when the child has already exited: its death says
        // nothing about the grandchildren, which are the whole point here.
        //
        // `self.group` is deliberately NOT taken until after SIGKILL. If this
        // future is cancelled at one of the awaits below -- and it is, on a
        // path that happens routinely: a failed session start sends its error
        // and then calls shutdown(), and the creator answers by aborting the
        // task (see session.rs) -- then `Drop` runs instead, and it can only
        // act as the backstop if the pgid is still here. Taking it first left
        // the group with no SIGKILL at all, which is exactly the leak this
        // whole mechanism exists to prevent.
        #[cfg(unix)]
        if let Some(pgid) = self.group {
            let deadline = tokio::time::Instant::now() + TERM_GRACE;
            signal_group(pgid, libc::SIGTERM, self.pid);

            // Reap the leader first. Until it is reaped it stays a zombie, and
            // a zombie is still a group member, so the emptiness check below
            // would never come back true while it lingers.
            let _ = tokio::time::timeout_at(deadline, self.child.wait()).await;

            // Now give the REST of the group the rest of the grace. Waiting on
            // the direct child alone was not a grace period at all: `try_wait`
            // above may already have reaped it, in which case `wait` returns at
            // once and SIGKILL followed SIGTERM by microseconds -- in precisely
            // the case this code is for, a live grandchild behind a dead
            // wrapper.
            let mut lingered = false;
            while tokio::time::Instant::now() < deadline {
                if group_gone(pgid) {
                    break;
                }
                lingered = true;
                tokio::time::sleep(POLL).await;
            }

            if group_gone(pgid) {
                tracing::debug!(pid = ?self.pid, pgid, lingered, "agent group stopped on SIGTERM");
            } else {
                tracing::debug!(pid = ?self.pid, pgid, "agent group still up after SIGTERM; sending SIGKILL");
            }
            // Harmless if the group is already gone: killpg answers ESRCH and
            // `signal_group` ignores it.
            //
            // On pid reuse: the leader has been reaped by now, which in
            // principle frees its pid, and this pgid is that pid. It is not a
            // hole in practice, because a process group lives as long as any
            // member does and the kernel will not hand that pid to a new
            // process while the group exists. So in the case this code is for
            // -- a grandchild still running -- the group is still ours.
            signal_group(pgid, libc::SIGKILL, self.pid);
            // Only now: past here there is nothing left for Drop to do.
            self.group = None;
        }

        // Reap, and say honestly whether anything was killed. The question is
        // whether the child is alive *now*, not whether it was alive when this
        // function started: on Unix the block above has already waited on it,
        // and asking the stale answer logged "agent kill failed" after every
        // ordinary shutdown.
        let still_running = matches!(self.child.try_wait(), Ok(None));
        if still_running {
            if let Err(err) = self.child.kill().await {
                tracing::debug!(pid = ?self.pid, %err, "agent kill failed (probably already gone)");
            } else {
                tracing::debug!(pid = ?self.pid, "agent terminated");
            }
        } else if !exited {
            tracing::debug!(pid = ?self.pid, "agent exited during shutdown");
        }
        // `_job` drops here: on Windows that closes the job and kills any
        // grandchildren the agent left behind.
    }
}

/// The promise in this module's header is that the tree dies *always*, not only
/// when someone remembers to call [`AgentProcess::shutdown`]. `kill_on_drop`
/// covers the direct child; this covers the rest of its group.
///
/// There is no grace period here, because `drop` cannot await one. A caller who
/// wants the agent asked politely first should call `shutdown` instead.
#[cfg(unix)]
impl Drop for AgentProcess {
    fn drop(&mut self) {
        if let Some(pgid) = self.group.take() {
            signal_group(pgid, libc::SIGKILL, self.pid);
        }
    }
}

/// The line transport handed to the protocol client.
pub type AgentLines = Lines<
    std::pin::Pin<Box<dyn futures::Sink<String, Error = std::io::Error> + Send>>,
    std::pin::Pin<Box<dyn futures::Stream<Item = std::io::Result<String>> + Send>>,
>;

/// Launch an agent and wire its stdio into a protocol transport.
pub fn spawn(spec: &AgentSpec) -> anyhow::Result<(AgentProcess, AgentLines)> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Give the agent a process group of its own (setpgid(0, 0) in the child),
    // so teardown can signal the agent *and what it started* with one call.
    // The adapters need exactly this: `npx` execs `node`, which runs the real
    // agent, and killing the middle process alone leaves the worker running.
    //
    // Its pgid is its pid, which is why `AgentProcess::group` can be derived
    // from `child.id()` below.
    //
    // Safe with piped stdio: job-control signals (SIGTTIN/SIGTTOU) only reach
    // a background group that touches a terminal, and these children get
    // pipes, never a tty.
    //
    // Limit worth knowing: a descendant that calls `setsid()` starts its own
    // session and leaves this group, so it survives. Node's
    // `spawn(.., {detached: true})` does that. Windows' job object has no such
    // hole, so the two platforms are not equally strict.
    // tokio::process::Command has its own process_group on unix, so no
    // std::os::unix::process::CommandExt import is needed here.
    //
    // What this costs, and it is a real cost: leaving our process group also
    // leaves the terminal's FOREGROUND group. SIGINT and SIGQUIT are delivered
    // to the foreground group, so Ctrl-C in a shell running `divixi-server` in
    // the foreground no longer reaches the agents -- it only reaches the server,
    // which has no signal handler and dies at once, running neither `shutdown`
    // nor `Drop`, and leaving the agent trees behind.
    //
    // Not a problem for the desktop app (closing the window goes through the
    // ordinary shutdown path) or on Windows (the kernel tears the job object
    // down when the handle closes). Not a problem under systemd either, whose
    // default `KillMode=control-group` kills the unit's whole cgroup, agents
    // included -- and that is how docs/divixi-server.md says to run it. It bites
    // the documented debugging path: `divixi-server serve` in a terminal.
    //
    // The fix belongs in the binary, not here: `divixi-server` needs to wait on
    // ctrl_c()/SIGTERM and close its sessions. Until it does, that terminal is
    // the one place where an interrupted Divixi leaks agents.
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to launch {}: {e}", spec.program))?;
    let pid = child.id();
    // Set above, so the group id is the child's pid.
    #[cfg(unix)]
    let group = pid.map(|p| p as i32);

    #[cfg(windows)]
    let job = match child.raw_handle().map(job::Job::containing) {
        Some(Ok(job)) => Some(job),
        Some(Err(err)) => {
            tracing::warn!(?pid, %err, "could not put agent in a job object; grandchildren may outlive it");
            None
        }
        None => None,
    };

    let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("agent stdin not piped"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("agent stdout not piped"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("agent stderr not piped"))?;

    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
    {
        let tail = stderr_tail.clone();
        let program = spec.program.clone();
        tokio::spawn(async move {
            // Bytes, decoded lossily: a non-UTF-8 line (a Korean-locale
            // shell, say) must not end the drain, or the agent blocks on a
            // full stderr pipe.
            let mut reader = futures::io::BufReader::new(stderr.compat());
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                tracing::debug!(target: "agent_stderr", program = %program, pid = ?pid, "{line}");
                if let Ok(mut t) = tail.lock() {
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            }
        });
    }

    let incoming: std::pin::Pin<Box<dyn futures::Stream<Item = std::io::Result<String>> + Send>> =
        Box::pin(futures::io::BufReader::new(stdout.compat()).lines());

    let outgoing: std::pin::Pin<Box<dyn futures::Sink<String, Error = std::io::Error> + Send>> =
        Box::pin(futures::sink::unfold(stdin, |mut writer, line: String| async move {
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            Ok::<_, std::io::Error>(writer)
        }));

    tracing::debug!(program = %spec.program, ?pid, "agent spawned");
    Ok((
        AgentProcess {
            child,
            pid,
            stderr_tail,
            #[cfg(windows)]
            _job: job,
            #[cfg(unix)]
            group,
        },
        Lines::new(outgoing, incoming),
    ))
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::RawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A Job Object that kills every process in it when the handle closes.
    pub struct Job(HANDLE);

    // The handle is only ever closed, from any thread.
    unsafe impl Send for Job {}

    impl Job {
        /// Create a kill-on-close job and put `process` in it.
        pub fn containing(process: RawHandle) -> anyhow::Result<Self> {
            // SAFETY: plain Win32 calls with valid arguments; the job handle is
            // owned by the returned `Job` and closed exactly once.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    anyhow::bail!("CreateJobObjectW failed: {}", std::io::Error::last_os_error());
                }
                let job = Job(job);

                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    anyhow::bail!("SetInformationJobObject failed: {}", std::io::Error::last_os_error());
                }
                if AssignProcessToJobObject(job.0, process as HANDLE) == 0 {
                    anyhow::bail!("AssignProcessToJobObject failed: {}", std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: handle is valid and owned; closing it ends the job.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Does killing an agent take its descendants with it?
    //!
    //! This is the shape the real adapters have: Divixi launches `node`, which
    //! launches the agent that does the work. Killing the middle process alone
    //! leaves the worker running, which is what these tests are here to stop.
    //!
    //! Liveness is observed through a file the grandchild appends to, not
    //! through any process-inspection API: it needs no extra dependency and
    //! behaves the same on every platform. The grandchild also exits by itself
    //! after 30 seconds, so a failing test cannot leave anything running on a
    //! CI machine.
    //!
    //! Two tests, because the platforms genuinely differ (see the module
    //! header):
    //!
    //! - [`shutdown_kills_the_process_group`] is the portable contract. The
    //!   grandchild stays in the agent's process group, as the adapters' own
    //!   chain does. Both platforms must pass this.
    //! - [`shutdown_kills_a_detached_grandchild`] is Windows only. A detached
    //!   child calls `setsid()` on Unix and leaves the group, which no
    //!   portable Unix mechanism can follow; the Job Object has no such hole.
    //!   It is `#[cfg(windows)]` rather than `#[ignore]` so that the asymmetry
    //!   is a statement in the code, not a skipped test someone has to explain.

    use std::path::Path;
    use std::time::{Duration, Instant};

    use crate::AgentSpec;

    /// Is `node` on PATH? These tests need it and say so rather than failing.
    fn have_node() -> bool {
        std::process::Command::new(if cfg!(windows) { "node.exe" } else { "node" })
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// A parent that starts a grandchild and then ignores stdin, the way an
    /// adapter behind `npx` does. `detached` decides whether the grandchild
    /// stays in the process group or leaves for a session of its own.
    fn wrapper_js(detached: bool) -> String {
        format!(
            r#"
const {{ spawn }} = require("node:child_process");
const child = spawn(process.execPath, [process.argv[2], process.argv[3]], {{
  detached: {detached},
  stdio: "ignore",
}});
child.unref();
// Outlive stdin closing, which is the case the module header calls out.
setInterval(() => {{}}, 1000);
"#
        )
    }

    /// The grandchild: append a byte forever, so being alive is visible from
    /// the file alone. Self-terminating, so nothing outlives a failed test.
    const HEARTBEAT_JS: &str = r#"
const fs = require("node:fs");
const file = process.argv[2];
setInterval(() => { try { fs.appendFileSync(file, "."); } catch {} }, 25);
setTimeout(() => process.exit(0), 30000);
"#;

    fn size(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    /// Wait for `f`, up to `limit`. Returns whether it happened.
    async fn until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    /// Start `node wrapper.js heartbeat.js beat.txt`, wait for the grandchild
    /// to be writing, tear the agent down, and report whether the grandchild
    /// kept going afterwards.
    /// How the agent is torn down, which is the axis these tests vary.
    #[derive(Clone, Copy, PartialEq)]
    enum Teardown {
        /// `shutdown().await` runs to completion.
        Graceful,
        /// The task is aborted while `shutdown()` is parked on an await, so the
        /// future is dropped and `Drop` is the only thing left to clean up.
        /// This is not hypothetical: session.rs aborts the task right after a
        /// failed session start calls `shutdown()`.
        Cancelled,
    }

    async fn grandchild_survives_shutdown(detached: bool, how: Teardown) -> bool {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("divixi-tree-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let wrapper = dir.join("wrapper.js");
        let heartbeat = dir.join("heartbeat.js");
        let beat = dir.join("beat.txt");
        std::fs::write(&wrapper, wrapper_js(detached)).expect("write wrapper");
        std::fs::write(&heartbeat, HEARTBEAT_JS).expect("write heartbeat");

        let spec = AgentSpec {
            program: "node".to_string(),
            args: vec![
                wrapper.to_string_lossy().into_owned(),
                heartbeat.to_string_lossy().into_owned(),
                beat.to_string_lossy().into_owned(),
            ],
            env: Vec::new(),
        };

        let (process, lines) = super::spawn(&spec).expect("spawn the wrapper");

        // Growing, not merely present: an empty file proves nothing.
        let started = until(Duration::from_secs(20), || size(&beat) > 4).await;
        assert!(started, "the grandchild never started writing {}", beat.display());

        // Closing stdin must not be what stops it: that is the case the module
        // header calls out, and the wrapper ignores it.
        drop(lines);

        match how {
            Teardown::Graceful => process.shutdown().await,
            Teardown::Cancelled => {
                // Get shutdown() into one of its awaits, then abort. Dropping
                // the future drops the AgentProcess, so Drop is the only
                // remaining chance to kill the group.
                let task = tokio::spawn(async move { process.shutdown().await });
                tokio::time::sleep(Duration::from_millis(60)).await;
                task.abort();
                let _ = task.await;
            }
        }

        let at_shutdown = size(&beat);
        tokio::time::sleep(Duration::from_millis(750)).await;
        let later = size(&beat);
        let _ = std::fs::remove_dir_all(&dir);

        later > at_shutdown
    }

    /// The portable contract: a grandchild in the agent's process group dies
    /// with it. Windows does this with the job object, Unix by signalling the
    /// group.
    ///
    /// Which platform this really tests: Unix. Measured on Windows 11, a
    /// non-detached grandchild already dies when its parent is force-killed, so
    /// here it would pass with or without the job object. On Unix an orphan is
    /// reparented to init and keeps running, so there the assertion has teeth
    /// and it is the process-group teardown that satisfies it.
    /// `shutdown_kills_a_detached_grandchild` is the one with teeth on Windows.
    #[tokio::test]
    // TODO: remove once the first CI run confirms the Unix teardown works.
    // The process-group fix landed with this test, but it has never been
    // executed on macOS or Linux -- only cross-compiled. ci.yml runs it there
    // as a reporting step (`continue-on-error`) so the real result is visible
    // without a wrong guess blocking every pull request. When that step passes,
    // delete this attribute and the step, and the test becomes an ordinary gate.
    #[cfg_attr(not(windows), ignore = "unverified on Unix: ci.yml runs it as a reporting step; promote to a gate once that passes")]
    async fn shutdown_kills_the_process_group() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let leaked = grandchild_survives_shutdown(false, Teardown::Graceful).await;
        assert!(
            !leaked,
            "a grandchild in the agent's own process group kept running after shutdown(). \
             On Unix that means the SIGTERM/SIGKILL to the group did not reach it; on Windows \
             it means the job object did not take the tree."
        );
    }

    /// The same contract when nobody gets to finish tidying up.
    ///
    /// `shutdown()` parks on an await, and session.rs aborts the task that is
    /// running it right after a failed session start -- so the future is
    /// dropped mid-teardown and `Drop` is all that is left. This shipped
    /// broken: the pgid was taken out of the struct before the first await, so
    /// `Drop` found `None` and the group never got SIGKILL. Nothing in the
    /// log said so; the grandchild simply stayed.
    ///
    /// Which platform this really tests: Unix, again. There the grace is 500ms
    /// and the poll loop runs, so the abort at 60ms is certain to land while
    /// `shutdown()` is parked. On Windows there is no grace at all -- the job
    /// handle does the work -- so `shutdown()` may well have finished before
    /// the abort arrives, and the case degenerates into the graceful one. It is
    /// still worth running there: it costs nothing and it pins that closing the
    /// job handle is what tears the tree down, however teardown was reached.
    #[tokio::test]
    #[cfg_attr(not(windows), ignore = "unverified on Unix: ci.yml runs it as a reporting step; promote to a gate once that passes")]
    async fn a_cancelled_shutdown_still_kills_the_tree() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let leaked = grandchild_survives_shutdown(false, Teardown::Cancelled).await;
        assert!(
            !leaked,
            "the grandchild outlived an aborted shutdown(). Drop is the backstop for exactly this              case, so either the pgid was cleared before Drop could use it (Unix) or the job handle              did not close (Windows)."
        );
    }

    /// Windows only, and on purpose. A detached child calls `setsid()` on Unix,
    /// leaving the process group behind, so no portable Unix teardown can reach
    /// it. The job object can, and that is worth pinning down: it is the one
    /// place where Windows gives a stronger guarantee than Unix, and a future
    /// change to the job flags would silently lose it.
    #[cfg(windows)]
    #[tokio::test]
    async fn shutdown_kills_a_detached_grandchild() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let leaked = grandchild_survives_shutdown(true, Teardown::Graceful).await;
        assert!(
            !leaked,
            "a detached grandchild outlived shutdown() on Windows: the job object should have \
             taken the whole tree regardless of how the child was started."
        );
    }
}
