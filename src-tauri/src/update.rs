//! Whether a newer release of divixi has been published.
//!
//! One command. It asks GitHub for the newest published release of
//! `jaemin93/divixi`, compares that release's tag against the tag this binary
//! was built from, and says which of the two is newer. That is all it does.
//!
//! Two things call it: the button on the settings page, and -- once a launch,
//! at most once a day, while the switch beside that button is on -- the app
//! itself. The schedule is not here: it is ui/src/lib/updateSchedule.ts, a
//! pure function with the tests for it beside it, because the switch and the
//! stamp it reads live in the settings the interface already owns. What is
//! here has no memory of when it last ran and no timer of its own.
//!
//! ## Where it stops, and why it stops there
//!
//! divixi has no code-signing certificate, and buying one is decided against
//! for now. That decision draws the line this module will not cross: the app
//! **never installs anything by itself.** No silent install, no `/S`, no
//! flags handed to a setup program, nothing at all that is not a press. The
//! automatic check is a *question*, and it is the only part of this that runs
//! unasked: every byte fetched and every file opened is still a press of its
//! own. An update channel that runs unsigned code it fetched is a
//! way in for whoever can answer for the server, and the signature is the
//! only thing that would make it not one. So Tauri's updater plugin is not
//! used and no `latest.json` or `.sig` is produced.
//!
//! What the app does do -- step 1 on a press or once a day, the rest only ever
//! on a press:
//!
//!   1. asks GitHub for the newest published release ([`update_check`]);
//!   2. downloads the installer for this platform into the app data folder
//!      ([`update_download`]), writing to a `.part` file;
//!   3. checks the bytes against the SHA-256 GitHub published for that asset,
//!      and deletes them if they are not it;
//!   4. on a second press, hands the verified file to the system shell
//!      ([`update_open`]) exactly as double-clicking it in the file manager
//!      would -- the system decides what happens next, and the person clicks
//!      through the installer's own windows.
//!
//! Step 4 is an *open*, not a *run*: the file goes to the shell with no
//! arguments, so whatever Windows does about an unsigned executable it does,
//! rather than being stepped around. The download even marks the file as
//! having come from the internet (`Zone.Identifier`), because it did.
//!
//! If a certificate is ever bought, the decision to revisit is
//! `tauri-plugin-updater` plus `plugins.updater.pubkey` in tauri.conf.json,
//! and the release workflow has to start producing `latest.json` and `.sig`
//! artifacts. Until then, none of that exists.
//!
//! Windows SmartScreen and macOS Gatekeeper both warn about the unsigned
//! bundles, which is why the buttons in front of all of this say so.
//!
//! ## Which platforms can be fetched
//!
//! Windows only, for now. [`installer`] is the one place that knows what this
//! platform's bundle is called; everything else is written against what it
//! returns, so adding macOS or Linux is editing that function. Where it
//! answers `None` the interface keeps the manual path it always had: the
//! release page, opened in the browser.
//!
//! ## Where the expected hash comes from
//!
//! GitHub computes a SHA-256 for every release asset when it is uploaded and
//! returns it as `assets[].digest` (`sha256:<hex>`) from the same request that
//! gives the download address. Checked against
//! `api.github.com/repos/jaemin93/divixi/releases/latest` on 2026-09-30: all
//! eight assets of `v2026-09-30` carry one. So no `SHA256SUMS` asset is added
//! to the release workflow -- the workflow is not touched at all. Reading the
//! digest out of the *same answer* as the URL is also what makes it safe to
//! use: one release, one pair, with no chance of checking one release's hash
//! against another's bytes. An asset with no digest is refused rather than
//! installed unverified.
//!
//! ## Privacy
//!
//! The README promises that divixi sends nothing *about you* anywhere. This is
//! the one request the app ever makes on its own behalf, so it is kept to the
//! smallest shape that can work:
//!
//!   * it happens on a press, or once a launch and at most once a day while
//!     the switch in Settings -> About is on. Never more often than that, and
//!     never at all with the switch off. There is no timer and nothing in the
//!     background: the automatic one is a single call as the window comes up,
//!     and it is capped by a stamp in the settings, not by a clock running
//!     inside the app;
//!   * it carries no authentication. The endpoint is public, so the user's
//!     GitHub token (which exists for remote instances, `remote::github`) is
//!     deliberately NOT sent: it would name the user to GitHub for no gain;
//!   * it carries no version, no platform, no identifier of any kind. The
//!     `User-Agent` is the bare word `divixi`, which the GitHub API requires
//!     of every caller. What GitHub learns from it is an address asking a
//!     public question once a day, which is what any reader of the releases
//!     page is;
//!   * unauthenticated, GitHub allows 60 requests an hour per address, which
//!     one a day plus a button nobody presses sixty times an hour will not
//!     reach. Being rate-limited is reported rather than retried.
//!
//! ## Drafts
//!
//! `releases/latest` returns the newest release that is neither a draft nor a
//! prerelease. divixi's nightly release is created as a *draft* and stays one
//! until a person edits the notes and publishes it (see the header of
//! .github/workflows/release.yml). So an unpublished nightly is invisible
//! here, and that is the intended behaviour: publishing is the act that means
//! "this is for people to install". A build made from a tag whose release is
//! still a draft therefore reads as [`Check::Ahead`] -- newer than anything
//! published -- rather than as an error.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use tauri::{Emitter, State};

// The app as every part of it holds it: Wry on a desktop, Tauri's mock runtime
// in divixi-server. A command that named `tauri::AppHandle` would compile only
// for the first of the two.
use crate::{AppHandle, AppState};

/// The repository the releases come from. Public: no token is used to read it.
pub(crate) const REPO: &str = "jaemin93/divixi";

/// The release this binary was built from, or `None` for a build that is not
/// a release.
///
/// `DIVIXI_RELEASE` is set by build.rs from the `DIVIXI_RELEASE_TAG`
/// environment variable, which only the release workflow sets. Empty means a
/// development build: `cargo build`, `npm run app`, a contributor's checkout.
pub fn release() -> Option<&'static str> {
    let tag = env!("DIVIXI_RELEASE");
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// A release tag: `vYYYY-MM-DD`, with `.N` for the Nth release of that day.
///
/// The fields are in the order they have to be compared in, so the derived
/// `Ord` is the right one. Numbers, not text: compared as strings, `v2026-9-9`
/// would sort above `v2026-9-10`, and `.10` above `.2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Release {
    year: u16,
    month: u8,
    day: u8,
    /// Where in the day's sequence this release sits. A tag with no `.N`
    /// suffix is the first of its day, so it is 1 -- which is what makes
    /// `v2026-09-28.2` come out above `v2026-09-28` rather than below it.
    seq: u32,
}

/// A run of ASCII digits as a number, or `None` for anything else.
///
/// `str::parse` on its own would take `+7`, `-7` and `  7`, none of which
/// belong in a tag.
fn digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A release tag, or `None` for anything that is not one.
///
/// Strict on purpose. A tag this does not recognise is either a mistake in the
/// workflow or a release named by hand, and comparing a date to something that
/// is not a date can only produce an answer nobody should act on.
pub(crate) fn parse(tag: &str) -> Option<Release> {
    let rest = tag.strip_prefix('v')?;
    // `2026-09-28` or `2026-09-28.2`: the date is fixed-width, the suffix is
    // whatever is left.
    let (date, seq) = match rest.split_once('.') {
        Some((date, seq)) => (date, Some(seq)),
        None => (rest, None),
    };
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: u16 = digits(&date[0..4])?;
    let month: u8 = digits(&date[5..7])?;
    let day: u8 = digits(&date[8..10])?;
    // The shape can be right and the date impossible (v2026-13-40). Day 31 of
    // a 30-day month is let through: this is a name to order, not a calendar.
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let seq: u32 = match seq {
        None => 1,
        // `.0` is not a release: the workflow refuses it, and the first
        // release of a day carries no suffix at all.
        Some(s) => digits(s).filter(|&n| n >= 1)?,
    };
    Some(Release { year, month, day, seq })
}

/// How the release this binary is stands against the newest published one.
///
/// Pure, so every case below is a unit test rather than a release that has to
/// be cut to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// No tag was compiled in: there is nothing to compare.
    DevBuild,
    /// A tag was compiled in and it is not a release tag. A build bug, not
    /// the user's problem -- but not something to hide either.
    BadBuild,
    /// GitHub's newest release is not named like a release tag.
    BadLatest,
    /// This build is the newest published release.
    UpToDate,
    /// This build is newer than the newest published release -- normal while
    /// its own release is still a draft.
    Ahead,
    /// There is a newer published release.
    Update,
}

/// `current` is the tag this binary was built from ([`release`]); `latest` is
/// the tag GitHub gave for the newest published release.
fn verdict(current: Option<&str>, latest: &str) -> Verdict {
    let Some(current) = current else { return Verdict::DevBuild };
    let Some(mine) = parse(current) else { return Verdict::BadBuild };
    let Some(theirs) = parse(latest) else { return Verdict::BadLatest };
    match theirs.cmp(&mine) {
        std::cmp::Ordering::Greater => Verdict::Update,
        std::cmp::Ordering::Equal => Verdict::UpToDate,
        std::cmp::Ordering::Less => Verdict::Ahead,
    }
}

/// What the check found. Every case is one the interface has a line for; the
/// failures are cases too, so nothing is swallowed.
///
/// `detail` carries the English specifics (a status, a transport error) for
/// the log and for a bug report. The sentence around it is translated in the
/// interface, as everywhere else in divixi.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// Built from working copy: update checking means nothing here.
    DevBuild,
    UpToDate { current: String },
    Ahead { current: String, latest: String },
    Update {
        current: String,
        latest: String,
        url: String,
        /// Whether this platform is one the app can fetch the installer for
        /// ([`installer`]). False on macOS and Linux, where the interface
        /// keeps to the release page. Answered here rather than guessed at
        /// from the user agent in the webview: the binary knows what it is.
        can_download: bool,
    },
    /// GitHub was not reached: no network, DNS, a proxy, a timeout.
    Offline { detail: String },
    /// 60 unauthenticated requests an hour per address, and they are spent.
    RateLimited { detail: String },
    /// The repository has no published release yet (404). Drafts do not count.
    NoRelease,
    /// Any other answer: a status that is not success, or a body that is not
    /// a release.
    Failed { detail: String },
    /// The tag compiled into this binary is not a release tag.
    BadBuild { current: String },
}

/// The one field of the answer that is needed, plus the two that say the
/// answer is not one to act on.
///
/// `releases/latest` never returns a draft or a prerelease, so both flags are
/// expected to be false. They are read anyway: if GitHub ever answered with
/// one, treating it as a release people should install would be wrong, and
/// finding that out from a bug report is worse than a line in the log.
#[derive(Deserialize)]
struct Latest {
    tag_name: String,
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// The files attached to the release. Empty for the check, which needs
    /// only the tag; [`download`] picks this platform's installer out of it.
    #[serde(default)]
    assets: Vec<Asset>,
}

/// One file attached to a release.
#[derive(Deserialize)]
struct Asset {
    /// As uploaded, e.g. `Divixi_0.1.0_x64-setup.exe`. Treated as untrusted
    /// text until [`safe_name`] has looked at it: it ends up as a file name.
    name: String,
    browser_download_url: String,
    /// What GitHub stored, for a progress bar before the first byte arrives.
    /// Zero when the field is missing, which reads as "unknown" downstream.
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>`, computed by GitHub when the asset was uploaded. An
    /// asset without one cannot be checked and so is not offered.
    #[serde(default)]
    digest: Option<String>,
}

/// A client with no identity in it and a deadline, so a hung connection ends
/// as an answer rather than a button that spins forever.
pub(crate) fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // Required by the GitHub API of every caller. The bare app name: no
        // version, no platform, nothing that distinguishes one install from
        // another.
        .user_agent("divixi")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// A client for the installer download: the same anonymity, a different
/// deadline.
///
/// `http()`'s twenty seconds is right for a JSON body and wrong for a hundred
/// megabytes, so the limit here is the wait between reads -- a stalled
/// connection ends as an answer, a slow line is left to finish.
fn bulk_http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("divixi")
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

/// Why a request to GitHub produced no release to talk about.
///
/// The check and the download both have to answer for the same four ways the
/// one request can fail, and having them share the cases is what keeps the
/// two from drifting into saying different things about the same failure.
enum Fault {
    /// Never became an answer: no route, DNS, TLS, a timeout.
    Offline { detail: String },
    /// 60 unauthenticated requests an hour per address, and they are spent.
    RateLimited { detail: String },
    /// 404: the repository has no published release. Drafts do not count.
    NoRelease,
    /// Any other answer: a status that is not success, or a body that is not
    /// a release.
    Failed { detail: String },
}

/// The newest published release of the repository, assets and all.
///
/// One request, one place. Carries no authentication and no identifier; see
/// the module header on why. Made only from a press -- both callers are
/// commands a button invokes.
async fn latest() -> Result<Latest, Fault> {
    let client = http().map_err(|e| Fault::Failed { detail: format!("could not build an HTTP client: {e}") })?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let res = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| Fault::Offline { detail: e.to_string() })?;

    let status = res.status();
    if !status.is_success() {
        // GitHub answers a spent allowance with 403 (or 429) and
        // `x-ratelimit-remaining: 0`. The status alone cannot tell that from
        // any other refusal, so the header decides.
        let spent = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).map(|v| v.trim() == "0").unwrap_or(false);
        if spent || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Fault::RateLimited { detail: format!("GitHub answered {status}; the unauthenticated allowance of 60 requests an hour is spent") });
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            // No published release. divixi's nightly releases are drafts
            // until a person publishes them, and a repository whose releases
            // are all still drafts answers 404 here.
            return Err(Fault::NoRelease);
        }
        return Err(Fault::Failed { detail: format!("GitHub answered {status}") });
    }

    let body = res.bytes().await.map_err(|e| Fault::Offline { detail: format!("the answer did not finish: {e}") })?;
    let latest: Latest = serde_json::from_slice(&body).map_err(|e| Fault::Failed { detail: format!("GitHub's answer was not a release: {e}") })?;
    if latest.draft || latest.prerelease {
        // Cannot happen through this endpoint; see [`Latest`].
        return Err(Fault::Failed { detail: format!("GitHub offered {} as the latest release, but it is a draft or a prerelease", latest.tag_name) });
    }
    Ok(latest)
}

/// Where to send a browser for a release: its own page when GitHub named it,
/// which it always does; the tag's page otherwise. Either lands on the
/// downloads, which is the manual path this feature never takes away.
fn page(latest: &Latest) -> String {
    latest.html_url.clone().unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{}", latest.tag_name))
}

/// Which release this build is, for the settings page to show beside the
/// version. `None` for a development build.
///
/// Separate from the check so the page can say what this build is without
/// asking GitHub anything -- the request only ever happens on a press of the
/// button.
#[tauri::command]
pub fn update_release() -> Option<&'static str> {
    release()
}

/// Ask GitHub for the newest published release and compare it with this build.
///
/// Runs on a press of "check for updates", and once a launch when the
/// interface decides one is due (ui/src/lib/updateSchedule.ts). This command
/// keeps no schedule of its own: it asks whenever it is called. See the module
/// header for why nothing is downloaded or installed by either caller.
#[tauri::command]
pub async fn update_check() -> Check {
    let found = check(release()).await;
    match &found {
        Check::Update { current, latest, .. } => tracing::info!("[update] {current} -> {latest} is published"),
        Check::UpToDate { current } => tracing::info!("[update] {current} is the newest published release"),
        Check::Ahead { current, latest } => tracing::info!("[update] this build ({current}) is newer than the newest published release ({latest})"),
        Check::DevBuild => tracing::info!("[update] a development build has no release to compare"),
        Check::NoRelease => tracing::warn!("[update] the repository has no published release"),
        Check::Offline { detail } => tracing::warn!("[update] GitHub was not reached: {detail}"),
        Check::RateLimited { detail } => tracing::warn!("[update] rate limited by GitHub: {detail}"),
        Check::Failed { detail } => tracing::warn!("[update] the check failed: {detail}"),
        Check::BadBuild { current } => tracing::error!("[update] this binary was built with DIVIXI_RELEASE={current:?}, which is not a release tag"),
    }
    found
}

/// The command's body with the build's tag passed in, so the request handling
/// is one function and the tag is not read from the environment twice.
async fn check(current: Option<&str>) -> Check {
    // A development build asks GitHub nothing at all: there is no answer the
    // request could give it, and not sending it is the better default for a
    // contributor running from a checkout.
    if current.is_none() {
        return Check::DevBuild;
    }
    if let Some(tag) = current {
        if parse(tag).is_none() {
            return Check::BadBuild { current: tag.to_string() };
        }
    }

    let latest = match latest().await {
        Ok(l) => l,
        Err(Fault::Offline { detail }) => return Check::Offline { detail },
        Err(Fault::RateLimited { detail }) => return Check::RateLimited { detail },
        Err(Fault::NoRelease) => return Check::NoRelease,
        Err(Fault::Failed { detail }) => return Check::Failed { detail },
    };

    let current = current.unwrap_or_default();
    match verdict(Some(current), &latest.tag_name) {
        Verdict::Update => Check::Update {
            current: current.to_string(),
            latest: latest.tag_name.clone(),
            can_download: installer().is_some(),
            url: page(&latest),
        },
        Verdict::UpToDate => Check::UpToDate { current: current.to_string() },
        Verdict::Ahead => Check::Ahead { current: current.to_string(), latest: latest.tag_name },
        Verdict::BadLatest => Check::Failed { detail: format!("GitHub's newest release is tagged {:?}, which is not a release tag", latest.tag_name) },
        // Both ruled out above, before the request was made.
        Verdict::DevBuild | Verdict::BadBuild => Check::BadBuild { current: current.to_string() },
    }
}

// ----------------------------------------------------------------------------
// Fetching the installer
//
// Everything below runs on a press of a button and nothing else. It ends with
// a verified file on disk and a second button; see the module header on why it
// ends there and not one step further.
// ----------------------------------------------------------------------------

/// Which bundle of a release this platform installs from.
///
/// **The one place platform knowledge lives.** The download, the verification
/// and the open are all written against what this returns, so a second
/// platform is this function and the strings in the interface -- nothing else.
struct Installer {
    /// The end of the asset's name. A release carries every platform's bundle
    /// at once (`Divixi_0.1.0_x64-setup.exe`, `..._x64_en-US.msi`,
    /// `..._universal.dmg`, `..._amd64.deb`, `..._amd64.AppImage`, an .rpm and
    /// the server binary), so the tail is what tells them apart. The version
    /// in the middle moves on its own schedule and is deliberately not matched
    /// on: the release is named by date and the manifest by semver, and tying
    /// the two together here would break on the next version bump.
    suffix: &'static str,
    /// What the file is, for the log and for a bug report.
    what: &'static str,
}

/// This platform's installer, or `None` where the app does not fetch one.
///
/// `cfg!` rather than `#[cfg]`: every branch is compiled and type-checked on
/// every platform, so a platform added here cannot break a build nobody ran.
fn installer() -> Option<Installer> {
    // NSIS, and per-user by default in Tauri v2 -- no elevation, so the
    // installer's own windows are all the person has to get through.
    // tauri.conf.json sets no `bundle.windows.nsis.installMode`; if it ever
    // sets `perMachine`, the note beside the button has to start saying that
    // Windows will ask for an administrator.
    if cfg!(windows) && cfg!(target_arch = "x86_64") {
        return Some(Installer { suffix: "_x64-setup.exe", what: "NSIS installer (per-user)" });
    }
    if cfg!(windows) && cfg!(target_arch = "aarch64") {
        // The release workflow does not build this today, so the download
        // reports "no asset" rather than fetching an x64 installer that an
        // ARM machine would run under emulation.
        return Some(Installer { suffix: "_arm64-setup.exe", what: "NSIS installer (per-user)" });
    }
    // macOS and Linux: the .dmg needs dragging into /Applications and the
    // .deb/.rpm/.AppImage each need a different thing done to them, none of
    // which is "open this file". The release page stays the answer there.
    None
}

/// This platform's installer among a release's assets.
///
/// Pure, and takes the rule rather than reading it: every case below is a unit
/// test on every platform, including the Windows one on a Linux runner.
fn pick<'a>(assets: &'a [Asset], want: &Installer) -> Option<&'a Asset> {
    assets.iter().find(|a| a.name.ends_with(want.suffix) && safe_name(&a.name))
}

/// Whether an asset's name may be used as a file name.
///
/// GitHub's own uploads are tame, but the name arrives over the network and
/// ends up joined onto a path. A name that is anything other than one plain
/// file name is refused rather than sanitised: there is no reading of
/// `..\..\system32\x.exe` that this should try to make sense of.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || c == '"' || c == '*' || c == '?' || c == '<' || c == '>' || c == '|')
        && Path::new(name).file_name().map(|f| f == name).unwrap_or(false)
}

/// The hex of a GitHub asset digest (`sha256:<64 hex>`), lowercased.
///
/// `None` for anything else -- an algorithm this cannot compute, a truncated
/// field, a shape that is not recognised. Refusing is the only safe reading: a
/// digest that is not understood must never come out as "nothing to compare".
fn expected_hex(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

/// Why a file on disk is not the file that was published.
#[derive(Debug, PartialEq, Eq)]
enum Bad {
    /// It could not be read back at all.
    Unreadable(String),
    /// It was read, and it is not it.
    Mismatch { got: String },
}

/// The SHA-256 of a file, lowercase hex.
///
/// Read in chunks rather than loaded: an installer is tens of megabytes today
/// and the .dmg a future platform would want is more.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether the file on disk is the one `expected` (lowercase hex) names.
///
/// The file is read back rather than the download's own byte count trusted.
/// That is the whole point of checking: a connection that ended early, a disk
/// that filled up, a file something else had a go at -- every one of them
/// comes out here as a hash that is not the published one.
fn verify(path: &Path, expected: &str) -> Result<(), Bad> {
    let got = sha256_file(path).map_err(|e| Bad::Unreadable(format!("{}: {e}", path.display())))?;
    if got == expected {
        Ok(())
    } else {
        Err(Bad::Mismatch { got })
    }
}

/// Where a download is, for a bar.
///
/// The same shape as the agent download's `DownloadProgress`
/// (crates/agents/src/lib.rs): a phase, bytes received, and the total when the
/// server said. Kept the same on purpose -- the webview already draws one of
/// these, and a second progress shape would be a second thing to keep right.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Progress {
    pub phase: Phase,
    /// Bytes on disk so far.
    pub received: u64,
    /// Total bytes, when they are known. `None` means the bar shows an amount
    /// and no percentage -- which is the honest answer for a server that sent
    /// no length.
    pub total: Option<u64>,
}

/// The five states the interface distinguishes. Failure is not among them: a
/// failure is what [`update_download`] *returns*, named and with its reason,
/// so it cannot arrive as a phase nobody wrote a line for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Asking GitHub which release is newest and where its installer is.
    Checking,
    /// Bytes arriving into the `.part` file.
    Downloading,
    /// Reading the file back and hashing it.
    Verifying,
    /// Verified, renamed, and waiting to be opened.
    Ready,
}

/// How the download ended. Every case is one the interface has a line for,
/// the failures included, so nothing is swallowed and nothing is guessed at.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Download {
    /// Downloaded, hashed, and the hash is the published one. `path` is what
    /// [`update_open`] takes.
    Ready { tag: String, name: String, path: String, size: u64 },
    /// This platform is not one the app fetches an installer for; the release
    /// page is the way ([`installer`]).
    Unsupported { platform: String, url: String },
    /// The newest published release is not one to install over this build: a
    /// development build, a build tag that is not a release tag, or nothing
    /// newer. Which of the three is what the check said a moment ago.
    Nothing,
    /// The release has no asset ending in this platform's suffix. A release
    /// built without one platform's bundle, or an ARM Windows machine.
    NoAsset { tag: String, want: String, url: String },
    /// GitHub published no usable SHA-256 for the asset, so there is nothing
    /// to check the bytes against. Not downloaded: an installer nobody can
    /// verify is exactly the thing this module refuses to hand over.
    NoDigest { tag: String, name: String, url: String },
    /// The bytes that arrived are not the bytes GitHub published. They have
    /// been deleted -- a file that failed its hash is never left somewhere a
    /// button could open it.
    Corrupt { expected: String, got: String },
    /// Stopped by the person who started it. The `.part` file is gone.
    Cancelled,
    /// A download is already running. One at a time.
    Busy,
    Offline { detail: String },
    RateLimited { detail: String },
    NoRelease,
    Failed { detail: String },
}

/// Only one download at a time, and only one thing that can cancel it.
static BUSY: AtomicBool = AtomicBool::new(false);
/// Set by [`update_download_cancel`], read between chunks.
static CANCEL: AtomicBool = AtomicBool::new(false);


/// Fetch the newest release's installer for this platform, check it against
/// the SHA-256 GitHub published, and say where it is.
///
/// Runs on a press of "download the installer" and at no other time: no
/// startup, no timer, no retry of its own. The check may run unasked; this
/// never does. Nothing is executed here either -- see the module header.
///
/// Reaches the app data folder through the handle rather than taking
/// `State<'_, AppState>`: an async command that holds a reference has to
/// return a `Result`, and wrapping a type whose every case is already named
/// in an `Ok` would only add a second way to say the same things.
#[tauri::command]
pub async fn update_download(app: AppHandle) -> Download {
    // `swap` rather than load-then-store: two presses that land together must
    // not both get through.
    if BUSY.swap(true, Ordering::SeqCst) {
        return Download::Busy;
    }
    // A cancel that arrived after the last download had finished is not this
    // one's; start from nothing asked.
    CANCEL.store(false, Ordering::SeqCst);
    let dir = {
        use tauri::Manager;
        app.state::<AppState>().data_dir.join("updates")
    };
    // A callback and not the handle, as the agent download does
    // (`download_antigravity_with`, crates/agents/src/lib.rs): it keeps the
    // whole of the download testable without a window.
    let got = download(&dir, move |p| {
        let _ = app.emit("update_download", p);
    })
    .await;
    CANCEL.store(false, Ordering::SeqCst);
    BUSY.store(false, Ordering::SeqCst);
    match &got {
        Download::Ready { tag, path, size, .. } => tracing::info!("[update] {tag}: {size} bytes verified at {path}"),
        Download::Unsupported { platform, .. } => tracing::info!("[update] no installer is fetched for {platform}; the release page is the way"),
        Download::Nothing => tracing::info!("[update] there is nothing newer to download"),
        Download::NoAsset { tag, want, .. } => tracing::warn!("[update] {tag} has no asset ending in {want}"),
        Download::NoDigest { tag, name, .. } => tracing::warn!("[update] {tag}: GitHub published no sha256 for {name}, so it was not downloaded"),
        Download::Corrupt { expected, got } => tracing::error!("[update] the download hashed to {got}, not the published {expected}; it has been deleted"),
        Download::Cancelled => tracing::info!("[update] the download was cancelled"),
        Download::Busy => tracing::info!("[update] a download is already running"),
        Download::Offline { detail } => tracing::warn!("[update] GitHub was not reached: {detail}"),
        Download::RateLimited { detail } => tracing::warn!("[update] rate limited by GitHub: {detail}"),
        Download::NoRelease => tracing::warn!("[update] the repository has no published release"),
        Download::Failed { detail } => tracing::warn!("[update] the download failed: {detail}"),
    }
    got
}

/// Stop the download that is running, if one is.
///
/// Takes effect between chunks, which is at most a few kilobytes of waiting.
/// Harmless when nothing is running: the flag is cleared at the start of the
/// next download.
#[tauri::command]
pub fn update_download_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    tracing::info!("[update] a cancel was asked for");
}

/// The command's body, with the folder passed in and progress handed out
/// through a callback, so all of it can be run without an app around it.
///
/// `progress` is called at each phase change and, while bytes arrive, at most
/// a few times a second.
async fn download(dir: &Path, mut progress: impl FnMut(Progress)) -> Download {
    // One name for "say where we are", so the phase changes below read as the
    // sequence they are.
    macro_rules! report {
        ($phase:expr, $received:expr, $total:expr) => {
            progress(Progress { phase: $phase, received: $received, total: $total })
        };
    }

    let Some(want) = installer() else {
        return Download::Unsupported {
            platform: std::env::consts::OS.to_string(),
            url: format!("https://github.com/{REPO}/releases/latest"),
        };
    };

    report!(Phase::Checking, 0, None);
    // The release is asked for again rather than remembered from the check.
    // Two reasons, and the second is the one that matters: a release may have
    // been published in between, and the digest and the download address have
    // to come out of one answer or the app could end up checking one release's
    // hash against another release's bytes.
    let latest = match latest().await {
        Ok(l) => l,
        Err(Fault::Offline { detail }) => return Download::Offline { detail },
        Err(Fault::RateLimited { detail }) => return Download::RateLimited { detail },
        Err(Fault::NoRelease) => return Download::NoRelease,
        Err(Fault::Failed { detail }) => return Download::Failed { detail },
    };
    if verdict(release(), &latest.tag_name) != Verdict::Update {
        return Download::Nothing;
    }
    let url = page(&latest);
    let tag = latest.tag_name.clone();

    let Some(asset) = pick(&latest.assets, &want) else {
        return Download::NoAsset { tag, want: want.suffix.to_string(), url };
    };
    let Some(expected) = asset.digest.as_deref().and_then(expected_hex) else {
        return Download::NoDigest { tag, name: asset.name.clone(), url };
    };
    tracing::info!("[update] {tag}: {} ({}) sha256 {expected}", asset.name, want.what);

    if let Err(e) = std::fs::create_dir_all(dir) {
        return Download::Failed { detail: format!("could not make {}: {e}", dir.display()) };
    }
    let final_path = dir.join(&asset.name);

    // A file already here under the final name and hashing to the published
    // digest is the installer: a second press, or the same release checked
    // twice. Hashing it takes a moment and saves the download.
    if final_path.is_file() {
        report!(Phase::Verifying, 0, None);
        match verify(&final_path, &expected) {
            Ok(()) => {
                let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                report!(Phase::Ready, size, Some(size));
                return Download::Ready { tag, name: asset.name.clone(), path: final_path.display().to_string(), size };
            }
            // Not it, so it is not kept: the next press must not find it and
            // decide it is fine, and no button may open it in the meantime.
            Err(bad) => {
                tracing::warn!("[update] {} was already here and is not the published file ({bad:?}); removing it", final_path.display());
                let _ = std::fs::remove_file(&final_path);
            }
        }
    }

    // Everything else in the folder goes: a `.part` from a run that was killed
    // (the app closed, the machine went down) and any older release's
    // installer. This is what makes a half-downloaded file impossible to use
    // across restarts -- there is nothing to resume from, because there is no
    // resuming.
    //
    // No HTTP Range resume, deliberately. A `.part` says how many bytes it has
    // and nothing about which release they came from, so resuming could staple
    // the head of one release's installer to the tail of the next one's: the
    // hash would refuse it, and the person would have waited twice to be
    // refused. Starting over takes seconds and cannot be wrong.
    sweep(dir, &asset.name);

    let part = dir.join(format!("{}.part", asset.name));
    let total = match asset.size {
        0 => None,
        n => Some(n),
    };
    report!(Phase::Downloading, 0, total);

    let client = match bulk_http() {
        Ok(c) => c,
        Err(e) => return Download::Failed { detail: format!("could not build an HTTP client: {e}") },
    };
    let res = match client.get(&asset.browser_download_url).send().await {
        Ok(res) => res,
        Err(e) => return Download::Offline { detail: e.to_string() },
    };
    if !res.status().is_success() {
        return Download::Failed { detail: format!("GitHub answered {} for {}", res.status(), asset.name) };
    }
    // The server's own length when it sent one; the size GitHub stored
    // otherwise. Either can be missing, and then the bar shows an amount with
    // no percentage rather than a made-up one.
    let total = res.content_length().or(total);

    let file = match std::fs::File::create(&part) {
        Ok(f) => f,
        Err(e) => return Download::Failed { detail: format!("could not write {}: {e}", part.display()) },
    };
    // Buffered, so a 16 KiB chunk is not a write of its own: the writes are
    // small enough not to be worth moving off the runtime, and the buffer
    // makes them fewer.
    let mut out = std::io::BufWriter::with_capacity(256 * 1024, file);
    let mut received: u64 = 0;
    let mut stream = res.bytes_stream();
    let mut last = std::time::Instant::now();
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            // The half file goes with it. Nothing keeps a partial download
            // anywhere a later run could pick it up.
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Download::Cancelled;
        }
        let chunk = match stream.next().await {
            None => break,
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return Download::Offline { detail: format!("the download did not finish: {e}") };
            }
        };
        if let Err(e) = std::io::Write::write_all(&mut out, &chunk) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Download::Failed { detail: format!("could not write {}: {e}", part.display()) };
        }
        received += chunk.len() as u64;
        // Throttle: a bar does not need every chunk.
        if last.elapsed() >= Duration::from_millis(120) {
            report!(Phase::Downloading, received, total);
            last = std::time::Instant::now();
        }
    }
    if let Err(e) = std::io::Write::flush(&mut out) {
        drop(out);
        let _ = std::fs::remove_file(&part);
        return Download::Failed { detail: format!("could not finish writing {}: {e}", part.display()) };
    }
    drop(out);
    // A cancel that landed on the last chunk is still a cancel: the person
    // asked for this not to happen, so it does not, even though the bytes are
    // all here. Checked before the rename, so nothing is left to open.
    if CANCEL.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&part);
        return Download::Cancelled;
    }
    report!(Phase::Downloading, received, total);

    report!(Phase::Verifying, received, total);
    let checking = part.clone();
    let want_hex = expected.clone();
    // Hashing a hundred megabytes is not something to do on an async worker.
    let checked = tokio::task::spawn_blocking(move || verify(&checking, &want_hex)).await;
    match checked {
        Ok(Ok(())) => {}
        Ok(Err(Bad::Mismatch { got })) => {
            let _ = std::fs::remove_file(&part);
            return Download::Corrupt { expected, got };
        }
        Ok(Err(Bad::Unreadable(detail))) => {
            let _ = std::fs::remove_file(&part);
            return Download::Failed { detail: format!("the download could not be read back: {detail}") };
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Download::Failed { detail: format!("the check did not run: {e}") };
        }
    }

    // Verified, and only now does it get the name a button will open. Until
    // this line there is no file in the folder that could be taken for a
    // finished download.
    if let Err(e) = std::fs::rename(&part, &final_path) {
        let _ = std::fs::remove_file(&part);
        return Download::Failed { detail: format!("could not name {}: {e}", final_path.display()) };
    }
    mark_from_internet(&final_path, &asset.browser_download_url);

    let size = received;
    report!(Phase::Ready, size, Some(size));
    Download::Ready { tag, name: asset.name.clone(), path: final_path.display().to_string(), size }
}

/// Empty the updates folder of everything but the file about to be written.
///
/// One installer at a time is kept. An older release's setup file left lying
/// around is something a person can open by mistake, and a `.part` left by a
/// run that never finished is worse.
fn sweep(dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name() == std::ffi::OsStr::new(keep) {
            continue;
        }
        if entry.path().is_file() {
            if let Err(err) = std::fs::remove_file(entry.path()) {
                tracing::warn!(%err, "[update] could not remove {}", entry.path().display());
            }
        }
    }
}

/// Record that the file came off the internet, because it did.
///
/// Windows keeps this in an alternate data stream and the shell reads it:
/// SmartScreen and the "do you want to run this" dialog both come from it.
/// Writing it is the opposite of a convenience -- it *adds* a warning for the
/// person to click through. It is written anyway. The app fetched an unsigned
/// executable over the network, and handing it to the shell dressed as a local
/// file would be taking away a check that exists for exactly this case.
///
/// A failure is logged and no more: the file is verified either way, and a
/// filesystem with no streams (a network share, a FAT volume) is not a reason
/// to throw away a good download.
fn mark_from_internet(path: &Path, url: &str) {
    #[cfg(windows)]
    {
        // ZoneId=3 is "Internet". The stream is addressed as `<path>:<name>`.
        let stream = format!("{}:Zone.Identifier", path.display());
        let body = format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={url}\r\n");
        if let Err(err) = std::fs::write(&stream, body) {
            tracing::warn!(%err, "[update] could not mark {} as coming from the internet", path.display());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, url);
    }
}

/// The verified installer in the updates folder, by name.
///
/// Answers `Err` for anything that is not this platform's installer sitting in
/// the app's own updates folder. The path makes a round trip through the
/// webview, so it is checked here rather than trusted: [`update_open`] hands
/// what it gets to the system shell, and "open any file on this machine" is not
/// a command this app has.
fn verified(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let want = installer().ok_or_else(|| "no installer is fetched for this platform".to_string())?;
    let name = Path::new(path).file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{path} is not a file name"))?;
    if !safe_name(name) || !name.ends_with(want.suffix) {
        return Err(format!("{name} is not an installer this app downloaded"));
    }
    // Built from the folder and the name rather than taken as given: a `..` or
    // a second path glued on cannot survive that.
    let full = dir.join(name);
    if !full.is_file() {
        return Err(format!("{} is not there; download it again", full.display()));
    }
    Ok(full)
}

/// Hand a path in the updates folder to the system, and let the system decide.
///
/// Never through a shell, never with an argument: one path, spawned as one
/// process, which is what makes this the same act as double-clicking the file.
fn shell_open(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        // explorer.exe splits its argument at commas, so a path with one in it
        // would open something else or nothing at all. It cannot be escaped
        // away, and the folder is under a user name nobody here chose. So it
        // is refused, and the interface says to open the folder and
        // double-click -- the same act, done by hand.
        let plain = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        if plain.contains(',') {
            return Err(format!("{plain} has a comma in it, which the Windows file manager reads as the end of the path; open the folder and run the file from there"));
        }
        std::process::Command::new("explorer.exe").arg(&plain).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Open the verified installer, on a press, with the system's own default for
/// the file.
///
/// **This is where the app stops.** It does not install anything: the file goes
/// to the shell with no arguments, so Windows applies whatever it applies to an
/// unsigned executable that came off the internet (the download marked it as
/// one -- [`mark_from_internet`]), and the person clicks through the
/// installer's own windows. There is no silent mode here, no `/S`, no
/// `--quiet`, and no caller anywhere that reaches this without a press. That is
/// the certificate decision and not an oversight: an app that runs unsigned
/// code it downloaded, unattended, is an update channel that installs whatever
/// the server says. See the module header.
#[tauri::command]
pub fn update_open(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let dir = state.data_dir.join("updates");
    let full = verified(&dir, &path)?;
    tracing::info!("[update] handing {} to the system shell on a press", full.display());
    shell_open(&full)
}

/// Show the updates folder in the system file manager, so the file can be
/// found, moved, or double-clicked by hand.
///
/// The folder and not the file: selecting a file means `explorer /select,<path>`
/// on Windows, and that comma is the separator, so a path is the one thing it
/// cannot carry safely.
#[tauri::command]
pub fn update_reveal(state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.data_dir.join("updates");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    shell_open(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_tag_reads_as_a_date_and_a_sequence() {
        assert_eq!(parse("v2026-09-28"), Some(Release { year: 2026, month: 9, day: 28, seq: 1 }));
        assert_eq!(parse("v2026-09-28.2"), Some(Release { year: 2026, month: 9, day: 28, seq: 2 }));
        // `.1` spelled out is the same release as no suffix at all.
        assert_eq!(parse("v2026-09-28.1"), parse("v2026-09-28"));
    }

    #[test]
    fn a_second_release_the_same_day_is_the_newer_one() {
        assert_eq!(verdict(Some("v2026-09-28"), "v2026-09-28.2"), Verdict::Update);
        // And the first is not offered to whoever already has the second: an
        // "update" that goes backwards is worse than no update.
        assert_eq!(verdict(Some("v2026-09-28.2"), "v2026-09-28"), Verdict::Ahead);
        assert_eq!(verdict(Some("v2026-09-28.2"), "v2026-09-28.2"), Verdict::UpToDate);
        // Ten past two, not before it: the suffix is a number.
        assert_eq!(verdict(Some("v2026-09-28.2"), "v2026-09-28.10"), Verdict::Update);
        assert_eq!(verdict(Some("v2026-09-28.10"), "v2026-09-28.2"), Verdict::Ahead);
    }

    #[test]
    fn dates_are_compared_as_numbers_not_as_text() {
        // The day, where text would put 9 above 10.
        assert_eq!(verdict(Some("v2026-09-09"), "v2026-09-10"), Verdict::Update);
        assert_eq!(verdict(Some("v2026-09-10"), "v2026-09-09"), Verdict::Ahead);
        // The month, where text would put 9 above 10.
        assert_eq!(verdict(Some("v2026-09-30"), "v2026-10-01"), Verdict::Update);
        assert_eq!(verdict(Some("v2026-10-01"), "v2026-09-30"), Verdict::Ahead);
    }

    #[test]
    fn the_year_turns_over() {
        assert_eq!(verdict(Some("v2026-12-31"), "v2027-01-01"), Verdict::Update);
        assert_eq!(verdict(Some("v2027-01-01"), "v2026-12-31"), Verdict::Ahead);
        // A second release on new year's eve is still behind new year's day.
        assert_eq!(verdict(Some("v2026-12-31.5"), "v2027-01-01"), Verdict::Update);
    }

    #[test]
    fn a_tag_that_is_not_a_release_tag_is_refused_rather_than_guessed_at() {
        for bad in [
            "2026-09-28",     // no v
            "v2026-9-28",     // not fixed width
            "v2026-09-28.0",  // the suffix counts from 1
            "v2026-09-28.x",  // not a number
            "v2026-13-01",    // no such month
            "v2026-09-32",    // no such day
            "v2026-00-10",    // no month zero
            "v2026-09-00",    // no day zero
            "v0.1.0",         // the manifest's semver, not a release tag
            "nightly",        //
            "",               //
            "v2026-09-28-2",  // the separator is a dot
            "v2026-09-281",   // eleven characters, not ten
            "v 2026-09-28",   //
        ] {
            assert_eq!(parse(bad), None, "{bad:?} is not a release tag");
        }
    }

    #[test]
    fn a_development_build_has_nothing_to_compare() {
        assert_eq!(verdict(None, "v2026-09-28"), Verdict::DevBuild);
        // Even when GitHub's answer is unusable: the build is the reason
        // there is no comparison, and that is what the user is told.
        assert_eq!(verdict(None, "not a tag"), Verdict::DevBuild);
    }

    #[test]
    fn a_wrongly_injected_tag_is_reported_not_ignored() {
        // A workflow that passed the release *title* instead of the tag, say.
        assert_eq!(verdict(Some("2026-09-28 (2)"), "v2026-09-28"), Verdict::BadBuild);
        // A build tag is judged before GitHub's, so the build bug is the one
        // reported even when both are wrong.
        assert_eq!(verdict(Some("0.1.0"), "also wrong"), Verdict::BadBuild);
    }

    #[test]
    fn a_release_github_named_oddly_is_not_compared_to() {
        assert_eq!(verdict(Some("v2026-09-28"), "v1.2.3"), Verdict::BadLatest);
        assert_eq!(verdict(Some("v2026-09-28"), ""), Verdict::BadLatest);
    }

    /// The check makes no request at all for a build that is not a release,
    /// which is both the privacy promise and the reason a contributor's
    /// checkout works offline.
    #[tokio::test]
    async fn a_development_build_asks_github_nothing() {
        assert!(matches!(check(None).await, Check::DevBuild));
    }

    /// And neither does a build whose tag could not be a release: there is no
    /// answer GitHub could give that would make the comparison meaningful.
    #[tokio::test]
    async fn a_bad_build_tag_asks_github_nothing() {
        let found = check(Some("v0.1.0")).await;
        assert!(matches!(found, Check::BadBuild { .. }), "{found:?}");
    }

    /// build.rs compiles the tag in, and nothing checks it between there and
    /// here. So whatever it compiled has to read back as a release tag or as
    /// no release at all, never as something in between -- this is what makes
    /// a release built with the wrong environment variable fail in CI rather
    /// than ship an app that tells everyone its build is broken.
    #[test]
    fn the_compiled_in_tag_is_either_a_release_tag_or_absent() {
        if let Some(tag) = release() {
            assert!(parse(tag).is_some(), "build.rs compiled in DIVIXI_RELEASE={tag:?}, which is not a release tag");
        }
    }

    /// Whatever the build, the interface gets a case it can render: the state
    /// is named in the answer, never left for the caller to infer.
    #[test]
    fn every_case_says_which_it_is() {
        let json = |c: Check| serde_json::to_value(c).unwrap()["kind"].as_str().unwrap().to_string();
        assert_eq!(json(Check::DevBuild), "dev_build");
        assert_eq!(json(Check::NoRelease), "no_release");
        assert_eq!(json(Check::RateLimited { detail: String::new() }), "rate_limited");
        assert_eq!(json(Check::UpToDate { current: "v2026-09-28".into() }), "up_to_date");
    }

    // ----- fetching the installer -----

    /// A release carries every platform's bundle at once. The Windows rule is
    /// tested here whatever the runner is: `pick` takes the rule rather than
    /// reading it off the host.
    fn windows_x64() -> Installer {
        Installer { suffix: "_x64-setup.exe", what: "NSIS installer (per-user)" }
    }

    fn asset(name: &str, digest: Option<&str>) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
            size: 8_384_716,
            digest: digest.map(str::to_string),
        }
    }

    /// The assets of v2026-09-30, by name, as GitHub listed them.
    fn a_release() -> Vec<Asset> {
        [
            "Divixi-0.1.0-1.x86_64.rpm",
            "divixi-server-2026-09-30-x86_64-linux",
            "Divixi_0.1.0_amd64.AppImage",
            "Divixi_0.1.0_amd64.deb",
            "Divixi_0.1.0_universal.app.tar.gz",
            "Divixi_0.1.0_universal.dmg",
            "Divixi_0.1.0_x64-setup.exe",
            "Divixi_0.1.0_x64_en-US.msi",
        ]
        .iter()
        .map(|n| asset(n, Some("sha256:c0eae3e4bbc7fd5405e996c27be97d1b6e37ef1871559901c54b7e0303e51b66")))
        .collect()
    }

    #[test]
    fn the_windows_installer_is_the_nsis_one_and_not_the_msi() {
        let assets = a_release();
        let found = pick(&assets, &windows_x64()).expect("the NSIS installer is in there");
        assert_eq!(found.name, "Divixi_0.1.0_x64-setup.exe");
    }

    #[test]
    fn the_version_in_the_middle_is_not_matched_on() {
        // The manifests move on their own schedule; the release is named by
        // date. A bundle from a later version has to keep being found.
        let assets = vec![asset("Divixi_2.5.0_x64-setup.exe", Some("sha256:00"))];
        assert_eq!(pick(&assets, &windows_x64()).map(|a| a.name.as_str()), Some("Divixi_2.5.0_x64-setup.exe"));
    }

    #[test]
    fn a_release_without_this_platforms_bundle_has_nothing_to_offer() {
        let assets: Vec<Asset> = a_release().into_iter().filter(|a| !a.name.ends_with("-setup.exe")).collect();
        assert!(pick(&assets, &windows_x64()).is_none());
        // And an ARM Windows machine looking at the same release.
        let arm = Installer { suffix: "_arm64-setup.exe", what: "" };
        assert!(pick(&a_release(), &arm).is_none());
    }

    /// The name becomes a file name, so a name that is not one is refused
    /// rather than tidied up. An asset can be uploaded by hand.
    #[test]
    fn an_asset_name_that_is_not_a_file_name_is_refused() {
        assert!(safe_name("Divixi_0.1.0_x64-setup.exe"));
        for bad in [
            r"..\..\Windows\System32\calc_x64-setup.exe",
            "../../tmp/evil_x64-setup.exe",
            "sub/dir_x64-setup.exe",
            "C:_x64-setup.exe",
            ".hidden_x64-setup.exe",
            "",
        ] {
            assert!(!safe_name(bad), "{bad:?} is not a file name");
        }
        // And `pick` will not hand one back even when the suffix matches.
        let assets = vec![asset(r"..\..\evil_x64-setup.exe", Some("sha256:00"))];
        assert!(pick(&assets, &windows_x64()).is_none());
    }

    #[test]
    fn a_digest_is_read_only_in_the_shape_github_publishes() {
        let hex = "C0EAE3E4BBC7FD5405E996C27BE97D1B6E37EF1871559901C54B7E0303E51B66";
        assert_eq!(expected_hex(&format!("sha256:{hex}")).as_deref(), Some(hex.to_ascii_lowercase().as_str()));
        for bad in [
            "sha512:c0eae3e4bbc7fd5405e996c27be97d1b6e37ef1871559901c54b7e0303e51b66",
            "c0eae3e4bbc7fd5405e996c27be97d1b6e37ef1871559901c54b7e0303e51b66", // no algorithm
            "sha256:c0eae3e4",                                                  // cut short
            "sha256:",                                                          //
            "sha256:zzeae3e4bbc7fd5405e996c27be97d1b6e37ef1871559901c54b7e0303e51b66",
            "",
        ] {
            assert_eq!(expected_hex(bad), None, "{bad:?} is not a digest to check against");
        }
    }

    /// A folder of this test's own, removed when it is done.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(what: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("divixi-update-{what}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn file(&self, name: &str, body: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, body).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_right_hash_passes() {
        let scratch = Scratch::new("right");
        let body = b"divixi installer bytes";
        let path = scratch.file("Divixi_0.1.0_x64-setup.exe", body);
        // Computed here rather than written down: the point of the test is
        // that `verify` agrees with `sha256_file`, and a constant typed in by
        // hand would only test the typing.
        let hex = sha256_file(&path).unwrap();
        assert_eq!(hex.len(), 64);
        assert_eq!(verify(&path, &hex), Ok(()));
    }

    /// The hash sha2 computes is the hash everyone else computes. One known
    /// answer, so a wrong algorithm cannot pass the test above by agreeing
    /// with itself.
    #[test]
    fn the_hash_is_the_sha256_everything_else_computes() {
        let scratch = Scratch::new("known");
        // The empty string's SHA-256, the most quoted one there is.
        let empty = scratch.file("empty_x64-setup.exe", b"");
        assert_eq!(sha256_file(&empty).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        // And "abc", the FIPS 180-4 example.
        let abc = scratch.file("abc_x64-setup.exe", b"abc");
        assert_eq!(sha256_file(&abc).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_hash_that_is_not_the_published_one_is_refused() {
        let scratch = Scratch::new("wrong");
        let path = scratch.file("Divixi_0.1.0_x64-setup.exe", b"someone else's bytes");
        let mine = sha256_file(&path).unwrap();
        // The published hash of a different file: the download that arrives
        // when a mirror, a proxy or an attacker answers instead of GitHub.
        let published = "c0eae3e4bbc7fd5405e996c27be97d1b6e37ef1871559901c54b7e0303e51b66";
        assert_eq!(verify(&path, published), Err(Bad::Mismatch { got: mine }));
    }

    /// The case the byte count cannot catch on its own: a connection that ended
    /// early leaves a file that is a *prefix* of the right one. Every byte in
    /// it is correct and it is still not the installer.
    #[test]
    fn a_file_that_was_cut_short_is_refused() {
        let scratch = Scratch::new("short");
        let whole = b"divixi installer bytes, all of them";
        let full = scratch.file("full_x64-setup.exe", whole);
        let published = sha256_file(&full).unwrap();
        let cut = scratch.file("Divixi_0.1.0_x64-setup.exe", &whole[..whole.len() - 9]);
        let refused = verify(&cut, &published);
        assert!(matches!(refused, Err(Bad::Mismatch { .. })), "{refused:?}");
        // A file grown past the right length is refused the same way: the hash
        // is over the whole file, not the first n bytes of it.
        let mut longer = whole.to_vec();
        longer.extend_from_slice(b" and a bit more");
        let grown = scratch.file("grown_x64-setup.exe", &longer);
        assert!(matches!(verify(&grown, &published), Err(Bad::Mismatch { .. })));
    }

    #[test]
    fn a_file_that_is_not_there_is_not_a_pass() {
        let scratch = Scratch::new("missing");
        let gone = scratch.0.join("never_written_x64-setup.exe");
        // Any hex will do: the file it names is not there, and "I could not
        // read it" must never come out as "it matched".
        let refused = verify(&gone, &"0".repeat(64));
        assert!(matches!(refused, Err(Bad::Unreadable(_))), "{refused:?}");
    }

    /// Nothing but the file about to be written survives a sweep: not an older
    /// release's installer, and not a `.part` a killed run left behind. That is
    /// what makes a half-downloaded file impossible to use after a restart.
    #[test]
    fn the_folder_is_emptied_of_half_downloads_and_older_installers() {
        let scratch = Scratch::new("sweep");
        scratch.file("Divixi_0.1.0_x64-setup.exe.part", b"half of a download");
        scratch.file("Divixi_0.0.9_x64-setup.exe", b"last month's installer");
        scratch.file("Divixi_0.1.0_x64-setup.exe", b"the one being written");
        sweep(&scratch.0, "Divixi_0.1.0_x64-setup.exe");
        let mut left: Vec<String> = std::fs::read_dir(&scratch.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["Divixi_0.1.0_x64-setup.exe"]);
    }

    /// The mark the shell reads before it runs a downloaded file, written
    /// because the file really did come off the internet. It makes Windows warn
    /// *more*, not less; see `mark_from_internet` on why that is the point.
    #[cfg(windows)]
    #[test]
    fn the_installer_is_marked_as_coming_from_the_internet() {
        let scratch = Scratch::new("motw");
        let path = scratch.file("Divixi_0.1.0_x64-setup.exe", b"setup");
        mark_from_internet(&path, "https://example.invalid/Divixi_0.1.0_x64-setup.exe");
        let zone = std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).expect("the stream is there");
        assert!(zone.contains("ZoneId=3"), "{zone:?}");
        assert!(zone.contains("HostUrl=https://example.invalid/"), "{zone:?}");
        // The file itself is untouched: the mark is a stream beside it, so the
        // hash that was just checked still holds.
        assert_eq!(std::fs::read(&path).unwrap(), b"setup");
    }

    /// `update_open` takes a path that has been through the webview, so it
    /// rebuilds it from the folder and the name and refuses anything that is
    /// not this platform's installer sitting in that folder.
    #[cfg(windows)]
    #[test]
    fn only_the_installer_in_the_updates_folder_can_be_opened() {
        let scratch = Scratch::new("verified");
        let installer = scratch.file("Divixi_0.1.0_x64-setup.exe", b"setup");
        scratch.file("notes.txt", b"not an installer");
        assert_eq!(verified(&scratch.0, &installer.display().to_string()).unwrap(), installer);
        // Only the name is used, so a path pointing anywhere else lands back
        // in the updates folder or nowhere at all.
        assert_eq!(verified(&scratch.0, r"C:\Windows\Temp\Divixi_0.1.0_x64-setup.exe").unwrap(), installer);
        for refused in [
            r"C:\Windows\System32\calc.exe",
            "notes.txt",
            "Divixi_0.1.0_x64_en-US.msi",
            "Divixi_0.1.0_x64-setup.exe.part",
            "",
        ] {
            assert!(verified(&scratch.0, refused).is_err(), "{refused:?} is not openable");
        }
    }

    /// Whatever the platform, the interface gets a case it can render.
    #[test]
    fn every_download_case_says_which_it_is() {
        let json = |d: Download| serde_json::to_value(d).unwrap()["kind"].as_str().unwrap().to_string();
        assert_eq!(json(Download::Cancelled), "cancelled");
        assert_eq!(json(Download::Nothing), "nothing");
        assert_eq!(json(Download::Busy), "busy");
        assert_eq!(json(Download::Corrupt { expected: String::new(), got: String::new() }), "corrupt");
        assert_eq!(json(Download::Ready { tag: "v2026-09-30".into(), name: "x".into(), path: "y".into(), size: 1 }), "ready");
        let phase = |p: Phase| serde_json::to_value(Progress { phase: p, received: 0, total: None }).unwrap()["phase"].as_str().unwrap().to_string();
        assert_eq!(phase(Phase::Checking), "checking");
        assert_eq!(phase(Phase::Downloading), "downloading");
        assert_eq!(phase(Phase::Verifying), "verifying");
        assert_eq!(phase(Phase::Ready), "ready");
    }

    /// The check tells the interface whether the download button belongs on
    /// the card, and that answer comes from the one function that knows the
    /// platform -- not from the user agent in the webview.
    #[test]
    fn only_windows_is_offered_a_download_today() {
        assert_eq!(installer().is_some(), cfg!(windows));
    }

    /// The whole of it against the real release: the request, the asset, the
    /// bytes, the hash, the rename. Not part of `cargo test` -- it needs the
    /// network and pulls down a real installer -- and it only does anything at
    /// all for a build that thinks it is an older release than the newest
    /// published one. Run it by hand:
    ///
    /// ```text
    /// DIVIXI_RELEASE_TAG=v2026-09-01 cargo test -p orchestra-app --lib \
    ///     update::tests::the_real_installer -- --ignored --nocapture
    /// ```
    ///
    /// Anything other than `Ready` fails the test, including "there is nothing
    /// newer": the run was asked for on purpose, and a silent pass would say
    /// the download works when nothing was downloaded.
    #[tokio::test]
    #[ignore = "needs the network and a build tagged as an older release"]
    async fn the_real_installer_downloads_and_verifies() {
        let scratch = Scratch::new("e2e");
        let mut phases: Vec<Phase> = Vec::new();
        let mut last = Progress { phase: Phase::Checking, received: 0, total: None };
        let got = download(&scratch.0, |p| {
            if phases.last() != Some(&p.phase) {
                phases.push(p.phase);
                println!("[phase] {:?} {} / {:?}", p.phase, p.received, p.total);
            }
            last = p;
        })
        .await;
        println!("{got:?}");
        let Download::Ready { name, path, size, .. } = &got else {
            panic!("expected Ready, got {got:?}");
        };
        // The phases the interface draws, in the order it draws them.
        assert_eq!(phases, vec![Phase::Checking, Phase::Downloading, Phase::Verifying, Phase::Ready]);
        assert_eq!(last.phase, Phase::Ready);
        // The file is under the name the release gave it, in the folder asked
        // for, and it is as long as the download counted.
        let file = std::path::Path::new(path);
        assert_eq!(file.parent(), Some(scratch.0.as_path()));
        assert_eq!(file.file_name().and_then(|n| n.to_str()), Some(name.as_str()));
        assert_eq!(std::fs::metadata(file).unwrap().len(), *size);
        // Nothing half-finished is left beside it.
        let left: Vec<String> = std::fs::read_dir(&scratch.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec![name.clone()]);
        // And the file that is there is the one the release published: the
        // check is run again here, from the outside, against the digest the
        // API gives -- so a `verify` that always said yes could not pass this.
        let published = latest().await.ok().and_then(|l| l.assets.into_iter().find(|a| a.name == *name).and_then(|a| a.digest)).expect("the asset has a digest");
        assert_eq!(sha256_file(file).unwrap(), expected_hex(&published).unwrap());
        // A second run finds it already there and does not fetch it again.
        let again = download(&scratch.0, |_| {}).await;
        assert!(matches!(again, Download::Ready { .. }), "{again:?}");
    }
}
