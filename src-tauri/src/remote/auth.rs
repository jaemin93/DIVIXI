//! Who may use Divixi from another device: pairing links, cookies, devices.
//!
//! As in Kiro Crew (docs/system-specs/modules/dashboard-token-auth.md):
//! a token is `base64url(claims).base64url(HMAC-SHA256)` under a key kept
//! in the app's data folder, so a restart signs nobody out. A pairing link
//! lives five minutes and is good once; it becomes an access cookie (an
//! hour) and a refresh cookie (thirty days, renewed each time it is used).
//! Each paired device is listed and can be dropped; "drop all" moves a
//! generation every token carries. A refresh token used twice means it
//! was copied: the device is dropped.

use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
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

/// What a token says.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// A device paired with this Divixi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    /// From its browser, for the list.
    pub name: String,
    pub created: i64,
    pub last_seen: i64,
    /// Its Tailscale login when it paired through tailscale serve; later
    /// requests through serve must carry the same.
    #[serde(default)]
    pub login: Option<String>,
    /// The refresh token's current link; an older one coming back is a copy.
    #[serde(default)]
    refresh: String,
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
    /// Pairing links already used, until they would have expired anyway.
    used: parking_lot::Mutex<HashMap<String, i64>>,
    /// Devices and generation are read and written under this, in the store.
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
        Self { key, used: parking_lot::Mutex::new(HashMap::new()), lock: parking_lot::Mutex::new(()) }
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
    pub fn pair_token(&self, store: &Store) -> (String, i64) {
        let exp = now() + PAIR_SECS;
        let token = self.sign(&Claims { k: Kind::Pair, exp, n: nonce(), d: String::new(), g: self.generation(store) });
        (token, exp)
    }

    /// Access and refresh tokens for a device, its refresh link moved on.
    fn issue(&self, store: &Store, devices: &mut [Device], i: usize) -> (String, String) {
        let g = self.generation(store);
        let link = nonce();
        devices[i].refresh = link.clone();
        devices[i].last_seen = now();
        let id = devices[i].id.clone();
        let access = self.sign(&Claims { k: Kind::Access, exp: now() + ACCESS_SECS, n: nonce(), d: id.clone(), g });
        let refresh = self.sign(&Claims { k: Kind::Refresh, exp: now() + REFRESH_SECS, n: link, d: id, g });
        (access, refresh)
    }

    /// Open a pairing link: once, within its five minutes. A new device.
    pub fn redeem(&self, store: &Store, token: &str, name: &str, login: Option<String>) -> Result<(String, String), Refused> {
        let claims = self.verify(token).filter(|c| c.k == Kind::Pair).ok_or(Refused::SignIn)?;
        {
            let mut used = self.used.lock();
            let t = now();
            used.retain(|_, exp| *exp > t);
            if used.insert(claims.n.clone(), claims.exp).is_some() {
                return Err(Refused::SignIn);
            }
        }
        let _one = self.lock.lock();
        if claims.g != self.generation(store) {
            return Err(Refused::SignIn);
        }
        let mut devices = self.devices(store);
        let t = now();
        devices.push(Device {
            id: nonce()[..12].to_string(),
            name: name.chars().take(80).collect(),
            created: t,
            last_seen: t,
            login,
            refresh: String::new(),
        });
        let i = devices.len() - 1;
        let pair = self.issue(store, &mut devices, i);
        self.save_devices(store, &devices);
        Ok(pair)
    }

    /// The device an access token belongs to, if it may still come in.
    /// `login` is the Tailscale login of a request through serve.
    pub fn check(&self, store: &Store, token: &str, login: Option<&str>) -> Result<Device, Refused> {
        let claims = self.verify(token).filter(|c| c.k == Kind::Access).ok_or(Refused::SignIn)?;
        if claims.g != self.generation(store) {
            return Err(Refused::Dropped);
        }
        let device = self.devices(store).into_iter().find(|d| d.id == claims.d).ok_or(Refused::Dropped)?;
        // Paired through serve as one person: through serve it must be that person.
        if let (Some(want), Some(got)) = (device.login.as_deref(), login) {
            if want != got {
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
        let (link, _) = a.pair_token(&s);
        let (access, _) = a.redeem(&s, &link, "Phone", None).unwrap();
        assert_eq!(a.check(&s, &access, None).unwrap().name, "Phone");
        assert_eq!(a.redeem(&s, &link, "Phone", None), Err(Refused::SignIn), "a link is good once");
        assert_eq!(a.redeem(&s, "junk.token", "x", None), Err(Refused::SignIn));
        // A link is not an access token, nor the other way round.
        assert_eq!(a.check(&s, &link, None).unwrap_err(), Refused::SignIn);
    }

    #[test]
    fn tampering_is_refused() {
        let (a, s) = auth();
        let (link, _) = a.pair_token(&s);
        let (access, _) = a.redeem(&s, &link, "Phone", None).unwrap();
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
        let (link, _) = a.pair_token(&s);
        let (_, refresh) = a.redeem(&s, &link, "Phone", None).unwrap();
        let (access2, refresh2) = a.refresh(&s, &refresh).unwrap();
        assert!(a.check(&s, &access2, None).is_ok());
        // The old refresh token again: someone copied it.
        assert_eq!(a.refresh(&s, &refresh), Err(Refused::Dropped));
        assert!(a.devices(&s).is_empty());
        assert_eq!(a.refresh(&s, &refresh2), Err(Refused::Dropped));
        assert_eq!(a.check(&s, &access2, None).unwrap_err(), Refused::Dropped);
    }

    #[test]
    fn drop_one_or_all() {
        let (a, s) = auth();
        let pair = |name: &str| a.redeem(&s, &a.pair_token(&s).0, name, None).unwrap();
        let (phone, _) = pair("Phone");
        let (tablet, _) = pair("Tablet");
        let phone_id = a.check(&s, &phone, None).unwrap().id;
        a.drop_device(&s, &phone_id);
        assert_eq!(a.check(&s, &phone, None).unwrap_err(), Refused::Dropped);
        assert!(a.check(&s, &tablet, None).is_ok());
        let (link, _) = a.pair_token(&s);
        a.drop_all(&s);
        assert_eq!(a.check(&s, &tablet, None).unwrap_err(), Refused::Dropped);
        assert_eq!(a.redeem(&s, &link, "Late", None), Err(Refused::SignIn), "a link from before goes too");
    }

    #[test]
    fn a_device_paired_as_one_tailscale_user_stays_that_user() {
        let (a, s) = auth();
        let (access, _) = a.redeem(&s, &a.pair_token(&s).0, "Phone", Some("me@example.com".into())).unwrap();
        assert!(a.check(&s, &access, Some("me@example.com")).is_ok());
        assert_eq!(a.check(&s, &access, Some("someone@else.com")).unwrap_err(), Refused::Dropped);
        assert!(a.check(&s, &access, None).is_ok(), "on the PC itself, no serve, no login");
    }
}
