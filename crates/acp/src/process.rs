//! Agent subprocess lifecycle.
//!
//! The protocol crate can spawn agents itself, but it hides the child and
//! its teardown only kills the direct child, after a grace period, and only
//! when the protocol ends cleanly. Agents that ignore stdin EOF (Antigravity's
//! ACP server does) or that launch a real worker behind a wrapper outlive
//! the lane. Orchestra spawns agents here instead, so that:
//!
//! - the child is killed on drop, always;
//! - on Windows the child is placed in a Job Object with kill-on-close, so
//!   its whole process tree dies with it;
//! - stderr is drained into tracing and a short tail is kept for error
//!   reports (an agent's last words are usually the only diagnostic).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use agent_client_protocol::Lines;
use futures::{AsyncBufReadExt, StreamExt};
use tokio::io::AsyncWriteExt;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::AgentSpec;

/// How many trailing stderr lines to keep for diagnostics.
const STDERR_TAIL: usize = 40;

/// A running agent. Dropping it kills the process (and its tree on Windows).
pub struct AgentProcess {
    child: tokio::process::Child,
    pid: Option<u32>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    #[cfg(windows)]
    _job: Option<job::Job>,
}

impl AgentProcess {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The last lines the agent wrote to stderr, oldest first.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail.lock().map(|t| t.iter().cloned().collect()).unwrap_or_default()
    }

    /// Terminate the agent. Idempotent.
    pub async fn shutdown(mut self) {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                tracing::debug!(pid = ?self.pid, ?status, "agent already exited");
            }
            _ => {
                if let Err(err) = self.child.kill().await {
                    tracing::debug!(pid = ?self.pid, %err, "agent kill failed (probably already gone)");
                } else {
                    tracing::debug!(pid = ?self.pid, "agent terminated");
                }
            }
        }
        // `_job` drops here: on Windows that closes the job and kills any
        // grandchildren the agent left behind.
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

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to launch {}: {e}", spec.program))?;
    let pid = child.id();

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
            let mut lines = futures::io::BufReader::new(stderr.compat()).lines();
            while let Some(Ok(line)) = lines.next().await {
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
