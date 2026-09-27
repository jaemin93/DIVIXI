//! Whether a newer release of divixi has been published.
//!
//! One command, run only when a person presses the button on the settings
//! page. It asks GitHub for the newest published release of `jaemin93/divixi`,
//! compares that release's tag against the tag this binary was built from, and
//! says which of the two is newer. That is all it does.
//!
//! ## Why it stops at telling you
//!
//! divixi has no code-signing certificate. Tauri's updater plugin will not
//! install anything without a signature -- and it should not: an update
//! channel that installs unsigned code is a way in for whoever can answer for
//! the server. So the app tells you a release exists and opens its page in
//! your browser; downloading and installing stays yours. If a certificate is
//! ever bought, the decision to revisit this is `tauri-plugin-updater` plus
//! `plugins.updater.pubkey` in tauri.conf.json, and the release workflow has
//! to start producing `latest.json` and `.sig` artifacts. Until then, none of
//! that exists and this module is the whole feature.
//!
//! Windows SmartScreen and macOS Gatekeeper both warn about the unsigned
//! bundles, which is why the button that opens the release page says so.
//!
//! ## Privacy
//!
//! The README promises that divixi sends nothing about you anywhere. This is
//! the one request the app ever makes on its own behalf, so it is kept to the
//! smallest shape that can work:
//!
//!   * it happens only on a press of the button -- never at startup, never on
//!     a timer, never in the background;
//!   * it carries no authentication. The endpoint is public, so the user's
//!     GitHub token (which exists for remote instances, `remote::github`) is
//!     deliberately NOT sent: it would name the user to GitHub for no gain;
//!   * it carries no version, no platform, no identifier of any kind. The
//!     `User-Agent` is the bare word `divixi`, which the GitHub API requires
//!     of every caller;
//!   * unauthenticated, GitHub allows 60 requests an hour per address, which
//!     a button nobody presses sixty times an hour will not reach. Being
//!     rate-limited is reported rather than retried.
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

use std::time::Duration;

use serde::Deserialize;

/// The repository the releases come from. Public: no token is used to read it.
const REPO: &str = "jaemin93/divixi";

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
struct Release {
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
fn parse(tag: &str) -> Option<Release> {
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
    Update { current: String, latest: String, url: String },
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
}

/// A client with no identity in it and a deadline, so a hung connection ends
/// as an answer rather than a button that spins forever.
fn http() -> Result<reqwest::Client, String> {
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
/// Runs on a press of "check for updates" and at no other time. See the
/// module header for why nothing is downloaded or installed.
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

    let client = match http() {
        Ok(c) => c,
        Err(e) => return Check::Failed { detail: format!("could not build an HTTP client: {e}") },
    };
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let res = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(res) => res,
        // Anything that never became an answer: no route, DNS, TLS, timeout.
        Err(e) => return Check::Offline { detail: e.to_string() },
    };

    let status = res.status();
    if !status.is_success() {
        // GitHub answers a spent allowance with 403 (or 429) and
        // `x-ratelimit-remaining: 0`. The status alone cannot tell that from
        // any other refusal, so the header decides.
        let spent = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).map(|v| v.trim() == "0").unwrap_or(false);
        if spent || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Check::RateLimited { detail: format!("GitHub answered {status}; the unauthenticated allowance of 60 requests an hour is spent") };
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            // No published release. divixi's nightly releases are drafts
            // until a person publishes them, and a repository whose releases
            // are all still drafts answers 404 here.
            return Check::NoRelease;
        }
        return Check::Failed { detail: format!("GitHub answered {status}") };
    }

    let body = match res.bytes().await {
        Ok(b) => b,
        Err(e) => return Check::Offline { detail: format!("the answer did not finish: {e}") },
    };
    let latest: Latest = match serde_json::from_slice(&body) {
        Ok(l) => l,
        Err(e) => return Check::Failed { detail: format!("GitHub's answer was not a release: {e}") },
    };
    if latest.draft || latest.prerelease {
        // Cannot happen through this endpoint; see [`Latest`].
        return Check::Failed { detail: format!("GitHub offered {} as the latest release, but it is a draft or a prerelease", latest.tag_name) };
    }

    let current = current.unwrap_or_default();
    match verdict(Some(current), &latest.tag_name) {
        Verdict::Update => Check::Update {
            current: current.to_string(),
            latest: latest.tag_name.clone(),
            // The release's own page when GitHub named it, which it always
            // does; the tag's page otherwise. Either lands on the downloads.
            url: latest.html_url.unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{}", latest.tag_name)),
        },
        Verdict::UpToDate => Check::UpToDate { current: current.to_string() },
        Verdict::Ahead => Check::Ahead { current: current.to_string(), latest: latest.tag_name },
        Verdict::BadLatest => Check::Failed { detail: format!("GitHub's newest release is tagged {:?}, which is not a release tag", latest.tag_name) },
        // Both ruled out above, before the request was made.
        Verdict::DevBuild | Verdict::BadBuild => Check::BadBuild { current: current.to_string() },
    }
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
}
