//! Agent catalog and detection.
//!
//! Orchestra drives four agents over ACP. This crate knows how each one is
//! installed, how its ACP surface is launched, and how to find out whether
//! it is ready, without prompting the user for anything:
//!
//! 1. **Locate** the agent's CLI executable. The process PATH is not enough:
//!    an installer updates the registry, not running processes, so a user who
//!    installs an agent while Orchestra is open would never see it. The search
//!    re-reads the registry PATH on Windows and checks known install
//!    directories on every platform.
//! 2. **Resolve** the ACP launch: a locally installed npm adapter, the CLI's
//!    own `--acp` mode, or a downloaded server binary.
//! 3. **Probe** over the protocol: `initialize` reports name, version and
//!    login methods; `session/new` answers `auth_required` when nobody has
//!    logged in. See [`orchestra_acp::probe`].
//!
//! The result is one [`AgentStatus`] per agent, which the app stores and the
//! setup and settings screens render.

use std::path::{Path, PathBuf};
use std::time::Duration;

use orchestra_acp::{authenticate, find_local_script, probe, AgentSpec, ProbeReport, SessionProbe};
use serde::{Deserialize, Serialize};

/// The agents Orchestra knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Copilot,
    Antigravity,
}

impl AgentKind {
    pub const ALL: [AgentKind; 4] = [
        AgentKind::ClaudeCode,
        AgentKind::Codex,
        AgentKind::Copilot,
        AgentKind::Antigravity,
    ];

    /// Stable id used in storage and IPC. Matches the serde name.
    pub fn id(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude_code",
            AgentKind::Codex => "codex",
            AgentKind::Copilot => "copilot",
            AgentKind::Antigravity => "antigravity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Copilot => "GitHub Copilot",
            AgentKind::Antigravity => "Antigravity",
        }
    }

    /// Base name of the CLI executable.
    pub fn cli(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Copilot => "copilot",
            AgentKind::Antigravity => "agy",
        }
    }

    /// What to run in a terminal to log in, when the agent does not say.
    pub fn login_hint(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex login",
            AgentKind::Copilot => "copilot login",
            AgentKind::Antigravity => "agy login",
        }
    }

    /// Where to get the CLI.
    pub fn install_hint(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "https://claude.com/claude-code",
            AgentKind::Codex => "https://openai.com/codex",
            AgentKind::Copilot => "https://github.com/github/copilot-cli",
            AgentKind::Antigravity => "https://antigravity.google/cli",
        }
    }
}

/// The Codex ACP adapter, published under the ACP org. It bundles
/// `@openai/codex`, so it does not need the Codex CLI to run, but it shares
/// the CLI's login state (`~/.codex/auth.json`).
pub const CODEX_ADAPTER: &str = "@agentclientprotocol/codex-acp@1.12.0";
const CODEX_ADAPTER_SCRIPT: &str = "node_modules/@agentclientprotocol/codex-acp/dist/index.js";

/// Google's ACP server for Antigravity, as registered in the ACP registry.
///
/// Antigravity's `agy` CLI has no `--acp` mode; Google ships this separate,
/// proprietary binary instead. It is downloaded on request into the app's
/// adapter directory, never installed system-wide.
pub const ANTIGRAVITY_ACP_VERSION: &str = "1.1.1";

/// A downloadable release of the Antigravity ACP server for this platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntigravityRelease {
    pub version: String,
    pub url: String,
    /// File name of the server inside the archive.
    pub exe: String,
    pub args: Vec<String>,
}

/// The release for the running platform, if Google publishes one.
pub fn antigravity_release() -> Option<AntigravityRelease> {
    let v = ANTIGRAVITY_ACP_VERSION;
    let base = "https://dl.google.com/agy-extensions/releases";
    let (url, exe, args): (String, &str, Vec<String>) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => (
            format!("{base}/windows/agy-acp-server-agy_acp_server_{v}-windows-x86_64.zip"),
            "agy_acp_server.exe",
            vec![],
        ),
        ("windows", "aarch64") => (
            format!("{base}/windows/agy-acp-server-agy_acp_server_{v}-windows-arm64.zip"),
            "agy_acp_server.exe",
            vec![],
        ),
        ("macos", "aarch64") => (
            format!("{base}/macos/agy-acp-server-agy_acp_server_{v}-darwin-arm64.zip"),
            "agy_acp_server.par",
            vec![],
        ),
        ("linux", "x86_64") => (
            format!("{base}/linux/agy-acp-server-agy_acp_server_{v}-linux-x86_64.zip"),
            "agy_acp_server.par",
            vec!["--uid=".to_string()],
        ),
        ("linux", "aarch64") => (
            format!("{base}/linux/agy-acp-server-agy_acp_server_{v}-linux-arm64.zip"),
            "agy_acp_server.par",
            vec!["--uid=".to_string()],
        ),
        _ => return None,
    };
    Some(AntigravityRelease {
        version: v.to_string(),
        url,
        exe: exe.to_string(),
        args,
    })
}

/// A located CLI executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliInfo {
    pub path: String,
    /// First line of `--version`, if it answered.
    pub version: Option<String>,
}

/// How the agent's ACP surface will be launched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Adapter {
    /// A locally installed npm adapter, run under `node`.
    LocalScript { path: String },
    /// An npm adapter fetched by `npx` on every launch. Works, but slow.
    Npx { package: String },
    /// The CLI itself speaks ACP.
    Cli { path: String },
    /// A downloaded server binary.
    Binary { path: String },
    /// The server binary is not downloaded yet.
    NeedsDownload { release: AntigravityRelease },
    /// Nothing to launch.
    Missing { reason: String },
}

/// One-word summary for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    /// Probed and a session opened.
    Ready,
    /// Installed; `session/new` asked for login.
    NeedsLogin,
    /// The CLI is there but the ACP server must be downloaded first.
    NeedsDownload,
    /// No CLI and no adapter.
    NotInstalled,
    /// Something answered, but not usefully.
    Error,
}

/// Everything detection learned about one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub kind: AgentKind,
    pub name: String,
    pub readiness: Readiness,
    pub cli: Option<CliInfo>,
    pub adapter: Adapter,
    /// The launch spec to use for sessions, when there is one.
    pub spec: Option<AgentSpec>,
    pub probe: Option<ProbeReport>,
    /// Why `readiness` is `Error`, or a probe failure message.
    pub error: Option<String>,
    pub login_hint: String,
    pub install_hint: String,
}

/// Where detection looks and what it may write.
#[derive(Debug, Clone)]
pub struct DetectOptions {
    /// Directory that holds downloaded adapters (`<dir>/antigravity/<ver>/`).
    pub adapters_dir: PathBuf,
    /// Working directory handed to probe sessions. The workspace root is the
    /// most faithful choice: agents apply per-project trust settings there.
    pub cwd: PathBuf,
    /// Upper bound per agent for the ACP round trip.
    pub probe_timeout: Duration,
    /// Set to skip the protocol probe (locate and resolve only).
    pub skip_probe: bool,
}

impl DetectOptions {
    pub fn new(adapters_dir: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            adapters_dir: adapters_dir.into(),
            cwd: cwd.into(),
            probe_timeout: Duration::from_secs(45),
            skip_probe: false,
        }
    }
}

/// Detect every agent, concurrently.
pub async fn detect_all(opts: &DetectOptions) -> Vec<AgentStatus> {
    futures::future::join_all(AgentKind::ALL.iter().map(|k| detect(*k, opts))).await
}

/// Detect one agent.
pub async fn detect(kind: AgentKind, opts: &DetectOptions) -> AgentStatus {
    let cli = locate_cli(kind).await;
    let adapter = resolve_adapter(kind, cli.as_ref(), &opts.adapters_dir);
    let spec = spec_for(&adapter);

    let mut status = AgentStatus {
        kind,
        name: kind.name().to_string(),
        readiness: Readiness::NotInstalled,
        cli,
        adapter: adapter.clone(),
        spec: spec.clone(),
        probe: None,
        error: None,
        login_hint: kind.login_hint().to_string(),
        install_hint: kind.install_hint().to_string(),
    };

    let Some(spec) = spec else {
        status.readiness = match adapter {
            Adapter::NeedsDownload { .. } => Readiness::NeedsDownload,
            _ => Readiness::NotInstalled,
        };
        return status;
    };

    if opts.skip_probe {
        status.readiness = Readiness::Error;
        status.error = Some("probe skipped".to_string());
        return status;
    }

    tracing::info!(agent = kind.id(), program = %spec.program, "probing agent");
    let outcome = probe(&spec, &opts.cwd, opts.probe_timeout).await;
    apply_probe(&mut status, outcome);
    tracing::info!(agent = kind.id(), readiness = ?status.readiness, "probe done");
    status
}

/// Log an agent in through ACP `authenticate` and re-probe it.
///
/// `method_id` must be one the agent advertised in `status.probe.auth_methods`;
/// `None` picks the first. The agent runs the flow itself (cached
/// credentials, a browser, or a device code), so this may take a while and
/// may need the user to act. The returned status reflects the outcome.
pub async fn login(status: &AgentStatus, method_id: Option<&str>, opts: &DetectOptions) -> AgentStatus {
    let mut next = status.clone();
    let Some(spec) = &status.spec else {
        next.error = Some("nothing to log in to: no adapter".to_string());
        return next;
    };
    let method = method_id
        .map(str::to_owned)
        .or_else(|| status.probe.as_ref()?.auth_methods.first().map(|m| m.id.clone()));
    let Some(method) = method else {
        next.error = Some("the agent advertises no auth methods; log in with its CLI".to_string());
        return next;
    };

    tracing::info!(agent = status.kind.id(), method, "authenticating agent");
    // Login flows can involve a browser round trip; give them longer.
    let timeout = opts.probe_timeout.max(Duration::from_secs(180));
    let outcome = authenticate(spec, &opts.cwd, &method, timeout).await;
    apply_probe(&mut next, outcome);
    tracing::info!(agent = status.kind.id(), readiness = ?next.readiness, "login done");
    next
}

fn apply_probe(status: &mut AgentStatus, outcome: anyhow::Result<ProbeReport>) {
    status.error = None;
    match outcome {
        Ok(report) => {
            status.readiness = match &report.session {
                SessionProbe::Ok => Readiness::Ready,
                SessionProbe::AuthRequired { detail } => {
                    status.error = detail.clone();
                    Readiness::NeedsLogin
                }
                SessionProbe::Failed { error, detail } => {
                    status.error = Some(match detail {
                        Some(d) => format!("{error}: {d}"),
                        None => error.clone(),
                    });
                    Readiness::Error
                }
            };
            if let Some(cmd) = report
                .auth_methods
                .iter()
                .find_map(|m| m.terminal_command.clone())
            {
                status.login_hint = cmd;
            }
            status.probe = Some(report);
        }
        Err(err) => {
            status.readiness = Readiness::Error;
            status.error = Some(err.to_string());
        }
    }
}

fn spec_for(adapter: &Adapter) -> Option<AgentSpec> {
    match adapter {
        Adapter::LocalScript { path } => Some(AgentSpec::node_script(path)),
        Adapter::Npx { package } => Some(AgentSpec::npx(package, &[])),
        Adapter::Cli { path } => Some(AgentSpec::binary(path, &["--acp"])),
        Adapter::Binary { path } => {
            let args = antigravity_release().map(|r| r.args).unwrap_or_default();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            Some(AgentSpec::binary(path, &args))
        }
        Adapter::NeedsDownload { .. } | Adapter::Missing { .. } => None,
    }
}

fn resolve_adapter(kind: AgentKind, cli: Option<&CliInfo>, adapters_dir: &Path) -> Adapter {
    match kind {
        AgentKind::ClaudeCode => match find_local_script(orchestra_acp::CLAUDE_ADAPTER_SCRIPT) {
            Some(p) => Adapter::LocalScript { path: p.to_string_lossy().into_owned() },
            None => Adapter::Npx { package: orchestra_acp::CLAUDE_ADAPTER.to_string() },
        },
        AgentKind::Codex => match find_local_script(CODEX_ADAPTER_SCRIPT) {
            Some(p) => Adapter::LocalScript { path: p.to_string_lossy().into_owned() },
            None => Adapter::Npx { package: CODEX_ADAPTER.to_string() },
        },
        AgentKind::Copilot => match cli {
            Some(c) => Adapter::Cli { path: c.path.clone() },
            None => Adapter::Missing { reason: "copilot CLI not found".to_string() },
        },
        AgentKind::Antigravity => {
            let Some(release) = antigravity_release() else {
                return Adapter::Missing { reason: "no Antigravity ACP server for this platform".to_string() };
            };
            match installed_antigravity_server(adapters_dir, &release) {
                Some(p) => Adapter::Binary { path: p.to_string_lossy().into_owned() },
                None => Adapter::NeedsDownload { release },
            }
        }
    }
}

fn antigravity_dir(adapters_dir: &Path, release: &AntigravityRelease) -> PathBuf {
    adapters_dir.join("antigravity").join(&release.version)
}

fn installed_antigravity_server(adapters_dir: &Path, release: &AntigravityRelease) -> Option<PathBuf> {
    find_file(&antigravity_dir(adapters_dir, release), &release.exe)
}

/// Where a download is, for a progress bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    /// Bytes received so far.
    pub received: u64,
    /// Total bytes, when the server said.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadPhase {
    Downloading,
    Unpacking,
    Done,
}

/// Download and unpack the Antigravity ACP server into `adapters_dir`.
///
/// Returns the server path. Safe to call again: an existing install is
/// returned without downloading.
pub async fn download_antigravity(adapters_dir: &Path) -> anyhow::Result<PathBuf> {
    download_antigravity_with(adapters_dir, |_| {}).await
}

/// [`download_antigravity`] with a progress callback, called as bytes
/// arrive (at most a few times per second) and at each phase change.
pub async fn download_antigravity_with(
    adapters_dir: &Path,
    mut progress: impl FnMut(DownloadProgress) + Send,
) -> anyhow::Result<PathBuf> {
    use futures::StreamExt;

    let release = antigravity_release()
        .ok_or_else(|| anyhow::anyhow!("no Antigravity ACP server is published for this platform"))?;
    if let Some(existing) = installed_antigravity_server(adapters_dir, &release) {
        return Ok(existing);
    }

    let dir = antigravity_dir(adapters_dir, &release);
    tracing::info!(url = %release.url, dir = %dir.display(), "downloading Antigravity ACP server");
    // A stalled connection must not leave the progress bar pending forever.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let response = client.get(&release.url).send().await?.error_for_status()?;
    let total = response.content_length();
    progress(DownloadProgress { phase: DownloadPhase::Downloading, received: 0, total });

    let mut bytes: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut stream = response.bytes_stream();
    let mut last_report = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        bytes.extend_from_slice(&chunk);
        // Throttle: a bar does not need every 16 KiB chunk.
        if last_report.elapsed() >= Duration::from_millis(120) {
            progress(DownloadProgress { phase: DownloadPhase::Downloading, received: bytes.len() as u64, total });
            last_report = std::time::Instant::now();
        }
    }
    let received = bytes.len() as u64;
    progress(DownloadProgress { phase: DownloadPhase::Unpacking, received, total: Some(received) });

    let dir_for_unpack = dir.clone();
    tokio::task::spawn_blocking(move || unpack_zip(&bytes, &dir_for_unpack)).await??;
    progress(DownloadProgress { phase: DownloadPhase::Done, received, total: Some(received) });

    let path = find_file(&dir, &release.exe)
        .ok_or_else(|| anyhow::anyhow!("{} was not in the archive", release.exe))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm)?;
    }
    tracing::info!(path = %path.display(), "Antigravity ACP server installed");
    Ok(path)
}

fn unpack_zip(bytes: &[u8], dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // `enclosed_name` refuses `..` and absolute paths.
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

/// Find an agent's CLI and ask it for its version.
pub async fn locate_cli(kind: AgentKind) -> Option<CliInfo> {
    let path = find_executable(kind.cli())?;
    let version = cli_version(&path).await;
    Some(CliInfo {
        path: path.to_string_lossy().into_owned(),
        version,
    })
}

async fn cli_version(path: &Path) -> Option<String> {
    let run = tokio::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned)
        }
        Ok(Ok(out)) => {
            tracing::debug!(path = %path.display(), status = ?out.status.code(), "--version failed");
            None
        }
        Ok(Err(err)) => {
            tracing::debug!(path = %path.display(), %err, "--version could not run");
            None
        }
        Err(_) => {
            tracing::warn!(path = %path.display(), "--version timed out");
            None
        }
    }
}

/// Search for an executable by base name.
///
/// Order: the process PATH, then (Windows) the user and machine PATH from the
/// registry, then known install directories. The first hit wins, `.exe`
/// preferred over `.cmd` shims on Windows.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(registry_path_dirs());
    dirs.extend(known_dirs(name));

    let candidates = executable_names(name);
    for dir in dirs {
        for cand in &candidates {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
}

/// Directories each agent's installer is known to use.
fn known_dirs(name: &str) -> Vec<PathBuf> {
    let home = home_dir();
    let mut dirs = Vec::new();

    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
        if let Some(h) = &home {
            dirs.push(h.join(".local").join("bin"));
        }
        if let Some(r) = &roaming {
            dirs.push(r.join("npm"));
        }
        if let Some(l) = &local {
            match name {
                "codex" => {
                    dirs.push(l.join("Programs").join("OpenAI").join("Codex").join("bin"));
                }
                "copilot" => {
                    // WinGet package directories carry a source suffix.
                    if let Ok(entries) = std::fs::read_dir(l.join("Microsoft").join("WinGet").join("Packages")) {
                        for e in entries.flatten() {
                            if e.file_name().to_string_lossy().starts_with("GitHub.Copilot") {
                                dirs.push(e.path());
                            }
                        }
                    }
                }
                "agy" => dirs.push(l.join("agy").join("bin")),
                _ => {}
            }
        }
        if name == "codex" {
            if let Some(h) = &home {
                dirs.push(h.join(".codex").join("packages").join("standalone").join("current").join("bin"));
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        if let Some(h) = &home {
            dirs.push(h.join(".local").join("bin"));
            dirs.push(h.join(".npm-global").join("bin"));
            dirs.push(h.join(".bun").join("bin"));
        }
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }

    dirs
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// The PATH as installers left it in the registry, which a running process
/// does not see until it restarts.
#[cfg(windows)]
fn registry_path_dirs() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let mut out = Vec::new();
    let sources = [
        (HKEY_CURRENT_USER, "Environment"),
        (HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
    ];
    for (hive, key) in sources {
        let Ok(k) = RegKey::predef(hive).open_subkey(key) else { continue };
        let Ok(raw) = k.get_value::<String, _>("Path") else { continue };
        out.extend(std::env::split_paths(&expand_env(&raw)));
    }
    out
}

#[cfg(not(windows))]
fn registry_path_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Expand `%NAME%` references the way `REG_EXPAND_SZ` values expect.
#[cfg(windows)]
fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_ids() {
        for k in AgentKind::ALL {
            assert_eq!(AgentKind::parse(k.id()), Some(k));
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.id()));
        }
    }

    #[test]
    fn release_exists_for_this_platform_or_is_none() {
        if let Some(r) = antigravity_release() {
            assert!(r.url.ends_with(".zip"));
            assert!(r.url.contains(ANTIGRAVITY_ACP_VERSION));
        }
    }

    #[cfg(windows)]
    #[test]
    fn expand_env_replaces_known_and_keeps_unknown() {
        std::env::set_var("ORCHESTRA_TEST_X", "val");
        assert_eq!(expand_env(r"%ORCHESTRA_TEST_X%\bin"), r"val\bin");
        assert_eq!(expand_env("%NOPE_NOT_SET%"), "%NOPE_NOT_SET%");
        assert_eq!(expand_env("50%"), "50%");
    }

    #[test]
    fn find_file_searches_nested_dirs() {
        let dir = std::env::temp_dir().join(format!("orchestra-agents-{}", std::process::id()));
        let nested = dir.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("srv.exe"), b"x").unwrap();
        assert_eq!(find_file(&dir, "srv.exe"), Some(nested.join("srv.exe")));
        assert_eq!(find_file(&dir, "other"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
