//! Who, on the tailnet, a forwarded request actually came from.
//!
//! Phone access puts `tailscale serve` in front of this server, and a proxy
//! on the same host connects from loopback — so `request.remote` is
//! `127.0.0.1` for every phone, and anything pinned to it is pinned to the
//! proxy. This module replaces that with an identity the local daemon
//! vouches for.
//!
//! The organizing rule, reimplemented from Kiro Crew's
//! `rfc-tailnet-dashboard-access.md` §1:
//!
//! > The immediate peer decides whether a forwarded header may be read at
//! > all. The local daemon, not the header, decides who the peer is. The
//! > header is only corroboration.
//!
//! Two consequences worth stating because they are easy to get backwards.
//! **A header is not a credential**: `Tailscale-User-Login` is checked for
//! agreement and never trusted on its own, so forging it cannot get anyone
//! in. And **failure degrades rather than denies**: every unresolvable case
//! is `None`, which falls back to what the request's token alone earns —
//! a stopped `tailscaled` must not lock the owner out of their own machine.
//!
//! The allowlist is **mandatory**. "Anyone on this tailnet" is not a private
//! network: the tailnet this was written on has 406 devices and 177 of them
//! online, all colleagues. Trust with an empty allowlist is a configuration
//! error, refused where it is read, and trust stays off.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use serde_json::Value;

use crate::AppHandle;

/// Only this module may read it, and only to corroborate.
const LOGIN_HEADER: &str = "tailscale-user-login";

/// The login `tailscale whois` reports for **every** ACL-tagged node.
///
/// Under login scope that one value would be the peer key for an entire
/// tagged fleet, so one leaked session would be replayable from any other
/// tagged node — and an allowlist entry of `tagged-devices` would admit all
/// of them. A resolved login equal to this is always pinned at node scope.
const TAGGED: &str = "tagged-devices";

/// How long a resolved address is reused. Peer identity is stable over
/// seconds, and without this a burst of requests forks a daemon call each.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// A ceiling on the cache, so a flood of distinct source addresses cannot
/// grow it without bound. Far above any real tailnet's online count.
const CACHE_MAX: usize = 512;

/// What a session is pinned to once a peer resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinScope {
    /// `ts:node:<login>|<node>` — a leaked session is usable only from the
    /// device it was made on. The default, because the flow it costs (one
    /// link opened on two devices) does not work under any pin.
    #[default]
    Node,
    /// `ts:login:<login>` — usable from any device carrying that identity.
    /// For whoever re-enrols devices often and would rather re-issue nothing.
    Login,
}

/// A peer the local daemon vouched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// The Tailscale login. An email address, which is why the key below
    /// joins with `|` — a character neither component may contain.
    pub login: String,
    /// `Node.StableID`: unique per device and unchanged by a rename.
    pub node: String,
    /// An ACL tag replaces the user identity on a device.
    pub tagged: bool,
}

impl Peer {
    /// What to pin a session to. A tagged node is **always** node scope,
    /// whatever is configured: see [`TAGGED`].
    pub fn key(&self, scope: PinScope) -> String {
        if self.tagged || scope == PinScope::Node {
            format!("ts:node:{}|{}", self.login, self.node)
        } else {
            format!("ts:login:{}", self.login)
        }
    }
}

/// Whether identity is trusted here, and for whom.
#[derive(Debug, Clone, Default)]
pub struct Trust {
    pub on: bool,
    /// Lower-cased logins that may come in. Never empty while `on`.
    pub logins: Vec<String>,
    pub pin: PinScope,
}

impl Trust {
    pub fn allows(&self, login: &str) -> bool {
        let login = login.trim().to_lowercase();
        self.logins.contains(&login)
    }
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    use tauri::Manager;
    app.state::<crate::AppState>().store.get_meta(&format!("{}{key}", crate::SETTING_PREFIX)).ok().flatten()
}

/// Read the trust configuration, refusing the one shape that would be
/// silently permissive.
///
/// **On unless turned off**, which is the opposite of how this started and
/// the right way round. Trust can only ever *refuse* somebody the token
/// would have let in — an unresolvable peer falls through to the token — so
/// defaulting it off meant the allowlist was written and never consulted,
/// and 405 colleagues on this tailnet were held out by nothing but the
/// secrecy of a QR code.
///
/// The allowlist defaults to the one login the daemon says owns this
/// machine, recorded by the last probe. That is the person at the keyboard,
/// and admitting exactly them is the only default that is not a guess.
/// `phone.allowed_logins` overrides it for whoever wants a second account.
///
/// Trust with an empty allowlist is a configuration error, not "let everyone
/// in", and is refused here where it is read — there is no later gate on
/// which an empty allowlist could admit anybody. Unknown own login and no
/// configured list is exactly that case, so trust stays off and the token is
/// the whole of the check, as it was before any of this.
pub fn trust(app: &AppHandle) -> Trust {
    let on = setting(app, "phone.trust_identity").as_deref() != Some("false");
    let configured = setting(app, "phone.allowed_logins").unwrap_or_default();
    let fallback = setting(app, "phone.self_login").unwrap_or_default();
    let from = if configured.trim().is_empty() { fallback } else { configured };
    let logins: Vec<String> = from
        .split(',')
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();
    if on && logins.is_empty() {
        tracing::debug!(
            "no tailnet login is known for this machine and none is configured, so identity trust stays off and a token is the whole of the check"
        );
        return Trust::default();
    }
    let pin = match setting(app, "phone.pin_scope").as_deref() {
        Some("login") => PinScope::Login,
        None | Some("node") => PinScope::Node,
        // A typo may only ever narrow, never widen.
        Some(other) => {
            tracing::warn!(%other, "unknown phone.pin_scope; pinning to the node");
            PinScope::Node
        }
    };
    Trust { on, logins, pin }
}

/// The ranges a tailnet peer can legitimately arrive from. Anything outside
/// these is not a tailnet address and is never sent to the daemon.
fn in_tailnet(addr: IpAddr) -> bool {
    match addr {
        // 100.64.0.0/10, the CGNAT range Tailscale assigns from.
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (64..128).contains(&o[1])
        }
        // fd7a:115c:a1e0::/48
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// The single forwarded address, or `None`.
///
/// Two or more means a proxy chain this cannot attribute. Taking the first or
/// the last is the classic way to get spoofed — whoever is furthest from us
/// supplies the value — so more than one is a refusal, not a choice.
fn single_forwarded(headers: &HeaderMap) -> Option<IpAddr> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    let mut parts = raw.split(',').map(str::trim).filter(|p| !p.is_empty());
    let only = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    only.parse().ok()
}

/// Whether this request arrived through a proxy at all.
pub fn forwarded(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
}

type Cached = HashMap<IpAddr, (Instant, Option<Peer>)>;

/// Resolutions kept briefly, by address.
#[derive(Default)]
pub struct Cache(parking_lot::Mutex<Cached>);

impl Cache {
    fn get(&self, addr: IpAddr) -> Option<Option<Peer>> {
        let mut map = self.0.lock();
        match map.get(&addr) {
            Some((at, peer)) if at.elapsed() < CACHE_TTL => Some(peer.clone()),
            Some(_) => {
                map.remove(&addr);
                None
            }
            None => None,
        }
    }

    fn put(&self, addr: IpAddr, peer: Option<Peer>) {
        let mut map = self.0.lock();
        if map.len() >= CACHE_MAX {
            // Drop what has aged out; if that frees nothing, drop the lot
            // rather than grow. A cache is an optimisation and losing it
            // costs a daemon call, not correctness.
            map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            if map.len() >= CACHE_MAX {
                map.clear();
            }
        }
        map.insert(addr, (Instant::now(), peer));
    }
}

/// Read a whois document into a [`Peer`].
fn read_whois(doc: &Value) -> Option<Peer> {
    let login = doc.get("UserProfile")?.get("LoginName")?.as_str()?.trim().to_lowercase();
    if login.is_empty() {
        return None;
    }
    let node = doc.get("Node")?;
    // `StableID` rather than the name: unique per device and unchanged by a
    // rename, which is what a pin wants.
    let id = node.get("StableID").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if id.is_empty() {
        return None;
    }
    let tagged = login == TAGGED
        || node.get("Tags").and_then(Value::as_array).is_some_and(|t| !t.is_empty());
    Some(Peer { login, node: id, tagged })
}

/// The peer behind a forwarded request, or `None`.
///
/// `None` unless **all** of these hold, and each is its own reason to refuse:
///
/// 1. the immediate peer is loopback — only the proxy on this host can reach
///    the socket, so only it could have set the forwarded header;
/// 2. identity trust is on (and therefore the allowlist is not empty);
/// 3. `X-Forwarded-For` carries exactly one address;
/// 4. that address is inside the tailnet ranges;
/// 5. the local daemon resolves it;
/// 6. the resolved login agrees with `Tailscale-User-Login` where that header
///    is present. A missing header costs nothing; a disagreeing one is a
///    rejection.
pub async fn resolve(app: &AppHandle, immediate: Option<IpAddr>, headers: &HeaderMap, trust: &Trust) -> Option<Peer> {
    if !immediate.is_some_and(|ip| ip.is_loopback()) || !trust.on {
        return None;
    }
    let addr = single_forwarded(headers)?;
    if !in_tailnet(addr) {
        return None;
    }
    use tauri::Manager;
    let cache = &app.state::<crate::AppState>().remote.peers;
    let peer = match cache.get(addr) {
        Some(hit) => hit,
        None => {
            let found = tailscale::whois(&addr.to_string()).await.as_ref().and_then(read_whois);
            cache.put(addr, found.clone());
            found
        }
    }?;
    // Corroboration only. The daemon already said who this is; a header that
    // disagrees means something between us and it is lying, so nobody comes in.
    if let Some(claimed) = headers.get(LOGIN_HEADER).and_then(|v| v.to_str().ok()) {
        let claimed = claimed.trim().to_lowercase();
        if !claimed.is_empty() && claimed != peer.login {
            tracing::warn!(claimed = %claimed, "a forwarded login header disagreed with the daemon; refusing the peer");
            return None;
        }
    }
    Some(peer)
}

use super::tailscale;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    /// The real document, from `tailscale whois --json` on this machine
    /// against a live peer of its tailnet, trimmed to what is read.
    fn whois_doc() -> Value {
        json!({
            "Node": {
                "ID": 1082334865992469i64,
                "StableID": "nQ6vpt6CT911CNTRL",
                "Name": "many.tailb45a71.ts.net.",
                "ComputedName": "many",
                "Online": true
            },
            "UserProfile": { "ID": 5865744717311672i64, "LoginName": "junhyuk@may-i.io", "DisplayName": "박준혁" },
            "CapMap": {}
        })
    }

    #[test]
    fn a_real_whois_document_reads_out_whole() {
        let p = read_whois(&whois_doc()).unwrap();
        assert_eq!(p.login, "junhyuk@may-i.io");
        assert_eq!(p.node, "nQ6vpt6CT911CNTRL", "the stable id, not the name");
        assert!(!p.tagged);
    }

    #[test]
    fn a_document_missing_what_a_pin_needs_resolves_to_nobody() {
        assert_eq!(read_whois(&json!({})), None);
        assert_eq!(read_whois(&json!({ "Node": { "StableID": "n1" } })), None, "no login");
        assert_eq!(read_whois(&json!({ "UserProfile": { "LoginName": "a@b" } })), None, "no node");
        assert_eq!(read_whois(&json!({ "Node": { "StableID": "" }, "UserProfile": { "LoginName": "a@b" } })), None);
        assert_eq!(read_whois(&json!({ "Node": { "StableID": "n1" }, "UserProfile": { "LoginName": "  " } })), None);
    }

    #[test]
    fn a_tagged_node_is_pinned_to_the_node_whatever_is_configured() {
        // An ACL tag replaces the user identity, so every tagged device
        // reports the same login. Under login scope that one key would be the
        // whole fleet's.
        let doc = json!({ "Node": { "StableID": "nCI01" }, "UserProfile": { "LoginName": "tagged-devices" } });
        let p = read_whois(&doc).unwrap();
        assert!(p.tagged);
        assert_eq!(p.key(PinScope::Login), "ts:node:tagged-devices|nCI01", "login scope is overridden");
        assert_eq!(p.key(PinScope::Node), "ts:node:tagged-devices|nCI01");

        // A node carrying tags but a normal login is tagged too.
        let doc = json!({ "Node": { "StableID": "n2", "Tags": ["tag:ci"] }, "UserProfile": { "LoginName": "a@b.io" } });
        assert!(read_whois(&doc).unwrap().tagged);
        let doc = json!({ "Node": { "StableID": "n2", "Tags": [] }, "UserProfile": { "LoginName": "a@b.io" } });
        assert!(!read_whois(&doc).unwrap().tagged, "an empty tag list is not a tag");
    }

    #[test]
    fn the_two_scopes_are_separately_pinned() {
        let p = Peer { login: "a@b.io".into(), node: "n1".into(), tagged: false };
        assert_eq!(p.key(PinScope::Node), "ts:node:a@b.io|n1");
        assert_eq!(p.key(PinScope::Login), "ts:login:a@b.io");
        // The separator cannot appear in either half, so neither key can be
        // read as the other. A login is an email and contains `@`, which is
        // why `@` is not the separator.
        assert!(!p.login.contains('|') && !p.node.contains('|'));
        let other = Peer { login: "a@b.io".into(), node: "n2".into(), tagged: false };
        assert_ne!(p.key(PinScope::Node), other.key(PinScope::Node), "another device, another key");
        assert_eq!(p.key(PinScope::Login), other.key(PinScope::Login), "the same person, by design");
    }

    #[test]
    fn only_a_single_address_inside_the_tailnet_is_attributable() {
        assert_eq!(single_forwarded(&headers(&[("x-forwarded-for", "100.90.48.5")])), Some("100.90.48.5".parse().unwrap()));
        // A chain cannot be attributed, and picking an end is how this gets
        // spoofed: the far end supplies the value.
        assert_eq!(single_forwarded(&headers(&[("x-forwarded-for", "100.90.48.5, 10.0.0.1")])), None);
        assert_eq!(single_forwarded(&headers(&[("x-forwarded-for", "")])), None);
        assert_eq!(single_forwarded(&headers(&[("x-forwarded-for", "not-an-address")])), None);
        assert_eq!(single_forwarded(&HeaderMap::new()), None);

        assert!(in_tailnet("100.64.0.1".parse().unwrap()));
        assert!(in_tailnet("100.127.255.254".parse().unwrap()));
        assert!(in_tailnet("fd7a:115c:a1e0::6737:3005".parse().unwrap()));
        // Just outside 100.64.0.0/10, both ways.
        assert!(!in_tailnet("100.63.255.255".parse().unwrap()));
        assert!(!in_tailnet("100.128.0.0".parse().unwrap()));
        assert!(!in_tailnet("127.0.0.1".parse().unwrap()));
        assert!(!in_tailnet("8.8.8.8".parse().unwrap()));
        assert!(!in_tailnet("fd7a:115c:a1e1::1".parse().unwrap()));
    }

    #[test]
    fn an_empty_allowlist_turns_trust_off_rather_than_letting_everyone_in() {
        // The shape that must never be permissive. A work tailnet is not a
        // private network; this one has 406 devices on it.
        let empty = Trust::default();
        assert!(!empty.on);
        assert!(!empty.allows("anyone@may-i.io"));

        let t = Trust { on: true, logins: vec!["owner@may-i.io".into()], pin: PinScope::Node };
        assert!(t.allows("owner@may-i.io"));
        assert!(t.allows("  Owner@May-I.io  "), "logins are matched case- and space-insensitively");
        assert!(!t.allows("colleague@may-i.io"));
        assert!(!t.allows(""));
    }

    #[test]
    fn the_cache_answers_twice_and_does_not_grow_without_bound() {
        let cache = Cache::default();
        let addr: IpAddr = "100.90.48.5".parse().unwrap();
        assert_eq!(cache.get(addr), None, "nothing known yet");
        let peer = Peer { login: "a@b.io".into(), node: "n1".into(), tagged: false };
        cache.put(addr, Some(peer.clone()));
        assert_eq!(cache.get(addr), Some(Some(peer)));

        // A refusal is cached too: a flood of unknown addresses must not fork
        // a daemon call each.
        let unknown: IpAddr = "100.90.48.6".parse().unwrap();
        cache.put(unknown, None);
        assert_eq!(cache.get(unknown), Some(None));

        for i in 0..(CACHE_MAX + 10) {
            cache.put(IpAddr::V4(std::net::Ipv4Addr::new(100, 64, (i / 256) as u8, (i % 256) as u8)), None);
        }
        assert!(cache.0.lock().len() <= CACHE_MAX, "bounded");
    }
}
