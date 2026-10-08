//! `divixi-server phone status|on|off|link`: phone access, decided at the
//! server itself.
//!
//! A divixi-server has no window, and phone access is closed to every other
//! device ([`super::bridge`]): who may reach a machine is decided at that
//! machine. Whoever can run this on the server already owns it -- the rule
//! `divixi-server token` follows -- so the decision is still made there.
//!
//! Each command does what the desktop app's phone-access card does, through
//! the same functions: [`super::keep_reading`] for the step, the address and
//! the settings the server reads on every request, [`tailscale::publish`] and
//! [`tailscale::unpublish`] for the write, [`super::mint_phone_link`] for the
//! link. Two things differ, because this runs beside the server rather than
//! inside it:
//!
//! * The server is another process. Nothing here starts or stops it; it is
//!   asked whether anything answers on its port, and that is said when nothing
//!   does. The settings written here go to the same store, which the server
//!   reads on every request, so they apply without a restart.
//! * The card never hands over a command line. A terminal is where one
//!   belongs: when `tailscale serve` needs root, the exact command is printed.

use std::future::Future;
use std::path::PathBuf;

use orchestra_store::Store;

use super::tailscale::{self, Code, Outcome, Probe, ServeState};
use super::{auth, Step};

/// What `divixi-server phone` says when it is given nothing it knows.
const SAY: &str = "say status, on, off or link";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Status,
    /// `dry_run`: read, and say what would be done, changing nothing.
    On { dry_run: bool },
    Off { dry_run: bool },
    /// `qr`: draw the link as a QR code under it as well.
    Link { qr: bool },
}

/// The words after `phone`.
pub fn parse(args: &[&str]) -> anyhow::Result<Cmd> {
    let Some((what, flags)) = args.split_first() else { anyhow::bail!(SAY) };
    let takes: &[&str] = match *what {
        "status" => &[],
        "on" | "off" => &["--dry-run"],
        "link" => &["--no-qr"],
        other => anyhow::bail!("unknown phone command {other:?}: {SAY}"),
    };
    if let Some(stray) = flags.iter().find(|f| !takes.contains(f)) {
        anyhow::bail!("phone {what} does not take {stray:?}");
    }
    let has = |f: &str| flags.contains(&f);
    Ok(match *what {
        "status" => Cmd::Status,
        "on" => Cmd::On { dry_run: has("--dry-run") },
        "off" => Cmd::Off { dry_run: has("--dry-run") },
        _ => Cmd::Link { qr: !has("--no-qr") },
    })
}

/// Tailscale, and the server's port, as these commands see them. A trait so
/// the tests can stand in for a daemon and a server.
pub trait Tailnet {
    fn look(&self, port: u16) -> impl Future<Output = (Probe, ServeState)>;
    /// Whether anything answers on `127.0.0.1:port`: the server, running.
    fn answering(&self, port: u16) -> impl Future<Output = bool>;
    fn publish(&self, port: u16) -> impl Future<Output = Outcome>;
    fn unpublish(&self, port: u16) -> impl Future<Output = Outcome>;
    /// The CLI, for a command printed for a human to run.
    fn cli(&self) -> Option<PathBuf>;
}

/// This machine's Tailscale.
pub struct Real;

impl Tailnet for Real {
    async fn look(&self, port: u16) -> (Probe, ServeState) {
        super::read_tailnet(port).await
    }
    async fn answering(&self, port: u16) -> bool {
        tailscale::listening(port).await == Some(true)
    }
    async fn publish(&self, port: u16) -> Outcome {
        tailscale::publish(port).await
    }
    async fn unpublish(&self, port: u16) -> Outcome {
        tailscale::unpublish(port).await
    }
    fn cli(&self) -> Option<PathBuf> {
        tailscale::cli_path()
    }
}

/// One reading, as the card would render it.
struct Reading {
    port: u16,
    running: bool,
    step: Step,
    address: String,
    probe: Probe,
    serve: ServeState,
}

async fn read(store: &Store, net: &impl Tailnet) -> Reading {
    let port = super::port_in(store);
    let (probe, serve) = net.look(port).await;
    let running = net.answering(port).await;
    let (step, address) = super::keep_reading(store, &probe, &serve, running);
    Reading { port, running, step, address, probe, serve }
}

/// The daemon's own words, after ours, when it said any.
fn with_words(ours: &str, words: &str) -> String {
    let words = words.trim();
    if words.is_empty() { ours.to_string() } else { format!("{ours} ({words})") }
}

/// Where phone access stands, in one sentence.
fn standing(r: &Reading) -> String {
    match r.step {
        Step::Install => "Tailscale is not installed: no tailscale CLI in a standard install location.".into(),
        Step::StartTailscale => with_words("Tailscale is not running, or is stopped.", &r.probe.detail),
        Step::SignIn => "This machine is not signed in to Tailscale.".into(),
        Step::EnableMagicDns => "This machine has no MagicDNS name: MagicDNS is off for the tailnet.".into(),
        Step::EnableHttps => "The tailnet does not issue HTTPS certificates: HTTPS is off for the tailnet.".into(),
        Step::Occupied => with_words(&format!("Tailscale serve holds port {} for something else.", r.serve.https_port()), &r.serve.detail),
        Step::Publish if r.serve.published == Some(true) => {
            format!("Published at {}, but nothing answers on 127.0.0.1:{}.", r.address, r.port)
        }
        Step::Publish => "Not published.".into(),
        Step::Ready => format!("Published at {}.", r.address),
    }
}

/// The one next thing to do, as a line to print.
fn next(r: &Reading) -> String {
    match r.step {
        Step::Install => "install Tailscale (https://tailscale.com/download) and sign in".into(),
        Step::StartTailscale => "start Tailscale (on Linux: sudo systemctl start tailscaled, then sudo tailscale up)".into(),
        Step::SignIn => "sign in: sudo tailscale up".into(),
        Step::EnableMagicDns | Step::EnableHttps => {
            "turn MagicDNS and HTTPS certificates on in the tailnet's DNS settings (https://login.tailscale.com/admin/dns)".into()
        }
        Step::Occupied => format!("free port {} in tailscale serve, or leave phone access off", r.serve.https_port()),
        Step::Publish if r.serve.published == Some(true) => "start divixi-server".into(),
        Step::Publish if !r.running => "start divixi-server, and turn it on: divixi-server phone on".into(),
        Step::Publish => "divixi-server phone on".into(),
        Step::Ready => "make a pairing link for a phone: divixi-server phone link".into(),
    }
}

/// Whether the step leaves nothing for `tailscale serve` to do yet.
fn blocked(step: Step) -> bool {
    matches!(step, Step::Install | Step::StartTailscale | Step::SignIn | Step::EnableMagicDns | Step::EnableHttps)
}

/// A command line for a human, from the CLI that was found (or its plain
/// name), with the path quoted if it needs it.
fn command_line(net: &impl Tailnet, args: &[String]) -> String {
    let cli = net.cli().map(|p| p.display().to_string()).unwrap_or_else(|| "tailscale".into());
    let cli = if cli.contains(' ') { format!("\"{cli}\"") } else { cli };
    format!("{cli} {}", args.join(" "))
}

/// The daemon's refusal, and -- when it was about permission -- the command
/// that does the same as root. The card says what is missing; a terminal
/// can say what to type.
fn refused(net: &impl Tailnet, out: &Outcome, args: &[String], again: &str) -> anyhow::Error {
    if out.code != Code::NoPermission {
        return anyhow::anyhow!("{}", out.detail);
    }
    let line = command_line(net, args);
    let root = if cfg!(windows) { format!("From an administrator prompt:\n  {line}") } else { format!("  sudo {line}") };
    let operator = if cfg!(windows) {
        String::new()
    } else {
        format!("\nOr let this user change serve from now on, and run `{again}` again:\n  sudo {} set --operator=$USER", command_line(net, &[]).trim_end())
    };
    anyhow::anyhow!("{}\n\nRun this as root, then `{again}` again:\n{root}{operator}", out.detail)
}

/// Run one `divixi-server phone` command against this store and this tailnet.
/// What it prints on success; an error is printed and exits non-zero.
pub async fn run(cmd: Cmd, store: &Store, auth: &auth::Auth, net: &impl Tailnet) -> anyhow::Result<String> {
    match cmd {
        Cmd::Status => status(store, net).await,
        Cmd::On { dry_run } => on(store, net, dry_run).await,
        Cmd::Off { dry_run } => off(store, net, dry_run).await,
        Cmd::Link { qr } => link(store, auth, net, qr).await,
    }
}

async fn status(store: &Store, net: &impl Tailnet) -> anyhow::Result<String> {
    let r = read(store, net).await;
    let on = if super::phone_on_in(store) { "on" } else { "off" };
    let server = if r.running { "answering" } else { "not answering" };
    let address = if r.address.is_empty() { "-" } else { &r.address };
    let days = super::phone_days_in(store);
    Ok([
        format!("phone access: {on}"),
        format!("tailscale:    {}", standing(&r)),
        format!("address:      {address}"),
        format!("server:       {server} on 127.0.0.1:{}", r.port),
        format!("links last:   {days} day{}", if days == 1 { "" } else { "s" }),
        format!("next:         {}", next(&r)),
    ]
    .join("\n"))
}

async fn on(store: &Store, net: &impl Tailnet, dry_run: bool) -> anyhow::Result<String> {
    let r = read(store, net).await;
    if blocked(r.step) {
        anyhow::bail!("{} Nothing was published.\nNext: {}", standing(&r), next(&r));
    }
    let args = tailscale::publish_args(r.port, r.serve.https_port());
    if dry_run {
        let would = match (r.serve.published, r.step) {
            (Some(true), _) => format!("Already published at {}: would only keep phone access on.", r.address),
            (_, Step::Occupied) => format!("Would refuse: {}", standing(&r)),
            _ => format!("Would run: {}", command_line(net, &args)),
        };
        return Ok(format!("{would}\nNothing was changed (--dry-run)."));
    }
    let was = super::phone_on_in(store) && r.step == Step::Ready;
    // The intent first and back out on failure, as the card does: a setting
    // left reading "on" over an address nobody can reach is the
    // working-looking control the card exists to avoid.
    super::set_in(store, "phone.enabled", "true").map_err(anyhow::Error::msg)?;
    let out = net.publish(r.port).await;
    if !out.ok {
        let _ = super::set_in(store, "phone.enabled", "false");
        read(store, net).await;
        return Err(refused(net, &out, &args, "divixi-server phone on"));
    }
    let now = read(store, net).await;
    let lead = if was { "Phone access was already on" } else { "Phone access is on" };
    let mut said = format!("{lead}: {}\n{}", now.address, out.detail);
    if !now.running {
        said.push_str(&format!(
            "\nNothing answers on 127.0.0.1:{} yet: start divixi-server, then make a link with `divixi-server phone link`.",
            now.port
        ));
    } else {
        said.push_str("\nMake a pairing link for a phone: divixi-server phone link");
    }
    Ok(said)
}

async fn off(store: &Store, net: &impl Tailnet, dry_run: bool) -> anyhow::Result<String> {
    let r = read(store, net).await;
    let args = tailscale::unpublish_args(r.serve.https_port());
    if dry_run {
        let would = match r.serve.published {
            Some(true) => format!("Would run: {}", command_line(net, &args)),
            _ => "Nothing of DIVIXI's is published: would only turn phone access off.".to_string(),
        };
        return Ok(format!("{would}\nNothing was changed (--dry-run)."));
    }
    let was_off = !super::phone_on_in(store) && r.serve.published != Some(true);
    // Off here either way -- the setting must not stick on because a daemon
    // would not answer -- as the card does.
    super::set_in(store, "phone.enabled", "false").map_err(anyhow::Error::msg)?;
    let out = net.unpublish(r.port).await;
    // Read again, so the origin the server allows follows what is published.
    read(store, net).await;
    if !out.ok {
        return Err(refused(net, &out, &args, "divixi-server phone off"));
    }
    Ok(if was_off { "Phone access was already off.".into() } else { format!("Phone access is off.\n{}", out.detail) })
}

async fn link(store: &Store, auth: &auth::Auth, net: &impl Tailnet, qr: bool) -> anyhow::Result<String> {
    let r = read(store, net).await;
    if r.step != Step::Ready {
        // A link for an address that answers nothing fails on the phone,
        // where there is nothing to read: the card refuses the same way.
        if r.serve.published == Some(true) && !r.running {
            anyhow::bail!("Nothing answers on 127.0.0.1:{}, so a link would not open anything. Start divixi-server first.", r.port);
        }
        anyhow::bail!("Phone access is not on, so a link would not open anything. {}\nNext: {}", standing(&r), next(&r));
    }
    let link = super::mint_phone_link(store, auth, &r.address);
    let mut said = link.url.clone();
    if qr {
        if let Some(code) = qr_code(&link.url) {
            said.push_str(&format!("\n\n{code}"));
        }
    }
    let days = if link.days == 1 { "1 day".to_string() } else { format!("{} days", link.days) };
    said.push_str(&format!(
        "\n\nOpen it on the phone within {} minutes. It works once, and signs the phone in for {days}.\n\
         If the phone says pairing failed, its browser may have opened the link early in a hidden tab \
         (iOS Chrome's \"Preload webpages\"): turn that off, or scan the code with the camera, and make a new link.",
        auth::PAIR_SECS / 60,
    ));
    Ok(said)
}

/// `text` as a QR code for a terminal, two rows to a line. Drawn for a dark
/// background (light modules are the filled ones), with the quiet zone a
/// camera needs around it.
fn qr_code(text: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M).ok()?;
    Some(code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).quiet_zone(true).build())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    const NAME: &str = "office-server.example.ts.net";

    fn ready_probe() -> Probe {
        Probe {
            installed: true,
            reachable: true,
            logged_in: true,
            name: NAME.into(),
            https: Some(true),
            peers: 1,
            peers_online: 1,
            login: "someone@example.com".into(),
            ..Default::default()
        }
    }

    pub(crate) fn free() -> ServeState {
        ServeState { published: Some(false), port_free: Some(true), https: tailscale::SERVE_PORT, ..Default::default() }
    }

    pub(crate) fn published() -> ServeState {
        ServeState { published: Some(true), port_free: Some(false), https: tailscale::SERVE_PORT, ..Default::default() }
    }

    fn done(detail: &str) -> Outcome {
        Outcome { ok: true, code: Code::Ok, detail: detail.into() }
    }

    /// A daemon and a server, as these commands see them. A successful
    /// publish or unpublish changes what the next look reads, as it would.
    pub(crate) struct Fake {
        probe: Probe,
        serve: RefCell<ServeState>,
        pub(crate) running: bool,
        publish: Outcome,
        unpublish: Outcome,
        published: Cell<usize>,
        unpublished: Cell<usize>,
    }

    impl Fake {
        pub(crate) fn new(serve: ServeState) -> Self {
            Self {
                probe: ready_probe(),
                serve: RefCell::new(serve),
                running: true,
                publish: done("DIVIXI is published on this machine's tailnet over HTTPS (443 → 127.0.0.1:7489)."),
                unpublish: done("DIVIXI is no longer published on this machine's tailnet."),
                published: Cell::new(0),
                unpublished: Cell::new(0),
            }
        }
    }

    impl Tailnet for Fake {
        async fn look(&self, _port: u16) -> (Probe, ServeState) {
            (self.probe.clone(), self.serve.borrow().clone())
        }
        async fn answering(&self, _port: u16) -> bool {
            self.running
        }
        async fn publish(&self, _port: u16) -> Outcome {
            self.published.set(self.published.get() + 1);
            if self.publish.ok {
                *self.serve.borrow_mut() = published();
            }
            self.publish.clone()
        }
        async fn unpublish(&self, _port: u16) -> Outcome {
            self.unpublished.set(self.unpublished.get() + 1);
            if self.unpublish.ok {
                *self.serve.borrow_mut() = free();
            }
            self.unpublish.clone()
        }
        fn cli(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/usr/bin/tailscale"))
        }
    }

    fn store() -> Store {
        Store::in_memory().unwrap()
    }

    fn get(store: &Store, key: &str) -> Option<String> {
        super::super::setting_in(store, key)
    }

    fn go(cmd: Cmd, store: &Store, net: &Fake) -> anyhow::Result<String> {
        let auth = auth::Auth::with_key([7; 32]);
        tauri::async_runtime::block_on(run(cmd, store, &auth, net))
    }

    fn port() -> u16 {
        super::super::LISTEN_PORT
    }

    #[test]
    fn the_words_after_phone_parse_to_one_command() {
        assert_eq!(parse(&["status"]).unwrap(), Cmd::Status);
        assert_eq!(parse(&["on"]).unwrap(), Cmd::On { dry_run: false });
        assert_eq!(parse(&["on", "--dry-run"]).unwrap(), Cmd::On { dry_run: true });
        assert_eq!(parse(&["off"]).unwrap(), Cmd::Off { dry_run: false });
        assert_eq!(parse(&["off", "--dry-run"]).unwrap(), Cmd::Off { dry_run: true });
        assert_eq!(parse(&["link"]).unwrap(), Cmd::Link { qr: true });
        assert_eq!(parse(&["link", "--no-qr"]).unwrap(), Cmd::Link { qr: false });
        assert_eq!(parse(&[]).unwrap_err().to_string(), SAY);
        assert!(parse(&["up"]).unwrap_err().to_string().contains("unknown phone command \"up\""));
        assert!(parse(&["status", "--dry-run"]).unwrap_err().to_string().contains("does not take"));
        assert!(parse(&["link", "--dry-run"]).is_err());
    }

    #[test]
    fn status_says_where_phone_access_stands_and_keeps_the_origin() {
        let s = store();
        let said = go(Cmd::Status, &s, &Fake::new(published())).unwrap();
        assert!(said.contains("phone access: off"), "{said}");
        assert!(said.contains(&format!("Published at https://{NAME}.")), "{said}");
        assert!(said.contains(&format!("answering on 127.0.0.1:{}", port())), "{said}");
        assert!(said.contains("links last:   7 days"), "{said}");
        assert!(said.contains("divixi-server phone link"), "{said}");
        // What the server reads on every request, written as the card writes it.
        assert_eq!(get(&s, "phone.origin").as_deref(), Some(format!("https://{NAME}").as_str()));
        assert_eq!(get(&s, "phone.self_login").as_deref(), Some("someone@example.com"));

        // The same mapping with no server behind it is not ready, and the
        // origin goes with it.
        let mut stopped = Fake::new(published());
        stopped.running = false;
        let said = go(Cmd::Status, &s, &stopped).unwrap();
        assert!(said.contains("nothing answers on 127.0.0.1"), "{said}");
        assert!(said.contains("next:         start divixi-server"), "{said}");
        assert_eq!(get(&s, "phone.origin").as_deref(), Some(""));
    }

    #[test]
    fn status_names_the_errand_when_tailscale_is_not_there() {
        let mut net = Fake::new(ServeState::default());
        net.probe = Probe::default();
        let said = go(Cmd::Status, &store(), &net).unwrap();
        assert!(said.contains("Tailscale is not installed"), "{said}");
        assert!(said.contains("address:      -"), "{said}");
    }

    #[test]
    fn on_publishes_and_keeps_phone_access_on() {
        let s = store();
        let net = Fake::new(free());
        let said = go(Cmd::On { dry_run: false }, &s, &net).unwrap();
        assert_eq!(net.published.get(), 1);
        assert!(said.starts_with(&format!("Phone access is on: https://{NAME}")), "{said}");
        assert_eq!(get(&s, "phone.enabled").as_deref(), Some("true"));
        assert_eq!(get(&s, "phone.origin").as_deref(), Some(format!("https://{NAME}").as_str()));

        let said = go(Cmd::On { dry_run: false }, &s, &net).unwrap();
        assert!(said.starts_with("Phone access was already on"), "{said}");
    }

    #[test]
    fn on_without_root_prints_the_command_and_stays_off() {
        let s = store();
        let mut net = Fake::new(free());
        net.publish = Outcome { ok: false, code: Code::NoPermission, detail: "Access denied: serve config denied".into() };
        let err = go(Cmd::On { dry_run: false }, &s, &net).unwrap_err().to_string();
        assert!(err.starts_with("Access denied: serve config denied"), "{err}");
        if !cfg!(windows) {
            let line = format!("sudo /usr/bin/tailscale serve --bg --https=443 http://127.0.0.1:{}", port());
            assert!(err.contains(&line), "{err}");
            assert!(err.contains("set --operator=$USER"), "{err}");
        }
        assert_eq!(get(&s, "phone.enabled").as_deref(), Some("false"));
        assert_eq!(get(&s, "phone.origin").as_deref(), Some(""));
    }

    #[test]
    fn other_refusals_pass_the_daemons_words_through_alone() {
        let mut net = Fake::new(free());
        net.publish = Outcome { ok: false, code: Code::Timeout, detail: "Tailscale did not answer within 15s.".into() };
        let err = go(Cmd::On { dry_run: false }, &store(), &net).unwrap_err().to_string();
        assert_eq!(err, "Tailscale did not answer within 15s.");
    }

    #[test]
    fn on_does_not_try_while_tailscale_cannot_serve() {
        let mut net = Fake::new(ServeState::default());
        net.probe.logged_in = false;
        let err = go(Cmd::On { dry_run: false }, &store(), &net).unwrap_err().to_string();
        assert!(err.contains("not signed in"), "{err}");
        assert_eq!(net.published.get(), 0);
    }

    #[test]
    fn a_dry_run_says_the_command_and_changes_nothing() {
        let s = store();
        let net = Fake::new(free());
        let said = go(Cmd::On { dry_run: true }, &s, &net).unwrap();
        assert!(said.contains(&format!("Would run: /usr/bin/tailscale serve --bg --https=443 http://127.0.0.1:{}", port())), "{said}");
        assert_eq!(net.published.get(), 0);
        assert_eq!(get(&s, "phone.enabled"), None);

        let net = Fake::new(published());
        let said = go(Cmd::Off { dry_run: true }, &s, &net).unwrap();
        assert!(said.contains("Would run: /usr/bin/tailscale serve --https 443 --set-path=/ off"), "{said}");
        assert_eq!(net.unpublished.get(), 0);
    }

    #[test]
    fn off_withdraws_and_closes_the_origin() {
        let s = store();
        let net = Fake::new(free());
        go(Cmd::On { dry_run: false }, &s, &net).unwrap();
        let said = go(Cmd::Off { dry_run: false }, &s, &net).unwrap();
        assert!(said.starts_with("Phone access is off."), "{said}");
        assert_eq!(net.unpublished.get(), 1);
        assert_eq!(get(&s, "phone.enabled").as_deref(), Some("false"));
        assert_eq!(get(&s, "phone.origin").as_deref(), Some(""));

        let said = go(Cmd::Off { dry_run: false }, &s, &net).unwrap();
        assert_eq!(said, "Phone access was already off.");
    }

    #[test]
    fn a_link_carries_the_phone_span() {
        let s = store();
        let auth = auth::Auth::with_key([7; 32]);
        for (setting, days) in [(None, auth::PHONE_DAYS_DEFAULT), (Some("30"), 30), (Some("1"), 1), (Some("12"), auth::PHONE_DAYS_DEFAULT)] {
            if let Some(d) = setting {
                super::super::set_in(&s, "phone.days", d).unwrap();
            }
            let said = tauri::async_runtime::block_on(run(Cmd::Link { qr: false }, &s, &auth, &Fake::new(published()))).unwrap();
            let url = said.lines().next().unwrap();
            let token = url.strip_prefix(&format!("https://{NAME}/?token=")).expect(url);
            let claims = auth.verify(token).unwrap();
            assert_eq!(claims.k, auth::Kind::Pair);
            assert_eq!(claims.s, auth::Scope::Conversation, "a phone's link, not the full one `token` prints");
            assert_eq!(claims.l, Some(days * 24 * 60 * 60), "{setting:?}");
            assert!(said.contains(&format!("signs the phone in for {days} day")), "{said}");
            assert!(said.contains("Preload webpages"), "{said}");
        }
    }

    #[test]
    fn a_link_is_drawn_as_a_code_when_asked() {
        let said = go(Cmd::Link { qr: true }, &store(), &Fake::new(published())).unwrap();
        assert!(said.contains('█') || said.contains('▀') || said.contains('▄'), "{said}");
        let plain = go(Cmd::Link { qr: false }, &store(), &Fake::new(published())).unwrap();
        assert!(!plain.contains('█'), "{plain}");
    }

    #[test]
    fn no_link_for_an_address_that_answers_nothing() {
        let err = go(Cmd::Link { qr: false }, &store(), &Fake::new(free())).unwrap_err().to_string();
        assert!(err.starts_with("Phone access is not on"), "{err}");
        assert!(err.contains("divixi-server phone on"), "{err}");

        let mut stopped = Fake::new(published());
        stopped.running = false;
        let err = go(Cmd::Link { qr: false }, &store(), &stopped).unwrap_err().to_string();
        assert!(err.contains("Start divixi-server first"), "{err}");
    }
}
