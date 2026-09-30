//! Putting divixi-server on a remote machine, and keeping it current.
//!
//! What docs/divixi-server.md asks a person to do by hand -- fetch the
//! binary, make it executable, put it somewhere, restart the server -- done
//! from the app over the SSH connection [`super::client`] already uses. No
//! new SSH stack: every step is a shell line run through `ssh` exactly as
//! `client::remote_line` runs one.
//!
//! ## Only the release binary, only x86_64 Linux
//!
//! Nothing is built on the remote machine: no apt, no Rust, no Node. The
//! release publishes one divixi-server asset,
//! `divixi-server-<date>-x86_64-linux`, uncompressed and about 207 MB (see
//! the `server` job in .github/workflows/release.yml). So an x86_64 Linux
//! remote can be installed to and anything else -- arm64, macOS -- is told
//! to follow the manual steps instead ([`Check::Unsupported`]). `uname -sm`
//! on the remote decides, never a guess from the host name.
//!
//! ## Which release
//!
//! The one *this app* is, when it is a release build. That is deliberate: a
//! remote instance is out of step with the app when their Rust sources hash
//! differently (`DIVIXI_BUILD`, see src-tauri/build.rs), and the release
//! workflow builds the app bundles and the server binary from the same
//! commit. Installing the app's own release is therefore the only choice
//! that makes the two agree. A development build of the app has no release
//! that matches it; it is offered the newest published one, and told the
//! builds will still differ.
//!
//! ## Where it goes, and how it is swapped
//!
//! ```text
//! ~/.divixi/server/v2026-09-30/divixi-server   the binary
//! ~/.divixi/server/current -> v2026-09-30      the pointer
//! ```
//!
//! A release never writes over another release's file, and the pointer moves
//! in one step (`ln -s` to a spare name, then `mv -T` onto it). The download
//! lands on `divixi-server.part` and is only renamed once its SHA-256
//! matches, so a connection that drops halfway cannot leave a half binary
//! installed. Renaming, not writing, is also what keeps `ETXTBSY` away: a
//! running executable cannot be written to, but it can be renamed away from.
//!
//! The order is stop, rename, swap the pointer, start -- in that order,
//! because the server that is running is the old binary and it has to be the
//! one that goes.
//!
//! ## Two ways the bytes travel
//!
//! `curl` or `wget` **on the remote**, straight from GitHub, is tried first:
//! 207 MB over the machine's own link is faster than 207 MB through the
//! app's SSH session, and the app stays out of the way. With no fetcher
//! there, or no route to GitHub from there, the app streams the bytes down
//! itself and pushes them over SSH (`cat > …`). Which of the two is
//! underway is in every progress event ([`Route`]), so a slow install says
//! why it is slow.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use super::client::{self, Host};
use crate::AppHandle;

/// The one platform the release publishes a server binary for.
const OS: &str = "Linux";
/// What `uname -m` calls it; a 64-bit x86 kernel may say either.
const ARCH: [&str; 2] = ["x86_64", "amd64"];

/// Where the app keeps the releases it installed, under the remote's home.
/// One shell word, quotes and all: a home folder with a space in it is as
/// ordinary as any other.
const STORE: &str = "\"$HOME\"/.divixi/server";
/// The host's `bin` setting once the app manages the server there.
pub const MANAGED_BIN: &str = "~/.divixi/server/current/divixi-server";

/// Room wanted beyond the binary's own size, so an install does not fill
/// the remote's home folder to the last byte.
const HEADROOM: u64 = 64 * 1024 * 1024;

// ----- shell words -----

/// A string as one single-quoted shell word.
///
/// Single quotes end every special meaning a shell has, `$` and `;` and a
/// space alike; the only thing they cannot hold is a single quote, which is
/// spliced in as `'\''`. Every path and every URL that reaches a remote
/// shell line goes through this or [`shell_path`].
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A remote path as one shell word, with a leading `~/` left for the remote
/// shell to expand.
///
/// `'~/x'` would be the literal four characters, so the tilde cannot simply
/// be quoted: it becomes `"$HOME"`, which is one word however many spaces
/// the home folder has in it, and the rest is single-quoted as usual.
pub fn shell_path(path: &str) -> String {
    if path == "~" {
        return "\"$HOME\"".to_string();
    }
    match path.strip_prefix("~/") {
        Some(rest) => format!("\"$HOME\"/{}", quote(rest)),
        None => quote(path),
    }
}

// ----- the release to install from -----

/// The published divixi-server binary of one release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asset {
    /// The release tag it belongs to.
    pub release: String,
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256, from the asset's `digest`.
    pub sha256: String,
}

/// A release as the GitHub API gives it, with only the fields used here.
#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>`, or absent for an asset uploaded before GitHub kept one.
    #[serde(default)]
    digest: Option<String>,
}

/// The server binary of a release, or why it cannot be installed from.
///
/// The name is the release workflow's: `divixi-server-<date>-x86_64-linux`.
/// Matched by both ends rather than by the exact date, so a release whose
/// asset is named from a different date than its tag still resolves.
fn pick_asset(release: &ReleaseJson) -> Result<Asset, String> {
    let found = release
        .assets
        .iter()
        .find(|a| a.name.starts_with("divixi-server-") && a.name.ends_with("-x86_64-linux"))
        .ok_or_else(|| format!("release {} has no divixi-server binary for x86_64 Linux", release.tag_name))?;
    let digest = found.digest.as_deref().unwrap_or_default();
    let sha256 = digest
        .strip_prefix("sha256:")
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            format!("GitHub gave no SHA-256 for {} (digest {digest:?}), so nothing can be checked against; the release needs a checksum asset", found.name)
        })?;
    if found.size == 0 {
        return Err(format!("GitHub gave no size for {}", found.name));
    }
    if !found.browser_download_url.starts_with("https://github.com/") {
        return Err(format!("{} is not published on github.com: {}", found.name, found.browser_download_url));
    }
    Ok(Asset { release: release.tag_name.clone(), name: found.name.clone(), url: found.browser_download_url.clone(), size: found.size, sha256 })
}

/// Which release the app would put there: its own when it is a release
/// build, the newest published one otherwise.
///
/// The distinction matters to the user, not only to the code: only the
/// app's own release can make the builds agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The release this app is: the server from it will match this app.
    App,
    /// The newest published release, because this app is a development
    /// build and no release matches it.
    Newest,
}

/// The release binary to install, and whether it can match this app.
pub async fn target(app_release: Option<&str>) -> Result<(Asset, Kind), String> {
    let (path, kind) = match app_release {
        Some(tag) => (format!("releases/tags/{tag}"), Kind::App),
        None => ("releases/latest".to_string(), Kind::Newest),
    };
    let client = crate::update::http()?;
    let url = format!("https://api.github.com/repos/{}/{path}", crate::update::REPO);
    let res = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("GitHub was not reached: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let spent = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).map(|v| v.trim() == "0").unwrap_or(false);
        if spent || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("GitHub refused the request as rate-limited; reading releases without signing in is allowed 60 times an hour".into());
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(match app_release {
                Some(tag) => format!("release {tag} is not published (a nightly release stays a draft until someone publishes it), so its server binary cannot be fetched"),
                None => "the repository has no published release yet".to_string(),
            });
        }
        return Err(format!("GitHub answered {status}"));
    }
    let body = res.bytes().await.map_err(|e| format!("GitHub's answer did not finish: {e}"))?;
    let release: ReleaseJson = serde_json::from_slice(&body).map_err(|e| format!("GitHub's answer was not a release: {e}"))?;
    Ok((pick_asset(&release)?, kind))
}

// ----- what is on the remote now -----

/// How the remote could fetch for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fetch {
    Curl,
    Wget,
}

/// What can hash a file there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hasher {
    Sha256sum,
    Shasum,
    Openssl,
}

/// How the server is started and stopped on that machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Launch {
    /// `setsid -f … serve`: the same shell form `client::remote_line`
    /// carries. Nothing on the remote has to be set up for it.
    Setsid,
    /// A systemd user service runs it. `exec_start` is the binary that unit
    /// names, which is what decides whether this app may install behind it.
    Systemd { exec_start: String },
}

/// The remote as one round trip of `ssh` found it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Probe {
    pub home: String,
    /// `uname -s` and `uname -m`.
    pub os: String,
    pub arch: String,
    /// Something runnable at the host's configured `bin` path.
    pub bin: bool,
    /// The release the `current` pointer names, when the app put one there.
    pub installed: Option<String>,
    /// A divixi-server of this user is running.
    pub running: bool,
    pub fetch: Option<Fetch>,
    pub hasher: Option<Hasher>,
    /// A systemd user service for divixi-server, enabled or active.
    pub systemd: Option<String>,
    /// Free bytes under the home folder, when `df` said.
    pub free: Option<u64>,
}

impl Probe {
    /// x86_64 Linux: the only platform with a published binary.
    pub fn supported(&self) -> bool {
        self.os == OS && ARCH.contains(&self.arch.as_str())
    }

    /// How the server is launched there.
    pub fn launch(&self) -> Launch {
        match &self.systemd {
            Some(exec_start) => Launch::Systemd { exec_start: exec_start.clone() },
            None => Launch::Setsid,
        }
    }
}

/// One line per fact, `key=value`, so the parse cannot be confused by a
/// locale or by a tool's own wording.
fn probe_script(bin: &str) -> String {
    let bin = shell_path(bin);
    format!(
        r#"printf 'home=%s\n' "$HOME"
printf 'os=%s\n' "$(uname -s 2>/dev/null)"
printf 'arch=%s\n' "$(uname -m 2>/dev/null)"
if [ -x {bin} ]; then printf 'bin=yes\n'; fi
p=$(readlink {STORE}/current 2>/dev/null); if [ -n "$p" ]; then printf 'installed=%s\n' "${{p##*/}}"; fi
if pgrep -u "$(id -u)" -x divixi-server >/dev/null 2>&1; then printf 'running=yes\n'; fi
if command -v curl >/dev/null 2>&1; then printf 'fetch=curl\n'; elif command -v wget >/dev/null 2>&1; then printf 'fetch=wget\n'; fi
if command -v sha256sum >/dev/null 2>&1; then printf 'hasher=sha256sum\n'; elif command -v shasum >/dev/null 2>&1; then printf 'hasher=shasum\n'; elif command -v openssl >/dev/null 2>&1; then printf 'hasher=openssl\n'; fi
if command -v systemctl >/dev/null 2>&1; then
  if systemctl --user is-enabled divixi-server >/dev/null 2>&1 || systemctl --user is-active divixi-server >/dev/null 2>&1; then
    printf 'systemd=%s\n' "$(systemctl --user show -p ExecStart --value divixi-server 2>/dev/null | tr '\n' ' ')"
  fi
fi
printf 'free=%s\n' "$(df -Pk "$HOME" 2>/dev/null | awk 'NR==2 {{print $4}}')"
"#
    )
}

/// The `path=…` of a systemd `ExecStart`, or the line itself when it is
/// already a bare path.
///
/// `systemctl show -p ExecStart` prints a record, not a command line:
/// `{ path=/home/u/.local/bin/divixi-server ; argv[]=… ; … }`. What decides
/// whether this app may install behind that unit is the path.
fn exec_start_path(shown: &str) -> String {
    for part in shown.split(';') {
        // The record opens with `{ `, so the first part carries it.
        let part = part.trim().trim_start_matches('{').trim();
        if let Some(rest) = part.strip_prefix("path=") {
            return rest.trim().to_string();
        }
    }
    shown.trim().trim_start_matches('{').trim().to_string()
}

/// The probe's output as facts. Unknown keys are ignored, so an older or a
/// newer script on either side degrades rather than fails.
pub fn parse_probe(out: &str) -> Probe {
    let mut p = Probe::default();
    for line in out.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "home" => p.home = value.to_string(),
            "os" => p.os = value.to_string(),
            "arch" => p.arch = value.to_string(),
            "bin" => p.bin = value == "yes",
            "installed" => p.installed = (!value.is_empty()).then(|| value.to_string()),
            "running" => p.running = value == "yes",
            "fetch" => {
                p.fetch = match value {
                    "curl" => Some(Fetch::Curl),
                    "wget" => Some(Fetch::Wget),
                    _ => None,
                }
            }
            "hasher" => {
                p.hasher = match value {
                    "sha256sum" => Some(Hasher::Sha256sum),
                    "shasum" => Some(Hasher::Shasum),
                    "openssl" => Some(Hasher::Openssl),
                    _ => None,
                }
            }
            "systemd" => {
                let path = exec_start_path(value);
                p.systemd = (!path.is_empty()).then_some(path);
            }
            "free" => p.free = value.parse::<u64>().ok().map(|kb| kb.saturating_mul(1024)),
            _ => {}
        }
    }
    p
}

// ----- the verdict -----

/// Why the divixi-server on the remote is not the one to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// A release older than the one the app would install.
    OlderRelease,
    /// It is answering and reports another build than this app's: some
    /// commands may not match.
    OtherBuild,
    /// It is answering and reports **no** build at all. Only a
    /// divixi-server from before `/api/health` carried that field does
    /// that, so this is not "a different build" -- it is far older than
    /// this app. Telling the two apart is the whole point of this case:
    /// "update it, it is too old" is what the user can act on, and
    /// "its build differs" is not.
    TooOld,
}

/// What the app found, and what it may do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// `uname` says a platform the release publishes nothing for: the
    /// manual steps are the only way, and are what the user is shown.
    Unsupported { os: String, arch: String },
    /// No divixi-server there at all.
    Missing { target: String, source: Kind, size: u64 },
    /// One is there and worth replacing.
    Outdated { why: Why, installed: Option<String>, target: String, source: Kind, size: u64 },
    /// As current as anything here can tell. `running` is false for a
    /// server that is installed and simply down: then the thing to offer is
    /// starting it, not installing 207 MB over it.
    Current { installed: Option<String>, running: bool },
    /// A divixi-server is there that this app did not put there, and
    /// nothing says it is old. Left alone unless the user says otherwise.
    Unmanaged { target: String, source: Kind, size: u64, running: bool },
    /// A systemd user service starts divixi-server from a path of its own.
    /// Installing behind it would put a new binary somewhere that unit
    /// never looks, and the old one would come straight back up, so the
    /// app stops and says which line to change.
    Managed { exec_start: String, want: String },
    /// There is no release to install from, and why not.
    NoTarget { detail: String },
    /// Not an SSH host: an instance reached at an address has no shell for
    /// the app to run anything in.
    NoShell,
}

/// What the connected instance's `/api/health` said, when it is connected.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Seen {
    /// `DIVIXI_BUILD` there. Empty from a server older than that field.
    pub build: String,
    /// Its release tag. Empty from a server older than that field, and
    /// from one built from a working copy.
    pub release: String,
}

/// The whole judgement, as one pure function over what was found.
///
/// `target` is `Err` when GitHub could not be asked; the platform is still
/// judged first, because an arm64 remote is no more installable with a
/// release in hand than without one.
pub fn check(probe: &Probe, seen: Option<&Seen>, target: &Result<(Asset, Kind), String>, app_build: &str) -> Check {
    if !probe.supported() {
        return Check::Unsupported { os: probe.os.clone(), arch: probe.arch.clone() };
    }
    if let Some(exec_start) = &probe.systemd {
        // The pointer is what the app can move; a unit naming anything else
        // would keep launching the binary beside it.
        if !exec_start.ends_with("/.divixi/server/current/divixi-server") {
            return Check::Managed { exec_start: exec_start.clone(), want: MANAGED_BIN.to_string() };
        }
    }
    let (asset, kind) = match target {
        Ok(t) => t,
        Err(detail) => return Check::NoTarget { detail: detail.clone() },
    };
    let (release, source, size) = (asset.release.clone(), *kind, asset.size);
    // Which release is on that machine: the pointer the app wrote, or -- for
    // one it did not install -- the tag the server reports of itself, which
    // a divixi-server built from a release does. Either is a date to compare;
    // a server built from a working copy reports neither.
    let there = probe
        .installed
        .clone()
        .or_else(|| seen.map(|s| s.release.clone()).filter(|r| !r.is_empty()));
    let out = |why: Why| Check::Outdated { why, installed: there.clone(), target: release.clone(), source, size };

    // Nothing there: the configured path runs nothing, the app's own pointer
    // names none, and nothing is answering from somewhere else either.
    if !probe.bin && probe.installed.is_none() && seen.is_none() {
        return Check::Missing { target: release, source, size };
    }
    // Older than the release the app would put there now. Compared as dates,
    // never as text (v2026-09-9 would sort above v2026-09-10).
    if let Some(installed) = &there {
        match (crate::update::parse(installed), crate::update::parse(&release)) {
            (Some(mine), Some(theirs)) if theirs > mine => return out(Why::OlderRelease),
            (Some(mine), Some(theirs)) if theirs == mine => return Check::Current { installed: Some(installed.clone()), running: probe.running },
            // The pointer names something that is not a release tag: it was
            // not this app that wrote it. Judged below, by what is running.
            _ => {}
        }
    }
    // What the server itself says, when it is answering.
    if let Some(seen) = seen {
        if seen.build.is_empty() {
            return out(Why::TooOld);
        }
        if seen.build != app_build {
            return out(Why::OtherBuild);
        }
        return Check::Current { installed: there.clone(), running: probe.running };
    }
    match &there {
        Some(installed) => Check::Current { installed: Some(installed.clone()), running: probe.running },
        // A binary at the configured path, not answering, nothing naming
        // its release: built from source, or installed by hand. Nothing
        // here can call that old.
        None => Check::Unmanaged { target: release, source, size, running: probe.running },
    }
}

// ----- progress -----

/// Where an install is. The shape of `agents`' `DownloadProgress`
/// (phase / received / total), with what only this can say beside it.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// Which remote instance.
    pub host: String,
    pub phase: Phase,
    /// Where it had got to, when `phase` is [`Phase::Failed`]. A failure is
    /// never shown as a bare "the install failed": the step it failed in and
    /// what the remote said about it are both here.
    pub at: Option<Phase>,
    /// Bytes on the remote's disk so far.
    pub received: u64,
    /// The asset's size, which GitHub always gives; `None` only if it ever
    /// stops doing so.
    pub total: Option<u64>,
    /// Which way the bytes travel, while they are travelling.
    pub route: Option<Route>,
    pub release: Option<String>,
    /// What went wrong, in the remote's own words, with `phase` saying
    /// where. Never flattened into "the install failed".
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Asking the remote what it is and what it has.
    Checking,
    Downloading,
    Verifying,
    /// Stopping the old server, renaming, moving the pointer.
    Installing,
    Starting,
    Done,
    Failed,
    Cancelled,
}

/// Which way the 207 MB travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// `curl` or `wget` on the remote, straight from GitHub.
    Remote,
    /// The app fetches and pushes the bytes over its SSH session.
    Ssh,
}

/// Installs underway, by host id. An install is one at a time per host, and
/// each watches its own flag so [`remote_server_cancel`] can stop it.
static JOBS: std::sync::OnceLock<parking_lot::Mutex<HashMap<String, Arc<AtomicBool>>>> = std::sync::OnceLock::new();

fn jobs() -> &'static parking_lot::Mutex<HashMap<String, Arc<AtomicBool>>> {
    JOBS.get_or_init(Default::default)
}

/// The install's own cancel flag, or `None` when one is already underway.
fn begin(id: &str) -> Option<Arc<AtomicBool>> {
    let mut open = jobs().lock();
    if open.contains_key(id) {
        return None;
    }
    let flag = Arc::new(AtomicBool::new(false));
    open.insert(id.to_string(), flag.clone());
    Some(flag)
}

fn end(id: &str) {
    jobs().lock().remove(id);
}

struct Say<'a> {
    app: &'a AppHandle,
    host: String,
    release: String,
    total: u64,
}

impl Say<'_> {
    fn at(&self, phase: Phase, received: u64, route: Option<Route>) {
        self.emit(Progress {
            host: self.host.clone(),
            phase,
            at: None,
            received,
            total: Some(self.total).filter(|t| *t > 0),
            route,
            release: (!self.release.is_empty()).then(|| self.release.clone()),
            error: None,
        });
    }

    fn failed(&self, at: Phase, error: String) {
        self.emit(Progress {
            host: self.host.clone(),
            phase: Phase::Failed,
            at: Some(at),
            received: 0,
            total: Some(self.total).filter(|t| *t > 0),
            route: None,
            release: (!self.release.is_empty()).then(|| self.release.clone()),
            error: Some(error),
        });
    }

    fn emit(&self, p: Progress) {
        let _ = self.app.emit("server_install", p);
    }
}

// ----- the remote shell lines -----

/// The versioned folder and the two names in it, as shell words.
///
/// Built from `$HOME` on the remote rather than from the home path the
/// probe read, so a home folder with a space in it needs nothing special:
/// `"$HOME"` is one word whatever is in it.
fn paths(release: &str) -> (String, String, String) {
    let dir = format!("{STORE}/{}", quote(release));
    (STORE.to_string(), format!("{dir}/'divixi-server'"), format!("{dir}/'divixi-server.part'"))
}

/// Fetch the asset on the remote, reporting the part file's size as it
/// grows.
///
/// The fetcher runs in the background and the loop is the deadman switch:
/// its `printf` goes to the app through ssh, so when the app kills ssh the
/// write fails, the shell takes SIGPIPE, the trap runs, and the half file
/// goes. The app removes it again afterwards regardless -- a cancel must
/// not leave 100 MB of nothing on someone's machine.
fn fetch_script(release: &str, url: &str, how: Fetch) -> String {
    let (_, _, part) = paths(release);
    let dir = format!("{STORE}/{}", quote(release));
    let url = quote(url);
    let get = match how {
        Fetch::Curl => format!("curl -fsS -L --retry 2 --retry-delay 2 --connect-timeout 20 -o {part} {url} 2>\"$E\""),
        Fetch::Wget => format!("wget -q --tries=2 --timeout=20 -O {part} {url} 2>\"$E\""),
    };
    format!(
        r#"mkdir -p {dir} || exit 1
rm -f {part}
E=$(mktemp 2>/dev/null || echo "/tmp/divixi-fetch.$$")
trap 'kill $F 2>/dev/null; rm -f {part} "$E"; exit 1' HUP INT TERM PIPE
{get} &
F=$!
while kill -0 $F 2>/dev/null; do
  printf '#got %s\n' "$(if [ -f {part} ]; then wc -c < {part}; else echo 0; fi)"
  sleep 1
done
wait $F; c=$?
printf '#got %s\n' "$(if [ -f {part} ]; then wc -c < {part}; else echo 0; fi)"
if [ "$c" -ne 0 ]; then sed 's/^/#err /' "$E" 2>/dev/null; rm -f {part}; fi
rm -f "$E"
printf '#exit %s\n' "$c"
exit "$c"
"#
    )
}

/// Take the bytes the app pushes on stdin. A dropped connection leaves a
/// short file, which the SHA-256 check then refuses.
fn push_script(release: &str) -> String {
    let (_, _, part) = paths(release);
    let dir = format!("{STORE}/{}", quote(release));
    format!(
        r#"mkdir -p {dir} || exit 1
rm -f {part}
if cat > {part}; then
  printf '#got %s\n' "$(if [ -f {part} ]; then wc -c < {part}; else echo 0; fi)"
  printf '#exit 0\n'
else
  rm -f {part}
  printf '#exit 1\n'
  exit 1
fi
"#
    )
}

/// The size and SHA-256 of one of the two files, as the remote sees them.
fn digest_script(release: &str, part: bool, hasher: Hasher) -> String {
    let (_, bin, tmp) = paths(release);
    let file = if part { tmp } else { bin };
    let hash = match hasher {
        Hasher::Sha256sum => format!("sha256sum {file}"),
        Hasher::Shasum => format!("shasum -a 256 {file}"),
        Hasher::Openssl => format!("openssl dgst -sha256 -r {file}"),
    };
    format!(
        r#"if [ ! -f {file} ]; then printf '#size 0\n'; exit 0; fi
printf '#size %s\n' "$(wc -c < {file})"
printf '#sha %s\n' "$({hash} 2>/dev/null | cut -d' ' -f1)"
"#
    )
}

/// Stop the old server, put the new binary in place, move the pointer, and
/// bring the server back.
///
/// In that order and for that reason: the binary that is running is the old
/// one, so it goes first; the new file arrives by rename, which a running
/// executable does not refuse (writing to one would, with `ETXTBSY`); and
/// the pointer moves in one step so nothing ever sees it half-changed.
/// How the server is stopped and started on that machine, as two shell
/// lines. One place, so the install and [`start_script`] cannot drift into
/// starting it two different ways.
///
/// `bin` is the shell word for the binary to run: the pointer where the app
/// installed one, the host's own path where it did not. `path` is the
/// instance's "Remote PATH" setting, put in front as `client::remote_line`
/// puts it, so the agents are found the same way either way.
fn stop_and_start(launch: &Launch, path: &str, bin: &str) -> (String, String) {
    match launch {
        Launch::Systemd { .. } => (
            "systemctl --user stop divixi-server >/dev/null 2>&1".to_string(),
            "systemctl --user start divixi-server || exit 1".to_string(),
        ),
        Launch::Setsid => {
            let path = if path.is_empty() { String::new() } else { format!("PATH={path}:\"$PATH\"; export PATH; ") };
            (
                "pkill -u \"$(id -u)\" -x divixi-server >/dev/null 2>&1 || true".to_string(),
                format!("{path}setsid -f {bin} serve >/dev/null 2>&1 </dev/null || exit 1"),
            )
        }
    }
}

/// Start the server without touching anything else: it is installed there
/// and simply not running (the machine was rebooted, someone killed it).
///
/// The app cannot bring a stopped server up on its own while connecting --
/// `client::connect` waits for the tunnel to answer before it ever runs the
/// line that would start one -- so without this the person is left with
/// "did not answer through the tunnel" and nothing to press.
fn start_script(launch: &Launch, path: &str, bin: &str) -> String {
    let (_, start) = stop_and_start(launch, path, bin);
    format!(
        r#"{start}
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if pgrep -u "$(id -u)" -x divixi-server >/dev/null 2>&1; then printf '#up
'; break; fi
  sleep 1
done
"#
    )
}

fn install_script(release: &str, launch: &Launch, path: &str) -> String {
    let (store, bin, part) = paths(release);
    let rel = quote(release);
    let (stop, start) = stop_and_start(launch, path, &format!("{store}/'current'/'divixi-server'"));
    format!(
        r#"if [ ! -f {part} ] && [ ! -f {bin} ]; then echo 'nothing to install' >&2; exit 1; fi
{stop}
for i in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -u "$(id -u)" -x divixi-server >/dev/null 2>&1 || break
  sleep 1
done
if [ -f {part} ]; then chmod 755 {part} || exit 1; mv -f {part} {bin} || exit 1; fi
chmod 755 {bin} || exit 1
ln -sfn {rel} {store}/'.current.new' || exit 1
if mv -T {store}/'.current.new' {store}/'current' 2>/dev/null; then
  :
else
  rm -f {store}/'current' {store}/'.current.new' || exit 1
  ln -s {rel} {store}/'current' || exit 1
fi
printf '#swapped\n'
{start}
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if pgrep -u "$(id -u)" -x divixi-server >/dev/null 2>&1; then printf '#up\n'; break; fi
  sleep 1
done
printf '#done\n'
"#
    )
}

/// Take the half-downloaded file away. Run after a cancel and after any
/// failure, and never allowed to fail the operation itself.
fn cleanup_script(release: &str) -> String {
    let (_, _, part) = paths(release);
    format!("rm -f {part}\n")
}

// ----- running them -----

/// Run one shell line on the remote and give back its output, or say why
/// not in the remote's own words.
async fn run(host: &Host, script: &str) -> Result<String, String> {
    let out = client::ssh()
        .arg(&host.ssh)
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("could not run ssh: {e}"))?;
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() { format!("the remote command ended {}", out.status) } else { err.to_string() });
    }
    Ok(said)
}

/// Ask the remote what it is and what it has.
pub async fn probe(host: &Host) -> Result<Probe, String> {
    let out = run(host, &probe_script(&host.bin)).await?;
    Ok(parse_probe(&out))
}

/// The `#sha`/`#size` of a digest run.
fn read_digest(out: &str) -> (u64, String) {
    let mut size = 0;
    let mut sha = String::new();
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("#size ") {
            size = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("#sha ") {
            sha = v.trim().to_ascii_lowercase();
        }
    }
    (size, sha)
}

/// Whether a file on the remote is the asset, byte for byte.
///
/// Both are checked, not only the hash: a truncated download has a
/// different hash too, but the size is what can be said about it.
fn matches(asset: &Asset, size: u64, sha: &str) -> Result<(), String> {
    if size == 0 {
        return Err("the file is not there".into());
    }
    if size != asset.size {
        return Err(format!("it is {size} bytes, not {} — the download did not finish", asset.size));
    }
    if sha.is_empty() {
        return Err("the remote could not hash it".into());
    }
    if sha != asset.sha256 {
        return Err(format!("its SHA-256 is {sha}, and GitHub says {}", asset.sha256));
    }
    Ok(())
}

/// Have the remote fetch the asset itself, telling the app how far it is.
///
/// `Ok(false)` means the remote could not reach GitHub (or has no fetcher
/// that worked): the caller pushes the bytes instead. An error is a failure
/// of the attempt itself, which is not something to fall back from.
async fn fetch_there(host: &Host, asset: &Asset, how: Fetch, say: &Say<'_>, cancel: &AtomicBool) -> Result<bool, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = client::ssh()
        .arg(&host.ssh)
        .arg(fetch_script(&asset.release, &asset.url, how))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run ssh: {e}"))?;
    let stdout = child.stdout.take().ok_or("ssh gave no output")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut code: Option<i32> = None;
    let mut said = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(String::new());
        }
        // Progress arrives once a second; the wait is what lets a cancel
        // be noticed in between.
        let line = match tokio::time::timeout(Duration::from_millis(500), lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(format!("the remote stopped talking: {e}")),
            Err(_) => continue,
        };
        if let Some(v) = line.strip_prefix("#got ") {
            if let Ok(n) = v.trim().parse::<u64>() {
                say.at(Phase::Downloading, n, Some(Route::Remote));
            }
        } else if let Some(v) = line.strip_prefix("#exit ") {
            code = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("#err ") {
            said.push(v.to_string());
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        use tokio::io::AsyncReadExt;
        let _ = e.read_to_string(&mut err).await;
    }
    if code == Some(0) && status.success() {
        return Ok(true);
    }
    // curl and wget say "could not resolve host", "connection refused",
    // "403": all of them mean this machine cannot get it, and the app can.
    let why = if said.is_empty() { err.trim().to_string() } else { said.join("\n") };
    tracing::info!(host = %host.ssh, why = %why, "the remote could not fetch divixi-server itself; pushing it over SSH");
    Ok(false)
}

/// Fetch the asset here and push it over SSH.
async fn push(host: &Host, asset: &Asset, say: &Say<'_>, cancel: &AtomicBool) -> Result<(), String> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    let http = reqwest::Client::builder()
        .user_agent("divixi")
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let res = http.get(&asset.url).send().await.map_err(|e| format!("GitHub was not reached: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GitHub answered {} for {}", res.status(), asset.name));
    }
    let mut child = client::ssh()
        .arg(&host.ssh)
        .arg(push_script(&asset.release))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run ssh: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("ssh took no input")?;
    let mut stream = res.bytes_stream();
    let mut sent = 0u64;
    let mut last = std::time::Instant::now();
    let mut failed = None;
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            failed = Some(String::new());
            break;
        }
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                failed = Some(format!("the download stopped: {e}"));
                break;
            }
        };
        if let Err(e) = stdin.write_all(&chunk).await {
            failed = Some(format!("the remote stopped taking the bytes: {e}"));
            break;
        }
        sent += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(200) {
            say.at(Phase::Downloading, sent, Some(Route::Ssh));
            last = std::time::Instant::now();
        }
    }
    // The remote's `cat` ends on end of input, never before.
    let closed = stdin.shutdown().await;
    drop(stdin);
    if let Some(why) = failed {
        let _ = child.kill().await;
        return Err(why);
    }
    closed.map_err(|e| format!("the remote stopped taking the bytes: {e}"))?;
    say.at(Phase::Downloading, sent, Some(Route::Ssh));
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("the remote did not keep the bytes: {}", err.trim()));
    }
    Ok(())
}

/// Put the release on the remote, from nothing or over an older one.
///
/// Every step says where it is through `server_install` events; the return
/// is the release now installed. A failure names the phase it failed in and
/// carries what the remote said, because "the install failed" is not
/// something anybody can act on.
pub async fn install(app: &AppHandle, id: &str) -> Result<String, String> {
    let host = client::hosts(app).into_iter().find(|h| h.id == id).ok_or("no such remote instance")?;
    if host.kind == "direct" {
        return Err("an instance reached at an address has no shell for the app to install through; use an SSH host, or follow docs/divixi-server.md there".into());
    }
    let Some(cancel) = begin(id) else {
        return Err("that instance is already being installed to".into());
    };
    let out = install_inner(app, &host, &cancel).await;
    end(id);
    out
}

async fn install_inner(app: &AppHandle, host: &Host, cancel: &AtomicBool) -> Result<String, String> {
    let mut say = Say { app, host: host.id.clone(), release: String::new(), total: 0 };
    say.at(Phase::Checking, 0, None);

    let probe = match probe(host).await {
        Ok(p) => p,
        Err(e) => {
            say.failed(Phase::Checking, e.clone());
            return Err(e);
        }
    };
    let step = |phase: Phase, say: &Say<'_>, e: String| {
        say.failed(phase, e.clone());
        e
    };
    if !probe.supported() {
        return Err(step(Phase::Checking, &say, format!("that machine is {} {}, and the release publishes divixi-server for x86_64 Linux only", probe.os, probe.arch)));
    }
    let launch = probe.launch();
    if let Launch::Systemd { exec_start } = &launch {
        if !exec_start.ends_with("/.divixi/server/current/divixi-server") {
            return Err(step(
                Phase::Checking,
                &say,
                format!("a systemd user service there starts divixi-server from {exec_start}, which the app does not install over. Point its ExecStart at {MANAGED_BIN} (systemctl --user daemon-reload) and try again"),
            ));
        }
    }
    let target = target(crate::update::release()).await;
    let (asset, _kind) = match target {
        Ok(t) => t,
        Err(e) => return Err(step(Phase::Checking, &say, e)),
    };
    say.release = asset.release.clone();
    say.total = asset.size;
    if let Some(free) = probe.free {
        let want = asset.size + HEADROOM;
        if free < want {
            return Err(step(
                Phase::Checking,
                &say,
                // Megabytes as the release counts them, as on the page.
                format!("{} has {} MB free and this needs about {} MB", probe.home, free / 1_000_000, want / 1_000_000),
            ));
        }
    }

    // Already there and right: an install asked for twice costs one hash,
    // not 207 MB.
    say.at(Phase::Verifying, 0, None);
    let mut have = false;
    if let Some(hasher) = probe.hasher {
        if let Ok(out) = run(host, &digest_script(&asset.release, false, hasher)).await {
            let (size, sha) = read_digest(&out);
            have = matches(&asset, size, &sha).is_ok();
        }
    }

    if !have {
        // The remote's own link first: 207 MB through the app's SSH session
        // is the slower way and the one that keeps the app busy.
        let mut route = Route::Ssh;
        let mut done = false;
        if let (Some(how), Some(_)) = (probe.fetch, probe.hasher) {
            say.at(Phase::Downloading, 0, Some(Route::Remote));
            match fetch_there(host, &asset, how, &say, cancel).await {
                Ok(true) => {
                    route = Route::Remote;
                    done = true;
                }
                Ok(false) => {}
                Err(e) => return Err(stop_here(host, &asset, &say, Phase::Downloading, e, cancel).await),
            }
        }
        if !done {
            say.at(Phase::Downloading, 0, Some(Route::Ssh));
            if let Err(e) = push(host, &asset, &say, cancel).await {
                return Err(stop_here(host, &asset, &say, Phase::Downloading, e, cancel).await);
            }
        }
        tracing::info!(host = %host.ssh, release = %asset.release, ?route, "divixi-server downloaded on the remote");

        say.at(Phase::Verifying, asset.size, Some(route));
        match probe.hasher {
            Some(hasher) => {
                let out = match run(host, &digest_script(&asset.release, true, hasher)).await {
                    Ok(o) => o,
                    Err(e) => return Err(stop_here(host, &asset, &say, Phase::Verifying, e, cancel).await),
                };
                let (size, sha) = read_digest(&out);
                if let Err(why) = matches(&asset, size, &sha) {
                    return Err(stop_here(host, &asset, &say, Phase::Verifying, format!("what arrived is not the published binary: {why}"), cancel).await);
                }
            }
            // No sha256sum, shasum or openssl there, so the remote route
            // was never taken and the app pushed the bytes it read from
            // GitHub itself. The size is all that can be checked there.
            None => {
                let part = paths(&asset.release).2;
                let out = run(host, &format!("printf '#size %s\\n' \"$(if [ -f {part} ]; then wc -c < {part}; else echo 0; fi)\"\n")).await;
                let (size, _) = read_digest(&out.unwrap_or_default());
                if size != asset.size {
                    return Err(stop_here(host, &asset, &say, Phase::Verifying, format!("what arrived is {size} bytes, not {}", asset.size), cancel).await);
                }
            }
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(stop_here(host, &asset, &say, Phase::Cancelled, String::new(), cancel).await);
    }

    say.at(Phase::Installing, asset.size, None);
    let out = match run(host, &install_script(&asset.release, &launch, &host.path)).await {
        Ok(o) => o,
        Err(e) => return Err(stop_here(host, &asset, &say, Phase::Installing, e, cancel).await),
    };
    if !out.contains("#swapped") {
        return Err(stop_here(host, &asset, &say, Phase::Installing, format!("the remote did not put it in place: {}", out.trim()), cancel).await);
    }
    say.at(Phase::Starting, asset.size, None);
    if !out.contains("#up") {
        let e = "it is installed, but divixi-server did not come up. Run it there to see why: ~/.divixi/server/current/divixi-server serve".to_string();
        say.failed(Phase::Starting, e.clone());
        return Err(e);
    }
    // The connection starts the server from this path from now on.
    if host.bin != MANAGED_BIN {
        if let Err(e) = client::set_bin(app, &host.id, MANAGED_BIN) {
            tracing::warn!(%e, "installed divixi-server but could not point the instance at it");
        }
    }
    tracing::info!(host = %host.ssh, release = %asset.release, "divixi-server installed");
    say.at(Phase::Done, asset.size, None);
    Ok(asset.release)
}

/// Stop, take the half file away, and say what happened -- as a cancel when
/// that is what it was.
async fn stop_here(host: &Host, asset: &Asset, say: &Say<'_>, phase: Phase, why: String, cancel: &AtomicBool) -> String {
    // Best effort, and never allowed to replace the real failure: an
    // unreachable machine cannot be tidied either.
    let _ = tokio::time::timeout(Duration::from_secs(20), run(host, &cleanup_script(&asset.release))).await;
    if cancel.load(Ordering::Relaxed) || why.is_empty() {
        say.at(Phase::Cancelled, 0, None);
        return "the install was cancelled".to_string();
    }
    say.failed(phase, why.clone());
    why
}

// ----- commands (this app's own page; never reachable remotely) -----

/// A "do not ask again" for one instance's updates.
fn mute_key(id: &str) -> String {
    format!("remote.install.mute.{id}")
}

/// What the check found, with the two things only the app knows beside it.
#[derive(Serialize)]
pub struct CheckView {
    #[serde(flatten)]
    pub check: Check,
    /// The user asked not to be offered this instance's updates again.
    pub muted: bool,
    /// How the server is started there, once that is known.
    pub launch: Option<Launch>,
}

/// Judge the remote's divixi-server: missing, older, or current.
///
/// One `ssh` round trip and one unauthenticated GitHub request, on a press
/// or when a connection failed -- never on a timer.
#[tauri::command]
pub async fn remote_server_check(app: AppHandle, id: String) -> Result<CheckView, String> {
    let host = client::hosts(&app).into_iter().find(|h| h.id == id).ok_or("no such remote instance")?;
    let muted = super::setting(&app, &mute_key(&id)).as_deref() == Some("true");
    if host.kind == "direct" {
        return Ok(CheckView { check: Check::NoShell, muted, launch: None });
    }
    let probe = probe(&host).await?;
    let target = target(crate::update::release()).await;
    let seen = client::seen(&app, &id).await;
    let check = check(&probe, seen.as_ref(), &target, client::BUILD);
    Ok(CheckView { check, muted, launch: Some(probe.launch()) })
}

/// Install (or replace) divixi-server on the remote. Progress goes out as
/// `server_install` events.
#[tauri::command]
pub async fn remote_server_install(app: AppHandle, id: String) -> Result<String, String> {
    install(&app, &id).await
}

/// Start the divixi-server that is already on that machine.
///
/// For a server that is installed and down -- the machine was rebooted and
/// nothing keeps it up. Nothing is downloaded and nothing is replaced; the
/// way it is started is the way that machine starts it (its systemd user
/// service, or `setsid` as `client::remote_line` does).
#[tauri::command]
pub async fn remote_server_start(app: AppHandle, id: String) -> Result<(), String> {
    let host = client::hosts(&app).into_iter().find(|h| h.id == id).ok_or("no such remote instance")?;
    if host.kind == "direct" {
        return Err("an instance reached at an address has no shell for the app to start it through".into());
    }
    let probe = probe(&host).await?;
    // Already up: nothing to do, and nothing to report as a failure either.
    if probe.running {
        return Ok(());
    }
    // The pointer where the app installed one, the host's own path where it
    // did not (a server built from source there).
    let bin = match probe.installed.is_some() {
        true => format!("{STORE}/'current'/'divixi-server'"),
        false => shell_path(&host.bin),
    };
    let out = run(&host, &start_script(&probe.launch(), &host.path, &bin)).await?;
    if !out.contains("#up") {
        return Err(format!("divixi-server did not come up. Run it there to see why: {} serve", host.bin));
    }
    tracing::info!(host = %host.ssh, "divixi-server started on the remote");
    Ok(())
}

/// Stop an install underway. The half-downloaded file goes with it.
#[tauri::command]
pub fn remote_server_cancel(id: String) {
    if let Some(flag) = jobs().lock().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Remember (or forget) that this instance's updates are not to be offered.
#[tauri::command]
pub fn remote_server_mute(app: AppHandle, id: String, muted: bool) -> Result<(), String> {
    super::set(&app, &mute_key(&id), if muted { "true" } else { "false" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> Asset {
        Asset {
            release: "v2026-09-30".into(),
            name: "divixi-server-2026-09-30-x86_64-linux".into(),
            url: "https://github.com/jaemin93/divixi/releases/download/v2026-09-30/divixi-server-2026-09-30-x86_64-linux".into(),
            size: 207_108_624,
            sha256: "d856519c026482a0fcfcfb09ec8673c0692775192c0f61cf053fc46da523c35f".into(),
        }
    }

    // ----- quoting -----

    #[test]
    fn a_path_with_a_space_stays_one_word() {
        assert_eq!(quote("/home/my user/bin"), "'/home/my user/bin'");
        assert_eq!(shell_path("~/.local/bin/divixi-server"), "\"$HOME\"/'.local/bin/divixi-server'");
        assert_eq!(shell_path("/opt/divixi server/bin"), "'/opt/divixi server/bin'");
        assert_eq!(shell_path("~"), "\"$HOME\"");
    }

    #[test]
    fn nothing_in_a_path_can_become_shell() {
        // The single quote is the only character that ends a quoted word, so
        // it is the only one that has to be spliced.
        assert_eq!(quote("it's"), r"'it'\''s'");
        for evil in ["x; rm -rf ~", "$(reboot)", "`reboot`", "a\nb", "x && y", "~/$HOME"] {
            let word = quote(evil);
            assert!(word.starts_with('\'') && word.ends_with('\''), "{word}");
            // Whatever is inside is one word: no unescaped quote can close it.
            assert!(!word[1..word.len() - 1].contains('\''), "{word}");
        }
    }

    #[test]
    fn every_script_quotes_the_home_folder_and_the_release() {
        let scripts = [
            probe_script("~/.local/bin/divixi-server"),
            fetch_script("v2026-09-30", &asset().url, Fetch::Curl),
            push_script("v2026-09-30"),
            digest_script("v2026-09-30", true, Hasher::Sha256sum),
            install_script("v2026-09-30", &Launch::Setsid, ""),
            start_script(&Launch::Setsid, "", &shell_path("~/.local/bin/divixi-server")),
            cleanup_script("v2026-09-30"),
        ];
        for s in scripts {
            // `$HOME` is never left bare in a word that a space could split.
            for line in s.lines() {
                assert!(!line.contains("$HOME/") || line.contains("\"$HOME\"/"), "unquoted $HOME: {line}");
            }
        }
    }

    // ----- uname -----

    #[test]
    fn uname_says_which_machines_can_be_installed_to() {
        let at = |os: &str, arch: &str| Probe { os: os.into(), arch: arch.into(), ..Default::default() }.supported();
        assert!(at("Linux", "x86_64"));
        assert!(at("Linux", "amd64"));
        assert!(!at("Linux", "aarch64"), "no arm64 binary is published");
        assert!(!at("Linux", "armv7l"));
        assert!(!at("Darwin", "x86_64"), "no macOS binary is published");
        assert!(!at("Darwin", "arm64"));
        assert!(!at("FreeBSD", "x86_64"));
        // Nothing came back at all: not a platform to install to.
        assert!(!at("", ""));
    }

    #[test]
    fn the_probe_reads_what_the_remote_printed() {
        let p = parse_probe(
            "home=/home/jm\nos=Linux\narch=x86_64\nbin=yes\ninstalled=v2026-09-28\nrunning=yes\nfetch=curl\nhasher=sha256sum\nfree=8000000\n",
        );
        assert_eq!(p.home, "/home/jm");
        assert!(p.supported());
        assert!(p.bin && p.running);
        assert_eq!(p.installed.as_deref(), Some("v2026-09-28"));
        assert_eq!(p.fetch, Some(Fetch::Curl));
        assert_eq!(p.hasher, Some(Hasher::Sha256sum));
        assert_eq!(p.free, Some(8_000_000 * 1024));
        assert_eq!(p.launch(), Launch::Setsid);
    }

    #[test]
    fn a_home_folder_with_a_space_comes_back_whole() {
        let p = parse_probe("home=/home/my user\nos=Linux\narch=x86_64\n");
        assert_eq!(p.home, "/home/my user");
    }

    #[test]
    fn a_missing_fact_is_absent_rather_than_wrong() {
        let p = parse_probe("os=Linux\narch=x86_64\n");
        assert!(!p.bin, "no bin= line means nothing runnable there");
        assert_eq!(p.installed, None);
        assert_eq!(p.fetch, None, "no fetcher there: the app has to push the bytes");
        assert_eq!(p.hasher, None);
        assert_eq!(p.free, None, "df said nothing, which is not the same as no room");
    }

    #[test]
    fn a_systemd_unit_is_read_down_to_the_binary_it_names() {
        let p = parse_probe("os=Linux\narch=x86_64\nsystemd={ path=/home/jm/.local/bin/divixi-server ; argv[]=/home/jm/.local/bin/divixi-server serve ; ignore_errors=no }\n");
        assert_eq!(p.launch(), Launch::Systemd { exec_start: "/home/jm/.local/bin/divixi-server".into() });
        // A bare path (an older systemd, or none of the record) still reads.
        assert_eq!(exec_start_path("/home/jm/.divixi/server/current/divixi-server"), "/home/jm/.divixi/server/current/divixi-server");
    }

    // ----- the verdict -----

    fn linux() -> Probe {
        Probe { os: "Linux".into(), arch: "x86_64".into(), home: "/home/jm".into(), fetch: Some(Fetch::Curl), hasher: Some(Hasher::Sha256sum), ..Default::default() }
    }

    fn found() -> Result<(Asset, Kind), String> {
        Ok((asset(), Kind::App))
    }

    #[test]
    fn nothing_there_is_told_apart_from_something_old() {
        // (a) nothing at all.
        assert_eq!(
            check(&linux(), None, &found(), "abc"),
            Check::Missing { target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
        // (b) an older release than the app's.
        let old = Probe { bin: true, installed: Some("v2026-09-28".into()), ..linux() };
        assert_eq!(
            check(&old, None, &found(), "abc"),
            Check::Outdated { why: Why::OlderRelease, installed: Some("v2026-09-28".into()), target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
        // (c) the release the app would install.
        let now = Probe { bin: true, installed: Some("v2026-09-30".into()), ..linux() };
        assert_eq!(check(&now, None, &found(), "abc"), Check::Current { installed: Some("v2026-09-30".into()), running: false });
    }

    /// The bug this had: a server from before `/api/health` carried a build
    /// answers with an empty one, and comparing strings called that "built
    /// from other code than this app". It is not a different build -- it is
    /// one old enough not to have the field, which is a different sentence
    /// and a different thing for the user to do.
    #[test]
    fn a_server_with_no_build_is_too_old_not_a_different_build() {
        let there = Probe { bin: true, ..linux() };
        let none = Seen { build: String::new(), release: String::new() };
        assert_eq!(
            check(&there, Some(&none), &found(), "abc"),
            Check::Outdated { why: Why::TooOld, installed: None, target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
        // And a build that is there and differs is the other sentence.
        let other = Seen { build: "999".into(), release: String::new() };
        assert_eq!(
            check(&there, Some(&other), &found(), "abc"),
            Check::Outdated { why: Why::OtherBuild, installed: None, target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
        // The same build is the same code: nothing to do.
        let same = Seen { build: "abc".into(), release: String::new() };
        assert_eq!(check(&there, Some(&same), &found(), "abc"), Check::Current { installed: None, running: false });
    }

    #[test]
    fn releases_are_compared_as_dates_not_as_text() {
        let at = |installed: &str| {
            let p = Probe { bin: true, installed: Some(installed.into()), ..linux() };
            check(&p, None, &Ok((Asset { release: "v2026-09-10".into(), ..asset() }, Kind::App)), "abc")
        };
        // Text would put 9 above 10 and call the older one current.
        assert!(matches!(at("v2026-09-09"), Check::Outdated { why: Why::OlderRelease, .. }));
        assert_eq!(at("v2026-09-10"), Check::Current { installed: Some("v2026-09-10".into()), running: false });
        // Newer than the app's own release: not something to replace.
        assert_eq!(at("v2026-09-30"), Check::Current { installed: Some("v2026-09-30".into()), running: false });
    }

    /// A divixi-server somebody installed by hand from a release still says
    /// which release it is (`/api/health`), and that is a date to compare
    /// even though the app's pointer knows nothing about it.
    #[test]
    fn a_release_server_the_app_did_not_install_still_names_itself() {
        let there = Probe { bin: true, ..linux() };
        let old = Seen { build: "999".into(), release: "v2026-09-28".into() };
        assert_eq!(
            check(&there, Some(&old), &found(), "abc"),
            Check::Outdated { why: Why::OlderRelease, installed: Some("v2026-09-28".into()), target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
        // The same release: the build comparison is what is left, and it
        // agrees, so there is nothing to do.
        let now = Seen { build: "abc".into(), release: "v2026-09-30".into() };
        assert_eq!(check(&there, Some(&now), &found(), "abc"), Check::Current { installed: Some("v2026-09-30".into()), running: false });
        // A server answering from a path the instance is not configured with
        // is not "missing", whatever the configured path holds.
        let elsewhere = Probe { bin: false, ..linux() };
        let dev = Seen { build: "999".into(), release: String::new() };
        assert_eq!(
            check(&elsewhere, Some(&dev), &found(), "abc"),
            Check::Outdated { why: Why::OtherBuild, installed: None, target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624 }
        );
    }

    #[test]
    fn a_platform_with_no_binary_is_refused_before_anything_else() {
        let arm = Probe { arch: "aarch64".into(), bin: true, ..linux() };
        assert_eq!(check(&arm, None, &found(), "abc"), Check::Unsupported { os: "Linux".into(), arch: "aarch64".into() });
        // Even when GitHub could not be asked: the platform is the reason.
        assert_eq!(check(&arm, None, &Err("offline".into()), "abc"), Check::Unsupported { os: "Linux".into(), arch: "aarch64".into() });
    }

    #[test]
    fn a_systemd_unit_of_its_own_stops_the_install_rather_than_fight_it() {
        let p = Probe { bin: true, systemd: Some("/home/jm/.local/bin/divixi-server".into()), ..linux() };
        assert_eq!(
            check(&p, None, &found(), "abc"),
            Check::Managed { exec_start: "/home/jm/.local/bin/divixi-server".into(), want: MANAGED_BIN.into() }
        );
        // A unit that already names the pointer is the app's to restart, so
        // the judgement carries on to what is actually installed.
        let ours = Probe { systemd: Some("/home/jm/.divixi/server/current/divixi-server".into()), ..p };
        assert!(!matches!(check(&ours, None, &found(), "abc"), Check::Managed { .. }));
    }

    #[test]
    fn a_server_nobody_can_place_is_left_alone() {
        // A binary at the configured path, not running, no pointer: built
        // from source by whoever runs that machine.
        let p = Probe { bin: true, ..linux() };
        assert_eq!(check(&p, None, &found(), "abc"), Check::Unmanaged { target: "v2026-09-30".into(), source: Kind::App, size: 207_108_624, running: false });
    }

    #[test]
    fn no_release_to_install_from_says_so() {
        let p = Probe { bin: false, ..linux() };
        assert_eq!(check(&p, None, &Err("GitHub was not reached".into()), "abc"), Check::NoTarget { detail: "GitHub was not reached".into() });
    }

    // ----- the asset and its digest -----

    #[test]
    fn the_release_binary_is_found_with_the_hash_github_keeps() {
        let json = serde_json::json!({
            "tag_name": "v2026-09-30",
            "assets": [
                { "name": "Divixi_0.1.0_amd64.AppImage", "browser_download_url": "https://github.com/x/y", "size": 1, "digest": "sha256:aa" },
                { "name": "divixi-server-2026-09-30-x86_64-linux", "browser_download_url": "https://github.com/jaemin93/divixi/releases/download/v2026-09-30/divixi-server-2026-09-30-x86_64-linux", "size": 207_108_624,
                  "digest": "sha256:D856519C026482A0FCFCFB09EC8673C0692775192C0F61CF053FC46DA523C35F" }
            ]
        });
        let got = pick_asset(&serde_json::from_value(json).unwrap()).unwrap();
        assert_eq!(got, asset(), "the hash is kept lowercase, whichever case GitHub used");
    }

    #[test]
    fn a_release_with_no_digest_is_refused_rather_than_installed_unchecked() {
        let json = serde_json::json!({
            "tag_name": "v2026-09-30",
            "assets": [{ "name": "divixi-server-2026-09-30-x86_64-linux", "browser_download_url": "https://github.com/jaemin93/divixi/x", "size": 207_108_624 }]
        });
        let err = pick_asset(&serde_json::from_value(json).unwrap()).unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
        // And so is one whose digest is not a sha-256.
        let json = serde_json::json!({
            "tag_name": "v1",
            "assets": [{ "name": "divixi-server-1-x86_64-linux", "browser_download_url": "https://github.com/jaemin93/divixi/x", "size": 2, "digest": "md5:abc" }]
        });
        assert!(pick_asset(&serde_json::from_value(json).unwrap()).unwrap_err().contains("SHA-256"));
    }

    #[test]
    fn a_release_without_the_linux_binary_is_not_installed_from() {
        let json = serde_json::json!({ "tag_name": "v2026-09-30", "assets": [{ "name": "Divixi_0.1.0_amd64.deb", "size": 1 }] });
        let err = pick_asset(&serde_json::from_value(json).unwrap()).unwrap_err();
        assert!(err.contains("no divixi-server binary"), "{err}");
    }

    // ----- verification -----

    #[test]
    fn a_file_is_taken_only_when_its_hash_and_its_size_both_match() {
        let a = asset();
        assert!(matches(&a, a.size, &a.sha256).is_ok());
    }

    #[test]
    fn a_wrong_hash_is_refused() {
        let a = asset();
        let err = matches(&a, a.size, &"0".repeat(64)).unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
    }

    #[test]
    fn a_truncated_download_is_refused_and_said_to_be_short() {
        let a = asset();
        // The whole point of the size check: this is what a dropped
        // connection leaves, and its hash is wrong too.
        let err = matches(&a, a.size - 4096, &a.sha256).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(matches(&a, 0, &a.sha256).unwrap_err().contains("not there"));
        // A remote that could not hash it is not a pass either.
        assert!(matches(&a, a.size, "").unwrap_err().contains("could not hash"));
    }

    #[test]
    fn the_hash_the_remote_printed_is_read_from_its_line() {
        let (size, sha) = read_digest("#size 207108624\n#sha D856519C026482A0FCFCFB09EC8673C0692775192C0F61CF053FC46DA523C35F\n");
        assert_eq!(size, 207_108_624);
        assert_eq!(sha, asset().sha256);
        // Nothing there at all.
        assert_eq!(read_digest("#size 0\n"), (0, String::new()));
    }

    // ----- the shell lines -----

    #[test]
    fn the_install_stops_the_old_server_before_it_moves_anything() {
        let s = install_script("v2026-09-30", &Launch::Setsid, "");
        let stop = s.find("pkill").expect("the old server is stopped");
        let mv = s.find("mv -f").expect("the new binary is renamed into place");
        let swap = s.find("ln -sfn").expect("the pointer moves");
        let start = s.find("setsid").expect("the server comes back");
        assert!(stop < mv && mv < swap && swap < start, "stop, rename, swap, start — in that order:\n{s}");
        // Never written over: ETXTBSY is what writing to a running binary
        // gives, and a rename does not.
        assert!(!s.contains("> \"$HOME\""), "nothing is written over:\n{s}");
    }

    #[test]
    fn the_remote_path_setting_reaches_the_started_server() {
        let s = install_script("v2026-09-30", &Launch::Setsid, "~/.local/bin:/usr/bin");
        assert!(s.contains("PATH=~/.local/bin:/usr/bin:\"$PATH\"; export PATH; setsid"), "{s}");
        // And what it starts is the pointer, not the release path: the next
        // install moves the pointer and the same line still works.
        assert!(s.contains("setsid -f \"$HOME\"/.divixi/server/'current'/'divixi-server' serve"), "{s}");
    }

    #[test]
    fn systemd_starts_and_stops_through_systemd() {
        let s = install_script("v2026-09-30", &Launch::Systemd { exec_start: "x".into() }, "");
        assert!(s.contains("systemctl --user stop divixi-server"));
        assert!(s.contains("systemctl --user start divixi-server"));
        assert!(!s.contains("setsid"), "the unit is what starts it:\n{s}");
    }

    #[test]
    fn the_download_lands_on_a_name_that_is_not_the_binary() {
        for s in [fetch_script("v1", "https://github.com/x", Fetch::Curl), push_script("v1")] {
            assert!(s.contains("divixi-server.part"), "{s}");
            // And it is taken away when the fetch fails.
            assert!(s.contains("rm -f"), "{s}");
        }
    }

    /// Every script the remote is asked to run is shell it can parse.
    ///
    /// `sh -n` reads a script without running any of it, which is what
    /// catches the mistakes that matter here: a quote left open by a path,
    /// an `fi` that does not close anything. Without this they would turn up
    /// as a remote command that quietly did nothing.
    ///
    /// Skipped where there is no POSIX shell to ask (a plain Windows box);
    /// CI runs it on Linux and macOS, which is where the scripts run.
    #[test]
    fn every_script_parses_as_posix_shell() {
        use std::process::Command;
        if !Command::new("sh").arg("-c").arg("exit 0").output().is_ok_and(|o| o.status.success()) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("divixi-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp folder");
        let scripts = [
            ("probe", probe_script("~/.local/bin/divixi-server")),
            ("probe-spaces", probe_script("/opt/my divixi/divixi-server")),
            ("fetch-curl", fetch_script("v2026-09-30", &asset().url, Fetch::Curl)),
            ("fetch-wget", fetch_script("v2026-09-30", &asset().url, Fetch::Wget)),
            ("push", push_script("v2026-09-30")),
            ("digest", digest_script("v2026-09-30", true, Hasher::Sha256sum)),
            ("digest-openssl", digest_script("v2026-09-30", false, Hasher::Openssl)),
            ("install-setsid", install_script("v2026-09-30", &Launch::Setsid, "~/.local/bin:/usr/bin")),
            ("install-systemd", install_script("v2026-09-30", &Launch::Systemd { exec_start: "x".into() }, "")),
            ("start-setsid", start_script(&Launch::Setsid, "~/.local/bin", &shell_path("~/.local/bin/divixi-server"))),
            ("start-systemd", start_script(&Launch::Systemd { exec_start: "x".into() }, "", "x")),
            ("cleanup", cleanup_script("v2026-09-30")),
        ];
        for (name, script) in scripts {
            let file = dir.join(format!("{name}.sh"));
            std::fs::write(&file, &script).expect("write the script");
            let out = Command::new("sh").arg("-n").arg(&file).output().expect("run sh -n");
            assert!(out.status.success(), "{name} is not shell sh can read: {}\n{script}", String::from_utf8_lossy(&out.stderr));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The install script, run on a real shell, in a home folder with a
    /// space in its name.
    ///
    /// Not a simulation of the shell: `sh` runs the very string the app
    /// sends. `pgrep`, `pkill` and `setsid` are stubs on `PATH` -- the stub
    /// `setsid` leaves a marker that the stub `pgrep` then finds, which is
    /// how "the server came back up" is arranged without starting one. What
    /// is under test is the part that has to be right: the part file becomes
    /// the binary by a rename, the pointer ends up on the release, and
    /// nothing is left behind.
    ///
    /// Unix only. Git Bash on Windows copies rather than links for `ln -s`,
    /// so `readlink` there would say nothing about the pointer; CI runs this
    /// on Linux and macOS, which is where the script runs for real.
    #[cfg(unix)]
    #[test]
    fn the_install_runs_on_a_real_shell_in_a_home_folder_with_a_space() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("divixi-install-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("my home");
        let stubs = root.join("stub bin");
        std::fs::create_dir_all(&stubs).expect("a stub folder");
        let write = |name: &str, body: &str| {
            let at = stubs.join(name);
            std::fs::write(&at, body).expect("write a stub");
            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
        };
        write("pgrep", "#!/bin/sh\n[ -f \"$HOME/up\" ] && exit 0\nexit 1\n");
        write("pkill", "#!/bin/sh\nrm -f \"$HOME/up\"\nexit 0\n");
        write("setsid", "#!/bin/sh\n: > \"$HOME/up\"\nshift\necho \"$1\" > \"$HOME/started\"\nexit 0\n");

        let version = home.join(".divixi/server/v2026-09-30");
        std::fs::create_dir_all(&version).expect("the version folder");
        std::fs::write(version.join("divixi-server.part"), b"a binary, near enough").expect("the download");
        // Something is running: it has to be stopped before anything moves.
        std::fs::write(home.join("up"), b"").expect("the marker");

        let path = format!("{}:{}", stubs.display(), std::env::var("PATH").unwrap_or_default());
        let out = Command::new("sh")
            .arg("-c")
            .arg(install_script("v2026-09-30", &Launch::Setsid, ""))
            .env("HOME", &home)
            .env("PATH", path)
            .output()
            .expect("run the install script");
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{said}{}", String::from_utf8_lossy(&out.stderr));
        assert!(said.contains("#swapped") && said.contains("#up"), "{said}");

        // The download became the binary, by a rename: nothing was written
        // over, and the part file is gone.
        assert_eq!(std::fs::read(version.join("divixi-server")).expect("the binary"), b"a binary, near enough");
        assert!(!version.join("divixi-server.part").exists(), "the part file is gone");
        assert_eq!(std::fs::metadata(version.join("divixi-server")).unwrap().permissions().mode() & 0o777, 0o755);
        // And the pointer names the release, through a folder with a space.
        let current = home.join(".divixi/server/current");
        assert_eq!(std::fs::read_link(&current).expect("the pointer"), std::path::PathBuf::from("v2026-09-30"));
        assert_eq!(std::fs::read(current.join("divixi-server")).expect("through the pointer"), b"a binary, near enough");
        // The server was started from the pointer, not from the release path.
        let started = std::fs::read_to_string(home.join("started")).expect("setsid ran");
        assert_eq!(started.trim(), current.join("divixi-server").display().to_string());
        assert!(!home.join(".divixi/server/.current.new").exists(), "the spare name is not left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Running it twice changes nothing and breaks nothing: the pointer is
    /// already there, and moving it onto itself is the case `ln -sfn` alone
    /// would have got wrong.
    #[cfg(unix)]
    #[test]
    fn installing_the_same_release_again_is_harmless() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("divixi-install-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let stubs = root.join("bin");
        std::fs::create_dir_all(&stubs).expect("a stub folder");
        for (name, body) in [("pgrep", "#!/bin/sh\nexit 0\n"), ("pkill", "#!/bin/sh\nexit 0\n"), ("setsid", "#!/bin/sh\nexit 0\n")] {
            let at = stubs.join(name);
            std::fs::write(&at, body).expect("write a stub");
            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
        }
        let version = home.join(".divixi/server/v2026-09-30");
        std::fs::create_dir_all(&version).expect("the version folder");
        std::fs::write(version.join("divixi-server"), b"already here").expect("the binary");
        std::fs::set_permissions(version.join("divixi-server"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", stubs.display(), std::env::var("PATH").unwrap_or_default());
        for _ in 0..2 {
            let out = Command::new("sh")
                .arg("-c")
                .arg(install_script("v2026-09-30", &Launch::Setsid, ""))
                .env("HOME", &home)
                .env("PATH", &path)
                .output()
                .expect("run the install script");
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        let current = home.join(".divixi/server/current");
        assert_eq!(std::fs::read_link(&current).expect("the pointer"), std::path::PathBuf::from("v2026-09-30"));
        assert_eq!(std::fs::read(current.join("divixi-server")).unwrap(), b"already here");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn starting_a_stopped_server_touches_nothing_else() {
        let s = start_script(&Launch::Setsid, "", "\"$HOME\"/.divixi/server/'current'/'divixi-server'");
        assert!(s.contains("setsid -f \"$HOME\"/.divixi/server/'current'/'divixi-server' serve"), "{s}");
        assert!(s.contains("#up"), "it says whether the server came up:
{s}");
        // Nothing is stopped, moved or removed: this is not an install.
        for verb in ["pkill", "mv ", "rm ", "ln ", "chmod"] {
            assert!(!s.contains(verb), "{verb} has no business here:
{s}");
        }
        // A server built from source there is started from its own path.
        let own = start_script(&Launch::Setsid, "", &shell_path("~/.local/bin/divixi-server"));
        assert!(own.contains("setsid -f \"$HOME\"/'.local/bin/divixi-server' serve"), "{own}");
        // And a systemd unit is what starts it where there is one.
        let unit = start_script(&Launch::Systemd { exec_start: "x".into() }, "", "ignored");
        assert!(unit.contains("systemctl --user start divixi-server") && !unit.contains("setsid"), "{unit}");
    }

    #[test]
    fn a_server_that_is_installed_and_down_says_so_rather_than_offering_207_mb() {
        let down = Probe { bin: true, installed: Some("v2026-09-30".into()), running: false, ..linux() };
        assert_eq!(check(&down, None, &found(), "abc"), Check::Current { installed: Some("v2026-09-30".into()), running: false });
        let up = Probe { running: true, ..down };
        assert_eq!(check(&up, None, &found(), "abc"), Check::Current { installed: Some("v2026-09-30".into()), running: true });
    }

    #[test]
    fn a_cancelled_install_leaves_nothing_behind() {
        let s = cleanup_script("v2026-09-30");
        assert_eq!(s.trim(), "rm -f \"$HOME\"/.divixi/server/'v2026-09-30'/'divixi-server.part'");
    }
}
