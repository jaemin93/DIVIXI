//! Terminals for the bottom panel: a real shell on a pseudo-terminal
//! (ConPTY on Windows), in the open track's folder. Output streams to the
//! webview as `term` events; input, resizes and closing come back as
//! commands. The UI draws them with xterm.js.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// One running shell.
struct Term {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Drop for Term {
    fn drop(&mut self) {
        // The shell (and what it runs in the foreground) goes with its tab.
        let _ = self.child.kill();
    }
}

/// Every open terminal, by id.
#[derive(Default)]
pub struct Terminals {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Term>>,
}

/// A chunk of a terminal's output.
#[derive(Clone, Serialize)]
struct Output {
    id: u32,
    data: String,
}

/// A terminal's shell ended.
#[derive(Clone, Serialize)]
struct Exit {
    id: u32,
}

/// What a new terminal runs: PowerShell 7 when installed, else Windows
/// PowerShell; the login shell elsewhere.
fn shell() -> CommandBuilder {
    #[cfg(windows)]
    {
        let pwsh = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file()));
        let mut cmd = CommandBuilder::new(if pwsh { "pwsh.exe" } else { "powershell.exe" });
        cmd.arg("-NoLogo");
        cmd
    }
    #[cfg(not(windows))]
    {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(sh);
        cmd.arg("-l");
        cmd.env("TERM", "xterm-256color");
        cmd
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(2),
        cols: cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Split `buf` into the longest valid UTF-8 prefix and the bytes of a
/// character cut off at the end, which wait for the next read. Invalid
/// bytes in the middle are replaced, not held back.
fn take_utf8(buf: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(buf) {
            Ok(s) => {
                out.push_str(s);
                buf.clear();
                return out;
            }
            Err(e) => {
                let good = e.valid_up_to();
                out.push_str(std::str::from_utf8(&buf[..good]).unwrap_or_default());
                match e.error_len() {
                    // Cut off mid-character: keep the tail for later.
                    None => {
                        buf.drain(..good);
                        return out;
                    }
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        buf.drain(..good + bad);
                    }
                }
            }
        }
    }
}

impl Terminals {
    /// Start a shell in `cwd` and stream its output. Returns the id.
    pub fn open(&self, app: AppHandle, cwd: Option<&Path>, cols: u16, rows: u16) -> Result<u32, String> {
        let pair = native_pty_system().openpty(size(cols, rows)).map_err(|e| e.to_string())?;
        let mut cmd = shell();
        if let Some(dir) = cwd.filter(|d| d.is_dir()) {
            cmd.cwd(dir);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // The shell holds the slave now; ours would keep the pty open after it exits.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.open.lock().insert(
            id,
            Term {
                master: pair.master,
                writer,
                child,
            },
        );

        // Blocking reads on their own thread, until the shell goes away.
        std::thread::Builder::new()
            .name(format!("term-{id}"))
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                let mut pending = Vec::new();
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            pending.extend_from_slice(&chunk[..n]);
                            let data = take_utf8(&mut pending);
                            if !data.is_empty() {
                                let _ = app.emit("term", Output { id, data });
                            }
                        }
                    }
                }
                let _ = app.emit("term_exit", Exit { id });
            })
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Type into a terminal.
    pub fn write(&self, id: u32, data: &str) -> Result<(), String> {
        let mut open = self.open.lock();
        let term = open.get_mut(&id).ok_or_else(|| format!("no terminal {id}"))?;
        term.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        term.writer.flush().map_err(|e| e.to_string())
    }

    /// Tell the shell its window changed size.
    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let open = self.open.lock();
        let term = open.get(&id).ok_or_else(|| format!("no terminal {id}"))?;
        term.master.resize(size(cols, rows)).map_err(|e| e.to_string())
    }

    /// End a terminal and its shell.
    pub fn close(&self, id: u32) {
        // Dropped outside the lock: killing can take a moment.
        let term = self.open.lock().remove(&id);
        drop(term);
    }
}

#[cfg(test)]
mod tests {
    use super::take_utf8;

    /// A real shell on a real pseudo-terminal answers.
    #[cfg(windows)]
    #[test]
    fn shell_runs_on_a_pty() {
        use portable_pty::native_pty_system;
        use std::io::{Read, Write};
        let pair = native_pty_system().openpty(super::size(80, 24)).unwrap();
        let mut cmd = super::shell();
        cmd.args(["-NoProfile", "-Command", "Write-Output (\"divixi-\" + \"pty-ok\")"]);
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                all.extend_from_slice(&buf[..n]);
                // ConPTY asks where the cursor is and waits for the answer;
                // xterm.js gives it in the app, the test has to.
                if buf[..n].windows(4).any(|w| w == b"[6n") {
                    let _ = writer.write_all(b"[1;1R");
                    let _ = writer.flush();
                }
                if String::from_utf8_lossy(&all).contains("divixi-pty-ok") {
                    let _ = tx.send(true);
                    return;
                }
            }
            eprintln!("pty said: {:?}", String::from_utf8_lossy(&all));
            let _ = tx.send(false);
        });
        let ok = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap_or(false);
        let _ = child.kill();
        assert!(ok, "the shell did not answer on the pty");
    }

    #[test]
    fn utf8_split_across_reads_waits_for_the_rest() {
        let bytes = "가나".as_bytes();
        let mut buf = bytes[..4].to_vec(); // "가" and one byte of "나"
        assert_eq!(take_utf8(&mut buf), "가");
        assert_eq!(buf.len(), 1);
        buf.extend_from_slice(&bytes[4..]);
        assert_eq!(take_utf8(&mut buf), "나");
        assert!(buf.is_empty());

        let mut bad = vec![b'a', 0xff, b'b'];
        assert_eq!(take_utf8(&mut bad), "a\u{FFFD}b");
    }
}
