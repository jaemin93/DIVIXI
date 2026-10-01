//! Where this machine sits on its tailnet, and putting Divixi's server there.
//!
//! Phone access reaches this Divixi over Tailscale: `tailscale serve` fronts
//! the tailnet with TLS and proxies to `127.0.0.1`, so the app's own server
//! (`remote::start`) keeps its loopback bind and the machine's exposure on the
//! LAN does not change. The browser gets a real certificate, which a phone
//! needs: without a secure context there is no installable page and no service
//! worker later.
//!
//! Two halves live here, with opposite contracts, and they must not be mixed:
//!
//! * The **read** half ([`probe`], [`serve_state`]) diagnoses. Nothing raises:
//!   every failure lands in a field. The card's whole job is to name the next
//!   errand, and "Tailscale is not working" is not an errand -- "install it",
//!   "start it", "sign in", "turn MagicDNS on" are four different ones.
//! * The **write** half ([`publish`], [`unpublish`]) acts. Here a failure is
//!   the point of the call: `tailscale serve` refuses for reasons nobody can
//!   guess from a summary (most often that changing serve configuration needs
//!   root or an `--operator` grant), so whatever the daemon said is passed
//!   through verbatim and our classification is only ever added beside it.
//!
//! The arrangement -- the diagnosing probe, the three-valued serve state, the
//! refusal to overwrite a mount we cannot prove is ours, and passing the
//! daemon's own words through -- is reimplemented from Kiro Crew (Apache-2.0),
//! `src/kiro_crew/dashboard/tailnet.py` and `tailnet_serve.py`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// Where the CLI is accepted from: **vetted absolute paths only, never
/// `PATH`**. A `PATH` lookup would make the binary itself choosable by whoever
/// can write a directory on it -- and an agent running under this app can write
/// plenty -- so a planted `tailscale` would be executed by the next press of
/// the button. The arguments here were never agent-influenced; without this the
/// binary would be.
///
/// A non-standard install is not found. That is the trade: the card says
/// Tailscale is not installed and the human can still publish by hand.
const CANDIDATES: &[&str] = &[
    #[cfg(windows)]
    r"C:\Program Files\Tailscale\tailscale.exe",
    #[cfg(windows)]
    r"C:\Program Files (x86)\Tailscale\tailscale.exe",
    #[cfg(not(windows))]
    "/usr/bin/tailscale",
    #[cfg(not(windows))]
    "/usr/local/bin/tailscale",
    #[cfg(not(windows))]
    "/opt/homebrew/bin/tailscale",
    // The macOS app ships the binary inside and does not always symlink it.
    #[cfg(target_os = "macos")]
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

/// A read is local and must not make the human wait: `tailscale status` blocks
/// while the daemon is still starting.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A publish is a daemon round trip. Longer, because it is asked for by hand
/// and reporting a success as a timeout is worse than waiting.
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// A whois is on the path of a request, so it may not make one wait: it is a
/// local round trip to a daemon that already knows the answer.
const WHOIS_TIMEOUT: Duration = Duration::from_secs(2);

/// The HTTPS port `tailscale serve` fronts. 443 is its own default, and the
/// reason the address carries no port: a browser leaves `:443` out of `Origin`,
/// which is what the origin check in a later stage will compare against.
pub const SERVE_PORT: u16 = 443;

/// The mount we publish at. Passed **explicitly when withdrawing**, which is
/// load-bearing rather than tidy: upstream treats an absent `--set-path` as
/// "every mount under this port", collects them all and deletes them -- so a
/// port-wide `off` would take out handlers somebody added by hand. It also
/// prompts interactively when there is more than one, and this command has no
/// terminal to answer in.
pub const SERVE_MOUNT: &str = "/";

/// How much of the daemon's output is carried into a `detail`. The reason leads
/// the stream; the pathological case is a status read that timed out part-way
/// through a large document.
const DETAIL_MAX: usize = 1000;

/// What this machine can do about its tailnet, and what is in the way.
///
/// Every negative is a different errand, so they are separate fields rather
/// than one "working" flag. Nothing here raises: a failure is a field.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Probe {
    /// A CLI at one of [`CANDIDATES`].
    pub installed: bool,
    /// The daemon answered a status read.
    pub reachable: bool,
    /// It answered, and said Tailscale is stopped (`tailscale down`). The
    /// daemon *is* running, so this is not `reachable: false`; the errand is to
    /// bring Tailscale up, not to start a service.
    pub stopped: bool,
    pub logged_in: bool,
    /// This machine's MagicDNS name, validated against the tailnet's own
    /// suffix. Empty when there is none.
    pub name: String,
    /// Whether the tailnet will provision a certificate for [`Probe::name`].
    /// `None` means the status document did not say -- an older client may
    /// still publish fine, and the `serve` call is the authority. It must not
    /// be rendered as `false`.
    pub https: Option<bool>,
    /// The tailnet's *other* devices. These answer a question no amount of
    /// local state can: whether there is a phone to reach this Divixi from.
    /// Publishing succeeds on a tailnet of one and the scan then fails in the
    /// browser with nothing on this machine wrong.
    pub peers: usize,
    pub peers_online: usize,
    /// The tailnet's name, for the card to say which one this is. A shared work
    /// tailnet is not a private network and the human should see which they are
    /// about to publish on.
    pub tailnet: String,
    /// What happened, in the daemon's words where there are any.
    pub detail: String,
}

/// Whether serve is fronting this Divixi, as far as we can tell.
///
/// Both flags are three-valued on purpose. `None` is "could not tell" -- no
/// CLI, no answer, or a document this build does not recognise -- and rendering
/// that as `false` is how a check that never ran gets shown as a clean result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ServeState {
    /// Serve is proxying [`SERVE_PORT`]`[`SERVE_MOUNT`] to our port.
    pub published: Option<bool>,
    /// The status document provably holds nothing on [`SERVE_PORT`], so
    /// publishing would replace nothing. `None` is treated exactly like
    /// `false` by the write guards.
    pub port_free: Option<bool>,
    pub detail: String,
}

/// Why a publish or withdrawal did not happen, for the UI to branch on. The
/// `detail` beside it is for the human and carries the daemon's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Ok,
    NoCli,
    NoPermission,
    DaemonUnavailable,
    Timeout,
    /// Something is at the mount and we cannot prove it is ours.
    NotOurs,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub code: Code,
    pub detail: String,
}

impl Outcome {
    fn bad(code: Code, detail: impl Into<String>) -> Self {
        Self { ok: false, code, detail: detail.into() }
    }
}

pub fn cli_path() -> Option<PathBuf> {
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// How a run ended before the CLI could produce an exit status. Kept apart
/// because each needs different words: "not installed" about a binary that is
/// plainly there is exactly the misleading diagnosis this module exists to
/// avoid.
enum Ran {
    NoCli,
    /// Timed out, with whatever was captured before the deadline. Carrying the
    /// output is the point: on a tailnet where Serve is not enabled,
    /// `tailscale serve` prints the enablement URL and then blocks forever, so
    /// the timeout is the only reachable outcome and that output is the only
    /// diagnosis there is.
    Timeout { out: String, err: String },
    /// The binary is there but would not start.
    NoStart(String),
    Done { status: i32, out: String, err: String },
}

/// Read a pipe to its end, alongside the wait.
///
/// Both pipes are drained by their own task rather than by
/// `wait_with_output`, because that future gives nothing back when it is
/// cancelled -- and what it would have given back is the whole diagnosis in
/// the case that matters most (see [`Ran::Timeout`]). Killing the child closes
/// the pipes, so these finish on their own straight after.
fn drain<R>(pipe: Option<R>) -> tokio::task::JoinHandle<String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf).await;
        }
        // Lossy: a deadline can cut a multibyte sequence, and one mangled
        // character beats a dropped reason.
        String::from_utf8_lossy(&buf).into_owned()
    })
}

async fn run(args: &[&str], timeout: Duration) -> Ran {
    let Some(cli) = cli_path() else { return Ran::NoCli };
    let mut cmd = tokio::process::Command::new(cli);
    #[cfg(windows)]
    {
        // No console window flashing up behind the app for a status read.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran::NoStart(e.to_string()),
    };
    let reading_out = drain(child.stdout.take());
    let reading_err = drain(child.stderr.take());
    let waited = tokio::time::timeout(timeout, child.wait()).await;
    if waited.is_err() {
        // Kill first so the pipes close and the readers finish; then take
        // whatever they got before the deadline.
        let _ = child.kill().await;
    }
    let out = reading_out.await.unwrap_or_default();
    let err = reading_err.await.unwrap_or_default();
    match waited {
        Err(_) => Ran::Timeout { out, err },
        Ok(Err(e)) => Ran::NoStart(e.to_string()),
        Ok(Ok(status)) => Ran::Done { status: status.code().unwrap_or(-1), out, err },
    }
}

/// Whatever the daemon said, wherever it said it.
///
/// stderr leads, because that is where a refusal belongs -- but upstream prints
/// some of its most useful messages to stdout (on a tailnet where Serve is not
/// enabled, the enablement URL goes there), so stdout is kept and follows.
/// Building this from stderr alone drops exactly that URL.
fn said(out: &str, err: &str) -> String {
    let (out, err) = (out.trim(), err.trim());
    let text = match (err.is_empty(), out.is_empty()) {
        (false, false) => format!("{err}\n{out}"),
        (false, true) => err.to_string(),
        (true, _) => out.to_string(),
    };
    match text.char_indices().nth(DETAIL_MAX) {
        Some((i, _)) => format!("{} …", &text[..i]),
        None => text,
    }
}

/// A guess at why a non-zero exit happened, never the only thing reported.
///
/// Matching on message text is fragile -- upstream owns this wording -- so this
/// only ever adds a hint beside the verbatim output. Fed the *leading* stream
/// only: with both in view, an incidental "operator" in stdout would turn a
/// daemon-down failure into a confidently wrong permissions remedy.
fn classify(output: &str) -> Code {
    let low = output.to_lowercase();
    if ["access denied", "permission denied", "must be run as root", "operator"].iter().any(|s| low.contains(s)) {
        return Code::NoPermission;
    }
    if ["not running", "cannot connect", "connection refused", "logged out", "not logged in", "tailscale is stopped"]
        .iter()
        .any(|s| low.contains(s))
    {
        return Code::DaemonUnavailable;
    }
    Code::Failed
}

/// `BackendState` values meaning the daemon runs but this machine is not signed
/// in. Upstream's own enum; only the not-signed-in half is listed, because
/// treating an unknown future value as "signed in" would send the human to the
/// wrong errand.
const NEEDS_LOGIN: &[&str] = &["NeedsLogin", "NoState", "NeedsMachineAuth"];

/// A MagicDNS name we are willing to put in front of a human and, later, to
/// compare an `Origin` against. An allowlist, not a denylist: the question is
/// not "does this look dangerous" but "is this provably a bare hostname under
/// this tailnet's own suffix".
///
/// The suffix is taken from the same status document rather than hardcoded:
/// upstream documents it as tailnet-specific (`ts.net` is not the only one, and
/// a self-hosted control plane has its own), so a fixed list would reject real
/// tailnets and rot.
fn valid_name(raw: &str, suffix: &str) -> Option<String> {
    let name = raw.trim().trim_end_matches('.').to_string();
    let suffix = suffix.trim().trim_matches('.').to_lowercase();
    if name.is_empty() || suffix.is_empty() || name.len() > 253 {
        return None;
    }
    if name.chars().any(|c| "/:@?# \t\r\n\\".contains(c)) || name != name.to_lowercase() {
        return None;
    }
    // Under the suffix, not the suffix itself and not merely containing it:
    // `desk.tail.ts.net.evil.com` must not pass.
    if !name.ends_with(&format!(".{suffix}")) {
        return None;
    }
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    let labels: Vec<&str> = name.split('.').collect();
    (labels.len() >= 2 && labels.iter().all(|l| label_ok(l))).then_some(name)
}

/// Ask the daemon who a tailnet address belongs to.
///
/// The identity half of this module: `peer.rs` decides what a resolved login
/// is allowed to do, and this only fetches it. Here rather than there because
/// the CLI is found and run in exactly one place -- the vetted-absolute-path
/// rule is only worth anything if nothing else spawns this binary.
///
/// `None` on every failure, which is the fail-closed direction: no peer
/// resolves and the request falls back to what its token alone earns it.
/// Short timeout, because this is on the path of a request.
pub async fn whois(addr: &str) -> Option<Value> {
    run_json(&["whois", "--json", addr], WHOIS_TIMEOUT).await
}

/// Run the CLI and parse stdout as JSON. `None` on any failure: the callers
/// here want a document or nothing.
async fn run_json(args: &[&str], timeout: Duration) -> Option<Value> {
    match run(args, timeout).await {
        Ran::Done { status: 0, out, .. } => serde_json::from_str(&out).ok(),
        _ => None,
    }
}

/// Diagnose the daemon for the phone-access card. Never fails.
pub async fn probe() -> Probe {
    if cli_path().is_none() {
        return Probe { detail: "Tailscale is not installed in a standard location.".into(), ..Default::default() };
    }
    let Some(status) = run_json(&["status", "--json"], READ_TIMEOUT).await else {
        return Probe {
            installed: true,
            detail: "Tailscale is installed, but its daemon did not answer.".into(),
            ..Default::default()
        };
    };
    let base = Probe { installed: true, reachable: true, ..Default::default() };
    let backend = status.get("BackendState").and_then(Value::as_str).unwrap_or_default();

    // Checked before the login test, which a stopped daemon passes: the machine
    // may well still be signed in while nothing on the tailnet can reach it.
    if backend == "Stopped" {
        return Probe {
            stopped: true,
            logged_in: true,
            detail: "Tailscale is stopped, so this machine is not on its tailnet.".into(),
            ..base
        };
    }
    if NEEDS_LOGIN.contains(&backend) {
        return Probe { detail: "Tailscale is running, but this machine is not signed in.".into(), ..base };
    }

    let peer = status.get("Peer").and_then(Value::as_object);
    let peers = peer.map_or(0, |p| p.len());
    let peers_online = peer.map_or(0, |p| p.values().filter(|v| v.get("Online") == Some(&Value::Bool(true))).count());
    let current = status.get("CurrentTailnet");
    let tailnet = current.and_then(|t| t.get("Name")).and_then(Value::as_str).unwrap_or_default().to_string();
    let base = Probe { logged_in: true, peers, peers_online, tailnet, ..base };

    // `CurrentTailnet` is absent when the node is not connected. The top-level
    // suffix is upstream-deprecated, so it is a fallback for an older daemon
    // and never the primary read.
    let suffix = current
        .and_then(|t| t.get("MagicDNSSuffix"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| status.get("MagicDNSSuffix").and_then(Value::as_str))
        .unwrap_or_default();
    let name = status
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(Value::as_str)
        .and_then(|raw| valid_name(raw, suffix))
        .unwrap_or_default();
    if name.is_empty() {
        return Probe {
            detail: "Signed in, but this machine has no MagicDNS name — MagicDNS may be off for this tailnet.".into(),
            ..base
        };
    }

    // `CertDomains` is the control plane's list of names it will help get a
    // certificate for. An explicit list without ours is a real prerequisite; an
    // absent or malformed field stays unknown, so an older client reaches the
    // authoritative `serve` call instead of being blocked forever by a field it
    // never emitted.
    let https = status.get("CertDomains").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(Value::as_str)
            .any(|d| d.trim().trim_end_matches('.').eq_ignore_ascii_case(&name))
    });
    Probe { name, https, ..base }
}

/// The port a status-document key names, if it names one.
///
/// Serve documents key mappings by port two ways -- a bare `"443"` and a
/// `"host:443"` suffix -- and this is the one parse both predicates below
/// share. ASCII digits only and anchored at the end: a key that counted as port
/// evidence for one predicate while escaping the other would be evidence of a
/// port-keyed schema that hides the very mapping the evidence is about.
fn key_port(key: &str) -> Option<u16> {
    let tail = key.rsplit(':').next()?;
    (!tail.is_empty() && tail.len() <= 5 && tail.bytes().all(|b| b.is_ascii_digit()))
        .then(|| tail.parse().ok())
        .flatten()
        .filter(|p| *p > 0)
}

/// Subtrees whose own key names `port`, at any depth.
///
/// "Is this Divixi served anywhere" is the wrong question for a withdrawal: a
/// Divixi published on 8443 with something unrelated on 443 would read as ours,
/// and the removal would take out the unrelated mapping.
fn port_subtrees<'a>(node: &'a Value, port: u16, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                if key_port(k) == Some(port) {
                    out.push(v);
                }
                port_subtrees(v, port, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| port_subtrees(v, port, out)),
        _ => {}
    }
}

/// Subtrees whose own key is `mount`. The same narrowing one level deeper:
/// withdrawal removes a single handler, so ownership has to be decided for that
/// handler. "Ours is somewhere under 443" was true while a stranger's handler
/// sat at the mount actually being removed.
fn mount_subtrees<'a>(node: &'a Value, mount: &str, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                if k == mount {
                    out.push(v);
                }
                mount_subtrees(v, mount, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| mount_subtrees(v, mount, out)),
        _ => {}
    }
}

/// Whether any string anywhere under `node` is one of `needles`.
///
/// Structure-agnostic on purpose: the exact schema of
/// `tailscale serve status --json` is barely observed here, so reading a key
/// path would be a guess that fails *silently* -- reporting "not published" for
/// a Divixi that is. Walking values only asks that the proxy target appear
/// somewhere, which is true of any shape that records it at all.
fn finds_target(node: &Value, needles: &[String]) -> bool {
    match node {
        Value::String(s) => needles.iter().any(|n| s.trim_end_matches('/') == n),
        Value::Object(map) => map.values().any(|v| finds_target(v, needles)),
        Value::Array(items) => items.iter().any(|v| finds_target(v, needles)),
        _ => false,
    }
}

/// Whether any key anywhere names a port.
///
/// The evidence that lets [`serve_state`] say "our port is free" rather than
/// "unknown" when a document holds configuration only for other ports. The
/// reasoning is the document's own shape, not a hardcoded path: one document
/// does not key one mapping by port and record another somewhere else, so a
/// document that demonstrably keys by port and names none of ours has nothing
/// on ours. Keys only -- a proxy target like `http://127.0.0.1:7488` names a
/// port too, but only keys carry the indexing.
fn has_port_keys(node: &Value) -> bool {
    match node {
        Value::Object(map) => map.iter().any(|(k, v)| key_port(k).is_some() || has_port_keys(v)),
        Value::Array(items) => items.iter().any(has_port_keys),
        _ => false,
    }
}

/// Whether serve is fronting Divixi's `port`.
/// Whether a document carries no serve configuration at all.
///
/// `{}` and `null` are the obvious shapes, but not the only ones: Go marshals
/// a `ServeConfig` whose maps are nil as `{"TCP":null,"Web":null}`, and that is
/// "nothing is served", not "something this build cannot read". Reading it as
/// the latter is what refused the very first publish on a machine with no
/// serve configuration -- the one case that has to work.
fn is_empty_doc(node: &Value) -> bool {
    match node {
        Value::Null => true,
        Value::Object(map) => map.values().all(is_empty_doc),
        Value::Array(items) => items.iter().all(is_empty_doc),
        _ => false,
    }
}

/// What a serve-status document says about Divixi on `port`.
///
/// Pure, and the whole of the classification: [`serve_state`] only fetches the
/// document and hands it here, so a test of this is a test of what runs.
fn classify_doc(doc: &Value, port: u16) -> ServeState {
    let unknown = |detail: String| ServeState { published: None, port_free: None, detail };
    if is_empty_doc(doc) {
        return ServeState {
            published: Some(false),
            port_free: Some(true),
            detail: "No serve configuration is active.".into(),
        };
    }

    let mut scoped = Vec::new();
    port_subtrees(doc, SERVE_PORT, &mut scoped);
    if scoped.is_empty() {
        // Nothing keys our port. When the document demonstrably keys mappings
        // by port, that absence is a determination: everything serve holds
        // sits on other ports and this write endangers none of it. Another
        // project on port 80 must not read as "something is on 443".
        return if has_port_keys(doc) {
            ServeState {
                published: Some(false),
                port_free: Some(true),
                detail: format!("Serve is configured for other ports only; nothing is on port {SERVE_PORT}."),
            }
        } else {
            unknown(format!("Serve is configured, but this build could not tell what is on port {SERVE_PORT}."))
        };
    }

    let mut mounts = Vec::new();
    for sub in &scoped {
        mount_subtrees(sub, SERVE_MOUNT, &mut mounts);
    }
    if mounts.is_empty() {
        return ServeState {
            published: None,
            port_free: Some(false),
            detail: format!("Serve is configured on port {SERVE_PORT}, but this build could not tell what is at {SERVE_MOUNT}."),
        };
    }
    let needles = [format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}")];
    if mounts.iter().any(|m| finds_target(m, &needles)) {
        ServeState {
            published: Some(true),
            port_free: Some(false),
            detail: format!("Serve is proxying {SERVE_PORT}{SERVE_MOUNT} to Divixi on port {port}."),
        }
    } else {
        ServeState {
            published: Some(false),
            port_free: Some(false),
            detail: format!("Serve is configured on {SERVE_PORT}{SERVE_MOUNT}, but not for this Divixi."),
        }
    }
}

/// Whether serve is fronting Divixi's `port`.
pub async fn serve_state(port: u16) -> ServeState {
    let unknown = |detail: String| ServeState { published: None, port_free: None, detail };
    let out = match run(&["serve", "status", "--json"], READ_TIMEOUT).await {
        Ran::NoCli => return unknown("The tailscale CLI was not found in a standard install location.".into()),
        Ran::NoStart(e) => return unknown(format!("The tailscale CLI could not be started: {e}")),
        Ran::Timeout { out, err } => {
            let mut detail = "The tailscale CLI did not answer in time.".to_string();
            let words = said(&out, &err);
            if !words.is_empty() {
                detail.push_str(&format!(" Before the deadline it printed: {words}"));
            }
            return unknown(detail);
        }
        Ran::Done { status: 0, out, .. } => out,
        Ran::Done { status, out, err } => {
            let words = said(&out, &err);
            return unknown(if words.is_empty() { format!("tailscale serve status exited {status}") } else { words });
        }
    };
    if out.trim().is_empty() {
        return ServeState {
            published: Some(false),
            port_free: Some(true),
            detail: "No serve configuration is active.".into(),
        };
    }
    match serde_json::from_str::<Value>(&out) {
        Ok(doc) => classify_doc(&doc, port),
        // The daemon *did* answer; we cannot read its shape. That is not the
        // same as no answer, and the write guards need the difference.
        Err(_) => unknown("tailscale serve status returned output this build cannot read.".into()),
    }
}

/// Add what the code means, beside the daemon's words and never instead.
///
/// Says what happened and what is missing. It does not hand over a command:
/// the button is the way this is done, and a card that prints a command line
/// has given up and called it help.
fn with_hint(code: Code, words: String, fallback: String, nothing_happened: &str) -> Outcome {
    let hint = match code {
        // The likeliest refusal, and the one nobody can guess from the message:
        // serve configuration is daemon state, so on Linux and macOS it needs a
        // standing grant this app cannot give itself.
        Code::NoPermission => format!(" {nothing_happened} Divixi is not allowed to change Tailscale's serve configuration on this machine."),
        Code::DaemonUnavailable => format!(" {nothing_happened} Tailscale is not answering — it may be stopped or signed out."),
        _ => String::new(),
    };
    let detail = if words.is_empty() { fallback } else { words };
    Outcome { ok: false, code, detail: format!("{detail}{hint}") }
}

/// Put Divixi on this machine's tailnet over HTTPS.
///
/// The proxy target is loopback on purpose: serve runs on this same host, so
/// nothing has to listen on another interface and Divixi's bind is left exactly
/// as it was. Publishing hands a TLS terminator a local address; it does not
/// widen the socket.
pub async fn publish(port: u16) -> Outcome {
    // Say "not installed" before the occupancy guard, or a missing binary gets
    // reported as "could not confirm the mount is free".
    if cli_path().is_none() {
        return Outcome::bad(
            Code::NoCli,
            "Tailscale was not found in a standard install location, so nothing was published.",
        );
    }
    // Proceed only when our port is provably free or the mount is already ours.
    // Anything else refuses, including a state we could not determine, because
    // the costs are not symmetric: `serve --bg --https=443 <target>` REPLACES
    // whatever is at the mount, so overwriting destroys configuration somebody
    // rebuilds from memory, while refusing costs one pasted command -- which
    // the refusal prints.
    //
    // Keyed on `port_free`, not on "is anything configured": serve config that
    // sits entirely on other ports belongs to something else on this machine
    // and is untouched by this write, so it must not block it.
    let state = serve_state(port).await;
    // Already ours and already right: that is the goal, not an obstacle. Adopt
    // it and say so rather than writing the same configuration over itself.
    if state.published == Some(true) {
        return Outcome {
            ok: true,
            code: Code::Ok,
            detail: format!("Divixi was already published on this machine's tailnet ({SERVE_PORT} → 127.0.0.1:{port})."),
        };
    }
    if state.port_free != Some(true) {
        return Outcome::bad(
            Code::NotOurs,
            format!(
                "{} Divixi will not publish over it, because `tailscale serve` replaces whatever is at {SERVE_PORT}{SERVE_MOUNT} and this check could not confirm it is free.",
                state.detail,
            ),
        );
    }
    match run(&["serve", "--bg", &format!("--https={SERVE_PORT}"), &format!("http://127.0.0.1:{port}")], WRITE_TIMEOUT).await {
        Ran::NoCli => Outcome::bad(Code::NoCli, "Tailscale was not found, so nothing was published."),
        Ran::NoStart(e) => Outcome::bad(Code::Failed, format!("Tailscale is installed but could not be started: {e}")),
        Ran::Timeout { out, err } => {
            // The usual way to land here is a tailnet where Serve has not been
            // enabled: the CLI prints the enablement URL and then blocks
            // waiting for it, so the captured output carries the one thing
            // needed and a bare "timed out" would hide it.
            let mut detail = format!(
                "Tailscale did not answer within {}s. It may still have applied — press Check again before retrying.",
                WRITE_TIMEOUT.as_secs()
            );
            let words = said(&out, &err);
            if !words.is_empty() {
                detail.push_str(&format!(" Before the deadline it printed: {words}"));
            }
            Outcome::bad(Code::Timeout, detail)
        }
        Ran::Done { status: 0, .. } => Outcome {
            ok: true,
            code: Code::Ok,
            detail: format!("Divixi is published on this machine's tailnet over HTTPS ({SERVE_PORT} → 127.0.0.1:{port})."),
        },
        Ran::Done { status, out, err } => {
            let lead = if err.trim().is_empty() { out.clone() } else { err.clone() };
            with_hint(classify(&lead), said(&out, &err), format!("Tailscale refused (exit {status})."), "Nothing was published.")
        }
    }
}

/// Take Divixi off the tailnet — only if the mount is ours.
///
/// An undetermined state refuses too, and that is the deliberate half: this
/// build has seen almost none of the real serve-status shapes, so "I could not
/// tell" must not become "go ahead".
pub async fn unpublish(port: u16) -> Outcome {
    let state = serve_state(port).await;
    if state.published == Some(false) && state.port_free == Some(true) {
        // Nothing of ours is published. Reported as success because the goal
        // already holds, and running the removal anyway would be a write
        // against configuration belonging to something else.
        return Outcome {
            ok: true,
            code: Code::Ok,
            detail: format!("Nothing of Divixi's is published on port {SERVE_PORT}; anything else serve holds is left alone."),
        };
    }
    if state.published != Some(true) {
        return Outcome::bad(
            Code::NotOurs,
            format!("{} Divixi will not withdraw it, because this check could not confirm {SERVE_PORT}{SERVE_MOUNT} is Divixi's.", state.detail),
        );
    }
    match run(&["serve", "--https", &SERVE_PORT.to_string(), &format!("--set-path={SERVE_MOUNT}"), "off"], WRITE_TIMEOUT).await {
        Ran::NoCli => Outcome::bad(Code::NoCli, "Tailscale was not found; nothing to do."),
        Ran::NoStart(e) => Outcome::bad(Code::Failed, format!("The tailscale CLI could not be started: {e}")),
        Ran::Timeout { out, err } => {
            let mut detail = format!("Tailscale did not answer within {}s.", WRITE_TIMEOUT.as_secs());
            let words = said(&out, &err);
            if !words.is_empty() {
                detail.push_str(&format!(" Before the deadline it printed: {words}"));
            }
            Outcome::bad(Code::Timeout, detail)
        }
        Ran::Done { status: 0, .. } => Outcome {
            ok: true,
            code: Code::Ok,
            detail: "Divixi is no longer published on this machine's tailnet.".into(),
        },
        Ran::Done { status, out, err } => {
            // The hint matters more here than on the publish side: a failed
            // withdrawal's verbatim output can read like a status line
            // ("Tailscale is stopped.") rather than like a failure, and without
            // it nobody can tell that nothing was withdrawn.
            let lead = if err.trim().is_empty() { out.clone() } else { err.clone() };
            with_hint(classify(&lead), said(&out, &err), format!("Tailscale refused (exit {status})."), "Nothing was withdrawn.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape a real daemon returns, from `tailscale status --json` on
    /// Windows Tailscale 1.102.3. Trimmed to the fields read here.
    fn status_doc() -> Value {
        json!({
            "BackendState": "Running",
            "MagicDNSSuffix": "tailb45a71.ts.net",
            "CurrentTailnet": { "Name": "may-i.io", "MagicDNSSuffix": "tailb45a71.ts.net", "MagicDNSEnabled": true },
            "Self": { "DNSName": "laptop-mc28nnvi.tailb45a71.ts.net." },
            "CertDomains": ["laptop-mc28nnvi.tailb45a71.ts.net"],
            "Peer": { "a": { "Online": true }, "b": { "Online": false } }
        })
    }

    #[test]
    fn a_name_must_sit_under_the_tailnets_own_suffix() {
        let s = "tailb45a71.ts.net";
        assert_eq!(valid_name("laptop-mc28nnvi.tailb45a71.ts.net.", s).as_deref(), Some("laptop-mc28nnvi.tailb45a71.ts.net"));
        // The suffix comes from the document, so another tailnet's works too.
        assert!(valid_name("desk.userfoo.tailscale.net", "userfoo.tailscale.net").is_some());

        // Merely containing the suffix is not being under it.
        assert_eq!(valid_name("laptop.tailb45a71.ts.net.evil.com", s), None);
        assert_eq!(valid_name("tailb45a71.ts.net", s), None, "the suffix itself is not a host");
        // Anything that is not a bare lowercase hostname.
        for bad in ["https://laptop.tailb45a71.ts.net", "laptop.tailb45a71.ts.net:443", "LAPTOP.tailb45a71.ts.net", "laptop.tailb45a71.ts.net/x", "a@laptop.tailb45a71.ts.net", "lap top.tailb45a71.ts.net", "-x.tailb45a71.ts.net", ""] {
            assert_eq!(valid_name(bad, s), None, "{bad:?} must not pass");
        }
        assert_eq!(valid_name("laptop.tailb45a71.ts.net", ""), None, "no suffix, no name");
    }

    #[test]
    fn a_running_signed_in_daemon_reads_out_whole() {
        // The parse of a real document, without running anything.
        let d = status_doc();
        let suffix = d["CurrentTailnet"]["MagicDNSSuffix"].as_str().unwrap();
        assert_eq!(valid_name(d["Self"]["DNSName"].as_str().unwrap(), suffix).unwrap(), "laptop-mc28nnvi.tailb45a71.ts.net");
        let peer = d["Peer"].as_object().unwrap();
        assert_eq!(peer.len(), 2);
        assert_eq!(peer.values().filter(|v| v.get("Online") == Some(&Value::Bool(true))).count(), 1);
    }

    #[test]
    fn a_key_names_a_port_the_two_ways_serve_documents_do() {
        assert_eq!(key_port("443"), Some(443));
        assert_eq!(key_port("desk.tail.ts.net:443"), Some(443));
        assert_eq!(key_port("*:443"), Some(443));
        assert_eq!(key_port("80"), Some(80));
        assert_eq!(key_port("Web"), None);
        assert_eq!(key_port(""), None);
        assert_eq!(key_port("0"), None);
        assert_eq!(key_port("123456"), None, "not a port");
    }

    /// The real thing, verbatim, from `tailscale serve status --json` on this
    /// machine (Windows Tailscale 1.102.3) while Divixi was published. Pinned
    /// as a string rather than a `json!` so a change to the shape has to be a
    /// change to this text: the previous tests built their own document and a
    /// helper that re-implemented the classification, so they agreed with each
    /// other while the code disagreed with the daemon.
    const OURS: &str = r#"{
      "TCP": { "443": { "HTTPS": true } },
      "Web": {
        "laptop-mc28nnvi.tailb45a71.ts.net:443": {
          "Handlers": { "/": { "Proxy": "http://127.0.0.1:7488" } }
        }
      }
    }"#;

    fn state_of(doc: &str, port: u16) -> (Option<bool>, Option<bool>) {
        let v: Value = serde_json::from_str(doc).expect("fixture parses");
        let st = classify_doc(&v, port);
        (st.published, st.port_free)
    }

    #[test]
    fn the_document_this_machine_really_returns_reads_as_ours() {
        assert_eq!(state_of(OURS, 7488), (Some(true), Some(false)), "{OURS}");
    }

    #[test]
    fn a_config_with_nothing_in_it_leaves_our_port_free() {
        // The bug this pins. Go marshals a `ServeConfig` with nil maps, so an
        // unconfigured daemon answers with keys and nulls rather than `{}` --
        // and reading that as "could not tell" refused the very first publish
        // on a machine that had no serve configuration at all, which is every
        // machine the first time.
        for doc in [r#"{}"#, r#"null"#, r#"[]"#, r#"{"TCP":null,"Web":null}"#, r#"{"TCP":null,"Web":null,"AllowFunnel":null}"#, r#"{"TCP":{},"Web":{}}"#] {
            assert_eq!(state_of(doc, 7488), (Some(false), Some(true)), "{doc} is nothing served");
        }
    }

    #[test]
    fn someone_elses_handler_at_our_mount_is_not_ours() {
        let doc = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"desk.tail.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(state_of(doc, 7488), (Some(false), Some(false)), "refuses: publishing would replace it");
    }

    #[test]
    fn ours_under_another_mount_does_not_make_the_mount_ours() {
        // Ours at /divixi, a stranger's at / -- the mount a withdrawal removes.
        let doc = r#"{"Web":{"desk.tail.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"},"/divixi":{"Proxy":"http://127.0.0.1:7488"}}}}}"#;
        assert_eq!(state_of(doc, 7488).0, Some(false));
    }

    #[test]
    fn serve_on_another_port_only_leaves_ours_free() {
        // A real document from a Windows 1.x daemon holding one port-80
        // mapping: another project on this machine must not read as "something
        // is on 443".
        let doc = r#"{"TCP":{"80":{"HTTP":true}},"Web":{"desk.tail.ts.net:80":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:9980"}}}}}"#;
        assert_eq!(state_of(doc, 7488), (Some(false), Some(true)), "free: this write endangers nothing");
    }

    #[test]
    fn a_document_with_no_port_keys_stays_unknown() {
        // Content, but no evidence of a port-keyed schema, so "our port is
        // free" is not a determination this build may make.
        let doc = r#"{"Something":{"Else":"http://127.0.0.1:9999"}}"#;
        assert_eq!(state_of(doc, 7488), (None, None));
    }

    #[test]
    fn a_port_443_mapping_we_cannot_read_is_unknown_not_free() {
        let doc = r#"{"Web":{"desk.tail.ts.net:443":{"Something":"unreadable"}}}"#;
        assert_eq!(state_of(doc, 7488), (None, Some(false)));
    }

    #[test]
    fn the_port_we_look_for_is_divixis_own() {
        // The same document is ours for 7488 and a stranger's for anything
        // else, so the needle really is the port and not just "something
        // loopback-shaped".
        assert_eq!(state_of(OURS, 7488).0, Some(true));
        assert_eq!(state_of(OURS, 9999).0, Some(false));
    }

    #[test]
    fn classify_separates_the_two_remedies() {
        assert_eq!(classify("Access denied: you must run as root"), Code::NoPermission);
        assert_eq!(classify("permission denied"), Code::NoPermission);
        assert_eq!(classify("use --operator to grant"), Code::NoPermission);
        assert_eq!(classify("Tailscale is stopped."), Code::DaemonUnavailable);
        assert_eq!(classify("failed to connect to local tailscaled; is it not running?"), Code::DaemonUnavailable);
        assert_eq!(classify("something nobody has seen"), Code::Failed);
    }

    #[test]
    fn the_daemons_words_lead_with_stderr_but_keep_stdout() {
        // The enablement URL goes to stdout and then the command blocks, so
        // building this from stderr alone drops the one useful thing.
        assert_eq!(said("https://login.tailscale.com/f/serve", ""), "https://login.tailscale.com/f/serve");
        assert_eq!(said("out", "err"), "err\nout");
        assert_eq!(said("  ", " \n"), "");
        let long = "x".repeat(DETAIL_MAX + 50);
        assert!(said(&long, "").ends_with(" …"));
        assert!(said(&long, "").chars().count() <= DETAIL_MAX + 2);
    }

    #[test]
    fn nothing_we_say_hands_the_human_a_command() {
        // Kiro Crew does not make the user type commands and neither do we:
        // the button acts, or the card says what is missing. A command line in
        // a card is homework.
        let doc: Value = serde_json::from_str(r#"{"Web":{"d:443":{"Something":"x"}}}"#).unwrap();
        let said = classify_doc(&doc, 7488).detail;
        assert!(!said.contains("tailscale "), "{said}");
        let refused = with_hint(Code::NoPermission, "access denied".into(), String::new(), "Nothing was published.");
        assert!(!refused.detail.contains("sudo"), "{}", refused.detail);
        assert!(!refused.detail.contains("--operator"), "{}", refused.detail);
    }
}
