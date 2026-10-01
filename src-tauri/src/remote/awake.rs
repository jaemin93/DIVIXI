//! Keeping the PC awake while phone access is on.
//!
//! A phone reaching this Divixi needs the machine answering, and a machine that
//! sleeps answers nothing. So while phone access is on, Divixi asks the OS not
//! to let the system sleep — and only the system: the screen may still turn
//! off, which is what a laptop shut on a desk should do.
//!
//! The assertion is held by a [`Guard`]. Dropping it releases, so turning phone
//! access off, stopping the server and quitting the app all release through the
//! same path and there is no second place to forget.
//!
//! Windows only for now. On macOS and Linux [`hold`] returns [`Held::Unsupported`]
//! and the card says so: a feature that silently does nothing on two platforms
//! is worse than one that admits where it works. The mechanisms are known —
//! `IOPMAssertionCreateWithName(kIOPMAssertPreventUserIdleSystemSleep)` on
//! macOS, a `org.freedesktop.login1.Manager.Inhibit` fd on systemd — and each
//! wants its own review.

use serde::Serialize;

/// Whether this machine is being kept awake, and if not, why not.
///
/// `Unsupported` is only ever built off Windows, so a Windows build reads it
/// as dead. It stays in the shape rather than being hidden behind a `cfg`,
/// because the card branches on all three on every platform and a value that
/// changes shape per build is a worse thing to render.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Held {
    /// The OS accepted the request: the system will not idle-sleep.
    Awake,
    /// This OS is not covered yet. Phone access still works — the machine will
    /// sleep on its own schedule and the phone will not reach it until it wakes.
    Unsupported,
    /// The OS refused. Carried as its own state rather than folded into
    /// `Unsupported`, because "we do not do this here" and "we tried and it
    /// said no" send the human to different places.
    Refused,
}

/// Holds the machine awake until it is dropped.
#[derive(Debug)]
pub struct Guard {
    pub held: Held,
    #[cfg(windows)]
    stop: Option<std::sync::mpsc::Sender<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(stop) = self.stop.take() {
            // The thread clears the flag and exits. If the receiver is already
            // gone the thread has ended and the flag went with it.
            let _ = stop.send(());
        }
    }
}

#[cfg(windows)]
mod sys {
    //! `SetThreadExecutionState` lives in kernel32, which MSVC links by
    //! default, so this costs no dependency and nothing in the notices.
    #[link(name = "kernel32")]
    extern "system" {
        pub fn SetThreadExecutionState(flags: u32) -> u32;
    }

    /// Keep this state until it is cleared, rather than for one call.
    pub const ES_CONTINUOUS: u32 = 0x8000_0000;
    /// The *system* stays awake. Deliberately without `ES_DISPLAY_REQUIRED`:
    /// the screen may turn off, which is what the card promises.
    pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
}

/// Ask this machine to stay awake. The returned guard releases when dropped.
pub fn hold() -> Guard {
    #[cfg(windows)]
    {
        // The flag is **per thread** and is cleared when that thread exits, so
        // it has to be set on a thread that stays alive for as long as we want
        // to be awake. A thread that sets it and parks is the whole mechanism.
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let (report, got) = std::sync::mpsc::channel::<Held>();
        std::thread::Builder::new()
            .name("divixi-awake".into())
            .spawn(move || {
                // Returns the previous state, or 0 for failure.
                let ok = unsafe { sys::SetThreadExecutionState(sys::ES_CONTINUOUS | sys::ES_SYSTEM_REQUIRED) } != 0;
                let _ = report.send(if ok { Held::Awake } else { Held::Refused });
                if !ok {
                    return;
                }
                // Parks until the guard is dropped (or the app goes, which
                // closes the sender and ends the recv the same way).
                let _ = wait.recv();
                unsafe { sys::SetThreadExecutionState(sys::ES_CONTINUOUS) };
            })
            .ok();
        let held = got.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(Held::Refused);
        if held == Held::Awake {
            tracing::info!("this PC will not sleep while phone access is on");
        } else {
            tracing::warn!("this PC could not be kept awake for phone access");
        }
        Guard { held, stop: Some(stop) }
    }
    #[cfg(not(windows))]
    {
        // Named so the card can say which OS is not covered rather than
        // claiming the machine is being held awake.
        tracing::info!("keeping this PC awake is not implemented on this OS yet");
        Guard { held: Held::Unsupported }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guard_reports_what_this_os_actually_did() {
        let g = hold();
        if cfg!(windows) {
            // CI and a developer machine both run as an ordinary user, and
            // `SetThreadExecutionState` needs no privilege, so this is Awake
            // unless the OS refused — never Unsupported on Windows.
            assert_ne!(g.held, Held::Unsupported, "Windows is covered");
        } else {
            assert_eq!(g.held, Held::Unsupported);
        }
        // Releasing must not panic, and must be safe to do twice over
        // (dropping the guard, then the app's own teardown finding none).
        drop(g);
    }

    #[test]
    fn holding_twice_and_releasing_in_any_order_is_fine() {
        let a = hold();
        let b = hold();
        drop(a);
        drop(b);
    }
}
