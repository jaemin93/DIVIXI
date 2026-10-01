//! Who may use this Divixi as a remote instance: devices and their tokens.
//!
//! As in Kiro Crew (https://github.com/kirodotdev/KiroCrew):
//! a token is `base64url(claims).base64url(HMAC-SHA256)` under a key kept
//! in the app's data folder, so a restart signs nobody out. A device comes
//! in one of two ways: with a pairing token minted on this machine
//! (`divixi-server token`, run over SSH: whoever can do that owns it; five
//! minutes, good once), or as this instance's owner's GitHub account
//! (server.rs checks that with GitHub). Either way it gets an access token
//! (an hour) and a refresh token (thirty days, renewed each time it is
//! used). Each device is listed and can be dropped; a new owner moves a
//! generation every token carries, dropping them all. A refresh token used
//! twice means it was copied: the device is dropped.

use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
// hmac 0.13 moved `new_from_slice` from `Mac` to `KeyInit`; both are needed.
use hmac::{Hmac, KeyInit, Mac};
use orchestra_store::Store;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// How long a pairing link can be opened.
pub const PAIR_SECS: i64 = 5 * 60;
/// How long an access cookie lasts.
pub const ACCESS_SECS: i64 = 60 * 60;
/// How long a refresh cookie lasts unused.
pub const REFRESH_SECS: i64 = 30 * 24 * 60 * 60;

const DEVICES_KEY: &str = "remote:devices";
const GENERATION_KEY: &str = "remote:generation";
const USED_KEY: &str = "remote:used";

/// How long a phone stays signed in, in days. The screen that mints a link
/// says this number, so the two cannot drift: whatever is shown is what is
/// signed into the token.
pub const PHONE_DAYS: &[i64] = &[1, 7, 30];

/// Seven, because one day trains whoever owns this to keep a QR on screen,
/// and thirty is long enough that losing the phone wants the drop button
/// rather than patience.
pub const PHONE_DAYS_DEFAULT: i64 = 7;

fn default_life() -> i64 {
    REFRESH_SECS
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Pair,
    Access,
    Refresh,
}

/// How much of this Divixi a device may drive ([`super::bridge::allowed`]).
///
/// The scope is carried *in the signed token*, not chosen by the endpoint that
/// redeems it. A pairing link minted for a phone must stay a phone's link even
/// if it is posted to the endpoint the Divixi app uses; deciding at the
/// endpoint would let whoever holds the link pick their own permissions.
///
/// `Full` is the serde default so a token or a device stored before this field
/// existed keeps the access it was granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Everything a remote caller may do: the conversation, and the machine
    /// (its shell, its folders, deleting things). What a pairing token minted
    /// on this machine over SSH, or this instance's owner signing in with
    /// GitHub, has always granted.
    #[default]
    Full,
    /// The conversation only ([`super::bridge::ALLOWED`]). A phone.
    Conversation,
}

/// What a token says.
#[derive(Clone, Serialize, Deserialize)]
pub struct Claims {
    pub k: Kind,
    /// Expiry, unix seconds.
    pub exp: i64,
    /// A one-time value (pairing) or the refresh chain's current link.
    pub n: String,
    /// The device, for access and refresh tokens.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub d: String,
    /// The generation the token was issued in.
    pub g: u64,
    /// What the device this token is for may do. Signed, so it cannot be
    /// widened by whoever presents the token.
    #[serde(default)]
    pub s: Scope,
    /// How long the device redeeming this stays signed in, in seconds.
    /// Absent means [`REFRESH_SECS`], which is every token minted before the
    /// span could be chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l: Option<i64>,
}

impl Claims {
    /// How long a device admitted on this token keeps its refresh.
    fn life(&self) -> i64 {
        self.l.filter(|l| *l > 0).unwrap_or(REFRESH_SECS)
    }
}

/// A device paired with this Divixi.
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    /// From its browser, for the list.
    pub name: String,
    pub created: i64,
    pub last_seen: i64,
    /// The GitHub login it came in as; it stays in while that is the owner.
    #[serde(default)]
    pub login: Option<String>,
    /// The refresh token's current link; an older one coming back is a copy.
    #[serde(default)]
    refresh: String,
    /// What this device may do. Fixed when it is admitted and never widened.
    #[serde(default)]
    pub scope: Scope,
    /// How long this device may go unused before its refresh stops working.
    /// Its own, not a global: a phone is signed in for days and the Divixi
    /// app for a month.
    #[serde(default = "default_life")]
    pub life: i64,
}

/// Without `n`: it is the one-time value a pairing link is good for and
/// the link in the refresh chain, so it does not go in a log.
///
/// Both impls below destructure `Self`, and the field they will not print
/// is bound to `_` rather than skipped: a field added to either struct then
/// fails to compile here, so whoever adds it decides whether it may be
/// logged. `finish_non_exhaustive()` is what says a field was left out.
impl std::fmt::Debug for Claims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { k, exp, n: _, d, g, s, l } = self;
        f.debug_struct("Claims").field("k", k).field("exp", exp).field("d", d).field("g", g).field("s", s).field("l", l).finish_non_exhaustive()
    }
}

/// Without the refresh token it holds; the rest is for the device list.
impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { id, name, created, last_seen, login, refresh: _, scope, life } = self;
        f.debug_struct("Device")
            .field("id", id)
            .field("name", name)
            .field("created", created)
            .field("last_seen", last_seen)
            .field("login", login)
            .field("scope", scope)
            .field("life", life)
            .finish_non_exhaustive()
    }
}

/// Why a request is not let in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No token, a bad or expired one: sign in again.
    SignIn,
    /// The device was dropped (or its refresh token was copied).
    Dropped,
}

pub struct Auth {
    key: [u8; 32],
    /// Devices, generation and spent links are read and written under this,
    /// in the store.
    lock: parking_lot::Mutex<()>,
}

impl Auth {
    /// The signing key from `<data>/remote.key`, made the first time.
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("remote.key");
        let key = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&bytes);
                k
            }
            _ => {
                let mut k = [0u8; 32];
                k[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
                k[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
                std::fs::write(&path, k)?;
                k
            }
        };
        Ok(Self::with_key(key))
    }

    pub fn with_key(key: [u8; 32]) -> Self {
        Self { key, lock: parking_lot::Mutex::new(()) }
    }

    fn mac(&self, data: &[u8]) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("any key length works");
        mac.update(data);
        mac
    }

    pub fn sign(&self, claims: &Claims) -> String {
        let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims serialize"));
        let sig = URL_SAFE_NO_PAD.encode(self.mac(body.as_bytes()).finalize().into_bytes());
        format!("{body}.{sig}")
    }

    /// The claims of a token signed here and not expired.
    pub fn verify(&self, token: &str) -> Option<Claims> {
        let (body, sig) = token.split_once('.')?;
        let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
        self.mac(body.as_bytes()).verify_slice(&sig).ok()?;
        let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).ok()?).ok()?;
        (claims.exp > now()).then_some(claims)
    }

    fn generation(&self, store: &Store) -> u64 {
        store.get_meta(GENERATION_KEY).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn devices(&self, store: &Store) -> Vec<Device> {
        store.get_meta(DEVICES_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn save_devices(&self, store: &Store, devices: &[Device]) {
        if let Ok(json) = serde_json::to_string(devices) {
            if let Err(err) = store.set_meta(DEVICES_KEY, &json) {
                tracing::warn!(%err, "could not keep the paired devices");
            }
        }
    }

    /// A pairing link's token, and when it stops working.
    ///
    /// `scope` and `life` are what the device redeeming it will get, and both
    /// are signed into the token: a link minted for a phone cannot be redeemed
    /// for more, or for longer, by sending it somewhere else.
    pub fn pair_token(&self, store: &Store, scope: Scope, life: i64) -> (String, i64) {
        let exp = now() + PAIR_SECS;
        let claims = Claims { k: Kind::Pair, exp, n: nonce(), d: String::new(), g: self.generation(store), s: scope, l: Some(life) };
        (self.sign(&claims), exp)
    }

    /// Spend a pairing link's one-time value, or say it was spent already.
    ///
    /// Kept in the store rather than in memory. A restart used to forget every
    /// spent link, so a link photographed off the screen could be redeemed a
    /// second time inside its five minutes if the app happened to restart --
    /// and "scan once" is printed beside the code.
    ///
    /// The caller holds [`Auth::lock`], which is what makes the read, the
    /// insert and the write one step: two phones scanning the same code at
    /// once must not both get in.
    fn spend(&self, store: &Store, nonce: &str, exp: i64) -> bool {
        let mut used: HashMap<String, i64> =
            store.get_meta(USED_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        let t = now();
        // A spent link is only worth remembering until it would have expired.
        used.retain(|_, e| *e > t);
        let fresh = used.insert(nonce.to_string(), exp).is_none();
        match serde_json::to_string(&used) {
            Ok(json) => {
                if let Err(err) = store.set_meta(USED_KEY, &json) {
                    // Could not record it, so cannot promise it is only used
                    // once: refuse rather than let it through unrecorded.
                    tracing::warn!(%err, "could not record a spent pairing link; refusing it");
                    return false;
                }
            }
            Err(err) => {
                tracing::warn!(%err, "could not record a spent pairing link; refusing it");
                return false;
            }
        }
        fresh
    }

    /// Access and refresh tokens for a device, its refresh link moved on.
    fn issue(&self, store: &Store, devices: &mut [Device], i: usize) -> (String, String) {
        let g = self.generation(store);
        let link = nonce();
        devices[i].refresh = link.clone();
        devices[i].last_seen = now();
        let id = devices[i].id.clone();
        let s = devices[i].scope;
        let l = Some(devices[i].life);
        let access = self.sign(&Claims { k: Kind::Access, exp: now() + ACCESS_SECS, n: nonce(), d: id.clone(), g, s, l });
        let refresh = self.sign(&Claims { k: Kind::Refresh, exp: now() + devices[i].life, n: link, d: id, g, s, l });
        (access, refresh)
    }

    /// Open a pairing link: once, within its five minutes. A new device.
    pub fn redeem(&self, store: &Store, token: &str, name: &str) -> Result<(String, String), Refused> {
        let claims = self.verify(token).filter(|c| c.k == Kind::Pair).ok_or(Refused::SignIn)?;
        {
            let _one = self.lock.lock();
            // The generation first, so a link from before everything was
            // dropped does not burn a nonce on its way to being refused.
            if claims.g != self.generation(store) || !self.spend(store, &claims.n, claims.exp) {
                return Err(Refused::SignIn);
            }
        }
        // Scope and span come from the token, not from this endpoint: see [`Scope`].
        Ok(self.admit(store, name, None, claims.s, claims.life()))
    }

    /// A new device, let in by the caller (a pairing token, or the owner's
    /// GitHub account as `login`).
    pub fn admit(&self, store: &Store, name: &str, login: Option<String>, scope: Scope, life: i64) -> (String, String) {
        let _one = self.lock.lock();
        let mut devices = self.devices(store);
        let t = now();
        // Each app start signs in anew: devices whose refresh token has run
        // out can never come back, and would only pile up. Each by its own
        // span, now that a phone's is shorter than the app's.
        devices.retain(|d| d.last_seen > t - d.life);
        devices.push(Device {
            id: nonce()[..12].to_string(),
            name: name.chars().take(80).collect(),
            created: t,
            last_seen: t,
            login,
            refresh: String::new(),
            scope,
            life: if life > 0 { life } else { REFRESH_SECS },
        });
        let i = devices.len() - 1;
        let pair = self.issue(store, &mut devices, i);
        self.save_devices(store, &devices);
        pair
    }

    /// The device an access token belongs to, if it may still come in.
    /// `owner` is this instance's owner (a GitHub login) now.
    pub fn check(&self, store: &Store, token: &str, owner: Option<&str>) -> Result<Device, Refused> {
        let claims = self.verify(token).filter(|c| c.k == Kind::Access).ok_or(Refused::SignIn)?;
        if claims.g != self.generation(store) {
            return Err(Refused::Dropped);
        }
        let device = self.devices(store).into_iter().find(|d| d.id == claims.d).ok_or(Refused::Dropped)?;
        // In as the owner's GitHub account: out once another owns it.
        if let Some(login) = device.login.as_deref() {
            if !owner.is_some_and(|o| o.eq_ignore_ascii_case(login)) {
                return Err(Refused::Dropped);
            }
        }
        Ok(device)
    }

    /// New tokens for a refresh token. One used before means a copy: the device goes.
    pub fn refresh(&self, store: &Store, token: &str) -> Result<(String, String), Refused> {
        let claims = self.verify(token).filter(|c| c.k == Kind::Refresh).ok_or(Refused::SignIn)?;
        let _one = self.lock.lock();
        if claims.g != self.generation(store) {
            return Err(Refused::Dropped);
        }
        let mut devices = self.devices(store);
        let i = devices.iter().position(|d| d.id == claims.d).ok_or(Refused::Dropped)?;
        if devices[i].refresh != claims.n {
            tracing::warn!(device = %devices[i].id, "a refresh token came back twice; dropping the device");
            devices.remove(i);
            self.save_devices(store, &devices);
            return Err(Refused::Dropped);
        }
        let pair = self.issue(store, &mut devices, i);
        self.save_devices(store, &devices);
        Ok(pair)
    }

    /// Drop one device.
    pub fn drop_device(&self, store: &Store, id: &str) {
        let _one = self.lock.lock();
        let mut devices = self.devices(store);
        devices.retain(|d| d.id != id);
        self.save_devices(store, &devices);
    }

    /// Drop every device: tokens of the old generation stop working.
    pub fn drop_all(&self, store: &Store) {
        let _one = self.lock.lock();
        let g = self.generation(store) + 1;
        let _ = store.set_meta(GENERATION_KEY, &g.to_string());
        self.save_devices(store, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> (Auth, Store) {
        (Auth::with_key([7u8; 32]), Store::in_memory().unwrap())
    }

    #[test]
    fn a_pairing_link_works_once() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        let (access, _) = a.redeem(&s, &link, "Phone").unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().name, "Phone");
        assert_eq!(a.redeem(&s, &link, "Phone"), Err(Refused::SignIn), "a link is good once");
        assert_eq!(a.redeem(&s, "junk.token", "x"), Err(Refused::SignIn));
        // A link is not an access token, nor the other way round.
        assert_eq!(a.check(&s, &link, None).unwrap_err(), Refused::SignIn);
    }

    #[test]
    fn tampering_is_refused() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        let (access, _) = a.redeem(&s, &link, "Phone").unwrap();
        let (body, sig) = access.split_once('.').unwrap();
        let mut forged = serde_json::from_slice::<Claims>(&URL_SAFE_NO_PAD.decode(body).unwrap()).unwrap();
        forged.exp += 1_000_000;
        let forged = format!("{}.{sig}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap()));
        assert_eq!(a.check(&s, &forged, None).unwrap_err(), Refused::SignIn);
        let other = Auth::with_key([8u8; 32]);
        assert_eq!(other.check(&s, &access, None).unwrap_err(), Refused::SignIn, "another key signed it");
    }

    #[test]
    fn refresh_rotates_and_a_copy_drops_the_device() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        let (_, refresh) = a.redeem(&s, &link, "Phone").unwrap();
        let (access2, refresh2) = a.refresh(&s, &refresh).unwrap();
        assert!(a.check(&s, &access2, None).is_ok());
        // The old refresh token again: someone copied it.
        assert_eq!(a.refresh(&s, &refresh), Err(Refused::Dropped));
        assert!(a.devices(&s).is_empty());
        assert_eq!(a.refresh(&s, &refresh2), Err(Refused::Dropped));
        assert_eq!(a.check(&s, &access2, None).unwrap_err(), Refused::Dropped);
    }

    #[test]
    fn drop_one() {
        let (a, s) = auth();
        let pair = |name: &str| a.redeem(&s, &a.pair_token(&s, Scope::Full, REFRESH_SECS).0, name).unwrap();
        let (phone, _) = pair("Phone");
        let (tablet, _) = pair("Tablet");
        let phone_id = a.check(&s, &phone, None).unwrap().id;
        a.drop_device(&s, &phone_id);
        assert_eq!(a.check(&s, &phone, None).unwrap_err(), Refused::Dropped);
        assert!(a.check(&s, &tablet, None).is_ok());
    }

    #[test]
    fn a_device_in_as_the_owner_stays_in_while_they_own_it() {
        let (a, s) = auth();
        let (access, _) = a.admit(&s, "Laptop", Some("Octocat".into()), Scope::Full, REFRESH_SECS);
        assert!(a.check(&s, &access, Some("octocat")).is_ok(), "GitHub logins ignore case");
        assert_eq!(a.check(&s, &access, Some("someone")).unwrap_err(), Refused::Dropped);
        assert_eq!(a.check(&s, &access, None).unwrap_err(), Refused::Dropped, "no owner, no GitHub devices");
        let (paired, _) = a.redeem(&s, &a.pair_token(&s, Scope::Full, REFRESH_SECS).0, "SSH").unwrap();
        assert!(a.check(&s, &paired, None).is_ok(), "a device paired over SSH has no login to match");
    }

    #[test]
    fn devices_past_their_refresh_are_let_go() {
        let (a, s) = auth();
        a.admit(&s, "Old", None, Scope::Full, REFRESH_SECS);
        let mut devices = a.devices(&s);
        devices[0].last_seen -= REFRESH_SECS + 1;
        a.save_devices(&s, &devices);
        a.admit(&s, "New", None, Scope::Full, REFRESH_SECS);
        let names: Vec<_> = a.devices(&s).into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["New"]);
    }

    #[test]
    fn a_scope_rides_in_the_token_and_sticks_to_the_device() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Conversation, 7 * 24 * 60 * 60);
        let (access, refresh) = a.redeem(&s, &link, "Phone").unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().scope, Scope::Conversation);
        // Renewing does not widen it.
        let (access2, _) = a.refresh(&s, &refresh).unwrap();
        assert_eq!(a.check(&s, &access2, None).unwrap().scope, Scope::Conversation);
    }

    #[test]
    fn a_phones_link_cannot_be_redeemed_for_more() {
        // The hole this guards: the Divixi app and a phone redeem through
        // different endpoints, so if the endpoint picked the scope, posting a
        // phone's link to the app's endpoint would hand it the machine.
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Conversation, 7 * 24 * 60 * 60);
        let (access, _) = a.redeem(&s, &link, "pretending to be the app").unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().scope, Scope::Conversation);
    }

    #[test]
    fn a_token_from_before_the_scope_existed_is_full() {
        // Devices and tokens kept by an older build carry no `s`/`scope`, and
        // must not quietly lose the access they were granted.
        let (a, s) = auth();
        let old: Claims = serde_json::from_str(r#"{"k":"pair","exp":99999999999,"n":"abc","g":0}"#).unwrap();
        assert_eq!(old.s, Scope::Full);
        let (access, _) = a.redeem(&s, &a.sign(&old), "Older app").unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().scope, Scope::Full);
        let device: Device = serde_json::from_str(r#"{"id":"x","name":"Old","created":0,"last_seen":0}"#).unwrap();
        assert_eq!(device.scope, Scope::Full);
    }

    #[test]
    fn a_spent_link_stays_spent_across_a_restart() {
        // The promise printed beside the code is "scan once". It used to hold
        // only until the app restarted, because spent links lived in memory.
        let store = Store::in_memory().unwrap();
        let first = Auth::with_key([7u8; 32]);
        let (link, _) = first.pair_token(&store, Scope::Conversation, REFRESH_SECS);
        assert!(first.redeem(&store, &link, "Phone").is_ok());
        // A new Auth over the same store is what a restart looks like.
        let after = Auth::with_key([7u8; 32]);
        assert_eq!(after.redeem(&store, &link, "Phone again"), Err(Refused::SignIn), "a restart does not forget");
    }

    #[test]
    fn spent_links_do_not_pile_up_past_their_expiry() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        a.redeem(&s, &link, "One").unwrap();
        let spent: HashMap<String, i64> = serde_json::from_str(&s.get_meta(USED_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(spent.len(), 1);
        // Age the record past its expiry; the next spend sweeps it.
        let old: HashMap<String, i64> = spent.keys().map(|k| (k.clone(), now() - 1)).collect();
        s.set_meta(USED_KEY, &serde_json::to_string(&old).unwrap()).unwrap();
        let (next, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        a.redeem(&s, &next, "Two").unwrap();
        let spent: HashMap<String, i64> = serde_json::from_str(&s.get_meta(USED_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(spent.len(), 1, "the old one was swept, not kept forever");
    }

    #[test]
    fn a_phones_span_is_its_own_and_rides_in_the_token() {
        let (a, s) = auth();
        let week = 7 * 24 * 60 * 60;
        let (link, _) = a.pair_token(&s, Scope::Conversation, week);
        let (access, refresh) = a.redeem(&s, &link, "Phone").unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().life, week);
        // The refresh token expires with the device's span, not the app's.
        let claims = a.verify(&refresh).unwrap();
        let issued = claims.exp - week;
        assert!((issued - now()).abs() <= 2, "a week, not {}", claims.exp - now());
        // Renewing keeps the span.
        let (_, refresh2) = a.refresh(&s, &refresh).unwrap();
        assert_eq!(a.verify(&refresh2).unwrap().l, Some(week));
    }

    #[test]
    fn a_device_from_before_the_span_existed_keeps_the_month() {
        let device: Device = serde_json::from_str(r#"{"id":"x","name":"Old","created":0,"last_seen":0}"#).unwrap();
        assert_eq!(device.life, REFRESH_SECS);
        let old: Claims = serde_json::from_str(r#"{"k":"pair","exp":99999999999,"n":"abc","g":0}"#).unwrap();
        assert_eq!(old.life(), REFRESH_SECS);
    }

    #[test]
    fn a_short_lived_device_is_let_go_before_a_long_lived_one() {
        let (a, s) = auth();
        let day = 24 * 60 * 60;
        a.admit(&s, "Phone", None, Scope::Conversation, day);
        a.admit(&s, "App", None, Scope::Full, REFRESH_SECS);
        let mut devices = a.devices(&s);
        // Both idle for two days: only the phone's span has run out.
        for d in devices.iter_mut() {
            d.last_seen -= 2 * day;
        }
        a.save_devices(&s, &devices);
        a.admit(&s, "New", None, Scope::Full, REFRESH_SECS);
        let names: Vec<_> = a.devices(&s).into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["App", "New"]);
    }

    #[test]
    fn drop_all_ends_every_token() {
        let (a, s) = auth();
        let (access, refresh) = a.admit(&s, "Laptop", Some("octocat".into()), Scope::Full, REFRESH_SECS);
        let (link, _) = a.pair_token(&s, Scope::Full, REFRESH_SECS);
        a.drop_all(&s);
        assert_eq!(a.check(&s, &access, Some("octocat")).unwrap_err(), Refused::Dropped);
        assert_eq!(a.refresh(&s, &refresh), Err(Refused::Dropped));
        assert_eq!(a.redeem(&s, &link, "Late"), Err(Refused::SignIn), "a link from before goes too");
    }
}
