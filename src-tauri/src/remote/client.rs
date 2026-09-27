//! Using a Divixi on another machine from this app (like Kiro Crew's remote
//! hosts): divixi-server started there over SSH if it is not running, a
//! pairing link it mints (`divixi-server token`), an SSH tunnel to it, and a window of this app
//! showing it. The page there talks to that server, not to this app
//! (it carries `divixi-served`; see ui/src/lib/ipc.svelte.ts).
//!
//! SSH runs non-interactively (BatchMode): the host must be reachable with
//! a key or an agent, as `ssh <host>` in a terminal would be. Its options
//! are Kiro Crew's for supervised tunnels (instances.md §9).

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{AppHandle, AppState};

const HOSTS_KEY: &str = "setting:remote.hosts";

/// A Divixi on another machine, reached over SSH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    /// What follows `ssh`: a name, `user@host`, or an ~/.ssh/config alias.
    pub ssh: String,
    /// divixi-server's port on that machine.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Where divixi-server is there (a login shell resolves `~`).
    #[serde(default = "default_bin")]
    pub bin: String,
    /// Put in front of PATH there when starting divixi-server, so it finds
    /// the agents (as Kiro Crew's "Remote PATH"). Empty: the SSH shell's own.
    #[serde(default)]
    pub path: String,
}

fn default_port() -> u16 {
    super::DEFAULT_PORT
}

fn default_bin() -> String {
    "~/.local/bin/divixi-server".to_string()
}

/// A host as the settings page shows it.
#[derive(Serialize)]
pub struct HostView {
    #[serde(flatten)]
    pub host: Host,
    /// The local port of its tunnel while connected.
    pub local_port: Option<u16>,
}

/// Open tunnels, by host id.
#[derive(Default)]
pub struct Tunnels {
    open: tokio::sync::Mutex<HashMap<String, Tunnel>>,
}

struct Tunnel {
    local_port: u16,
    child: tokio::process::Child,
}

fn hosts(app: &AppHandle) -> Vec<Host> {
    app.state::<AppState>().store.get_meta(HOSTS_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn save(app: &AppHandle, list: &[Host]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    app.state::<AppState>().store.set_meta(HOSTS_KEY, &json).map_err(|e| e.to_string())
}

/// An `ssh` argument that cannot be read as an option.
fn check_ssh(target: &str) -> Result<(), String> {
    if target.trim().is_empty() || target.split(['@', ':']).any(|p| p.starts_with('-')) || target.contains(char::is_whitespace) {
        return Err("the SSH host is a name, user@host or an ~/.ssh/config alias".into());
    }
    Ok(())
}

/// The remote command's path: letters, digits and the usual path signs only.
fn check_bin(bin: &str) -> Result<(), String> {
    let ok = !bin.is_empty() && bin.chars().all(|c| c.is_ascii_alphanumeric() || "~/._-".contains(c)) && !bin.contains("..");
    if ok {
        Ok(())
    } else {
        Err("the divixi-server path may hold letters, digits, ~ / . _ - only".into())
    }
}

/// The remote PATH: directories as in a path, joined by `:`.
fn check_path(path: &str) -> Result<(), String> {
    if path.chars().all(|c| c.is_ascii_alphanumeric() || "~/._-:".contains(c)) {
        Ok(())
    } else {
        Err("the remote PATH may hold letters, digits, ~ / . _ - : only".into())
    }
}

/// The shell line run there: start divixi-server if this user has none
/// running (detached, so it outlives the SSH session), then mint a link.
fn remote_line(host: &Host) -> String {
    let path = if host.path.is_empty() { String::new() } else { format!("PATH={}:\"$PATH\"; export PATH; ", host.path) };
    let bin = &host.bin;
    format!(
        "{path}pgrep -u \"$(id -u)\" -x divixi-server >/dev/null || setsid -f {bin} serve >/dev/null 2>&1 </dev/null; {bin} token"
    )
}

fn ssh() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("ssh");
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Never ask: no terminal to answer in. Never share a connection: a shared
    // one hands the forward over and exits, and the tunnel is no longer ours.
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

/// A free local port (the OS picks one).
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())
}

/// Wait until the tunnel answers, or say why not.
async fn wait_for(port: u16, child: &mut tokio::process::Child) -> Result<(), String> {
    for _ in 0..60 {
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            return Err(format!("ssh ended ({status}): {}", err.trim()));
        }
        if let Ok(mut s) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let probe = format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
            if s.write_all(probe.as_bytes()).await.is_ok() {
                let mut buf = vec![0u8; 256];
                if let Ok(n) = s.read(&mut buf).await {
                    if String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200") {
                        return Ok(());
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("the remote Divixi did not answer through the tunnel: is divixi-server running there?".into())
}

/// Start the remote Divixi if need be and take a pairing token from it.
async fn start_and_token(host: &Host) -> Result<String, String> {
    let out = ssh()
        .arg(&host.ssh)
        .arg(remote_line(host))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("could not run ssh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("/auth/pair?token=")).ok_or_else(|| {
        format!("{} token gave no link: {}", host.bin, String::from_utf8_lossy(&out.stderr).trim())
    })?;
    Ok(line.split("/auth/pair?token=").nth(1).unwrap_or_default().trim().to_string())
}

fn window_label(id: &str) -> String {
    format!("remote-{id}")
}

// ----- commands (the PC's settings page; never reachable remotely) -----

#[tauri::command]
pub async fn remote_hosts(app: AppHandle) -> Vec<HostView> {
    let open = app.state::<AppState>().tunnels.open.lock().await.iter().map(|(k, t)| (k.clone(), t.local_port)).collect::<HashMap<_, _>>();
    hosts(&app).into_iter().map(|h| HostView { local_port: open.get(&h.id).copied(), host: h }).collect()
}

/// Add a host, or change one (same id).
#[tauri::command]
pub async fn remote_host_save(app: AppHandle, mut host: Host) -> Result<Vec<HostView>, String> {
    host.ssh = host.ssh.trim().to_string();
    host.bin = host.bin.trim().to_string();
    host.name = host.name.trim().to_string();
    host.path = host.path.trim().to_string();
    check_ssh(&host.ssh)?;
    check_bin(&host.bin)?;
    check_path(&host.path)?;
    if host.name.is_empty() {
        host.name = host.ssh.clone();
    }
    if host.id.is_empty() {
        host.id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    }
    let mut list = hosts(&app);
    match list.iter_mut().find(|h| h.id == host.id) {
        Some(h) => *h = host,
        None => list.push(host),
    }
    save(&app, &list)?;
    Ok(remote_hosts(app).await)
}

#[tauri::command]
pub async fn remote_host_delete(app: AppHandle, id: String) -> Result<Vec<HostView>, String> {
    disconnect(&app, &id).await;
    let mut list = hosts(&app);
    list.retain(|h| h.id != id);
    save(&app, &list)?;
    Ok(remote_hosts(app).await)
}

/// Open the host in a window: the remote Divixi started if it is not
/// running and a pairing token, then the tunnel (or the one already open),
/// then the window.
#[tauri::command]
pub async fn remote_host_connect(app: AppHandle, id: String) -> Result<Vec<HostView>, String> {
    let host = hosts(&app).into_iter().find(|h| h.id == id).ok_or("no such host")?;
    let token = start_and_token(&host).await?;
    let tunnels = &app.state::<AppState>().tunnels;
    let local_port = {
        let mut open = tunnels.open.lock().await;
        // A tunnel whose ssh has ended is gone.
        if let Some(t) = open.get_mut(&id) {
            if !matches!(t.child.try_wait(), Ok(None)) {
                open.remove(&id);
            }
        }
        match open.get(&id) {
            Some(t) => t.local_port,
            None => {
                let local_port = free_port()?;
                let mut child = ssh()
                    .args(["-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3", "-L"])
                    .arg(format!("127.0.0.1:{local_port}:127.0.0.1:{}", host.port))
                    .arg(&host.ssh)
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("could not run ssh: {e}"))?;
                wait_for(local_port, &mut child).await?;
                open.insert(id.clone(), Tunnel { local_port, child });
                local_port
            }
        }
    };
    let link = format!("http://127.0.0.1:{local_port}/auth/pair?token={token}");
    let url = link.parse::<tauri::Url>().map_err(|e| e.to_string())?;
    let label = window_label(&id);
    match app.get_webview_window(&label) {
        Some(w) => {
            w.navigate(url).map_err(|e| e.to_string())?;
            let _ = w.show();
            let _ = w.set_focus();
        }
        None => {
            tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(url))
                .title(format!("Divixi — {}", host.name))
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .build()
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(remote_hosts(app).await)
}

async fn disconnect(app: &AppHandle, id: &str) {
    if let Some(mut t) = app.state::<AppState>().tunnels.open.lock().await.remove(id) {
        let _ = t.child.kill().await;
    }
    if let Some(w) = app.get_webview_window(&window_label(id)) {
        let _ = w.close();
    }
}

#[tauri::command]
pub async fn remote_host_disconnect(app: AppHandle, id: String) -> Vec<HostView> {
    disconnect(&app, &id).await;
    remote_hosts(app).await
}

/// End every tunnel (the app is quitting; a server has no tray to quit from).
#[cfg_attr(feature = "server", allow(dead_code))]
pub fn close_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let guard = state.tunnels.open.try_lock();
    if let Ok(mut open) = guard {
        for (_, mut t) in open.drain() {
            let _ = t.child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_targets_and_paths_are_checked() {
        assert!(check_ssh("user@example-host").is_ok());
        assert!(check_ssh("my-alias").is_ok());
        assert!(check_ssh("-oProxyCommand=evil").is_err());
        assert!(check_ssh("user@-oevil").is_err());
        assert!(check_ssh("a b").is_err());
        assert!(check_bin("~/.local/bin/divixi-server").is_ok());
        assert!(check_bin("~/x; rm -rf /").is_err());
        assert!(check_bin("../../bin/sh").is_err());
        assert!(check_path("").is_ok());
        assert!(check_path("~/.local/bin:/usr/bin:/bin").is_ok());
        assert!(check_path("/bin;reboot").is_err());
        assert!(check_path("$(reboot)").is_err());
    }

    #[test]
    fn the_remote_line_starts_the_server_once() {
        let mut h = Host { id: "a".into(), name: "a".into(), ssh: "a".into(), port: 7488, bin: "~/x/divixi-server".into(), path: String::new() };
        assert_eq!(
            remote_line(&h),
            "pgrep -u \"$(id -u)\" -x divixi-server >/dev/null || setsid -f ~/x/divixi-server serve >/dev/null 2>&1 </dev/null; ~/x/divixi-server token"
        );
        h.path = "~/.local/bin".into();
        assert!(remote_line(&h).starts_with("PATH=~/.local/bin:\"$PATH\"; export PATH; pgrep"));
    }
}
