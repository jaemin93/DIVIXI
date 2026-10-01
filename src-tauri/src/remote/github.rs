//! This PC's GitHub account, for remote instances: signing in to a remote
//! instance reached at an address (no SSH to mint a token), and owning this
//! Divixi when it is itself a remote instance for other PCs.
//!
//! Sign-in is GitHub's device flow with an OAuth app the user registers
//! (Device Flow enabled; only its client id is needed, no secret), asking
//! for no scopes: the token can say who the user is and little more. It is
//! kept in the store (`github:account`), out of the settings a remote
//! caller can read.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{AppHandle, AppState};

const ACCOUNT_KEY: &str = "github:account";
/// This instance's owner when it serves: a GitHub login.
pub const OWNER_KEY: &str = "remote:owner";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default)]
    pub login: String,
}

/// Never the token, which is the GitHub account itself.
///
/// Destructured so that a field added to `Account` fails to compile here
/// rather than deciding by default whether it reaches the log.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { client_id, token, login } = self;
        f.debug_struct("Account")
            .field("client_id", client_id)
            .field("token", &if token.is_empty() { "unset" } else { "hidden" })
            .field("login", login)
            .finish()
    }
}

/// What the settings page sees (never the token).
#[derive(Serialize)]
pub struct AccountView {
    pub client_id: String,
    pub login: String,
    /// The owner of this Divixi when it serves.
    pub owner: String,
}

pub fn account(app: &AppHandle) -> Account {
    app.state::<AppState>().store.get_meta(ACCOUNT_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn keep(app: &AppHandle, a: &Account) -> Result<(), String> {
    let json = serde_json::to_string(a).map_err(|e| e.to_string())?;
    app.state::<AppState>().store.set_meta(ACCOUNT_KEY, &json).map_err(|e| e.to_string())
}

pub fn owner(store: &orchestra_store::Store) -> Option<String> {
    store.get_meta(OWNER_KEY).ok().flatten().filter(|o| !o.is_empty())
}

/// A new owner (or none): every device that came in before goes.
pub fn set_owner(store: &orchestra_store::Store, auth: &super::auth::Auth, login: Option<&str>) -> anyhow::Result<()> {
    let now = login.unwrap_or_default();
    if owner(store).unwrap_or_default().eq_ignore_ascii_case(now) {
        return Ok(());
    }
    store.set_meta(OWNER_KEY, now)?;
    auth.drop_all(store);
    Ok(())
}

fn view(app: &AppHandle) -> AccountView {
    let a = account(app);
    AccountView { client_id: a.client_id, login: a.login, owner: owner(&app.state::<AppState>().store).unwrap_or_default() }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().user_agent("divixi").connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().expect("a plain client")
}

/// Who a token belongs to, as GitHub says.
pub async fn login_of(token: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let res = http()
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub did not answer: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GitHub refused the token ({})", res.status()));
    }
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice::<User>(&body).map(|u| u.login).map_err(|e| e.to_string())
}

/// A form body (GitHub's OAuth endpoints take forms).
fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>())).collect::<Vec<_>>().join("&")
}

async fn post_form(url: &str, body: String) -> Result<serde_json::Value, String> {
    let res = http()
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("GitHub did not answer: {e}"))?;
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| format!("GitHub answered: {}", String::from_utf8_lossy(&bytes).chars().take(200).collect::<String>()))
}

/// A device-flow sign-in underway.
#[derive(Default)]
pub struct Pending {
    now: parking_lot::Mutex<Option<(String, u64)>>,
}

#[derive(Serialize)]
pub struct Code {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

// ----- commands (this app's own page; never reachable remotely) -----

#[tauri::command]
pub fn github_account(app: AppHandle) -> AccountView {
    view(&app)
}

#[tauri::command]
pub fn github_set_client_id(app: AppHandle, client_id: String) -> Result<AccountView, String> {
    let id = client_id.trim();
    if !id.is_empty() && !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
        return Err("a client id is letters and digits (Ov23li…)".into());
    }
    let mut a = account(&app);
    a.client_id = id.to_string();
    keep(&app, &a)?;
    Ok(view(&app))
}

/// Start signing in: the code to enter at GitHub (whose page opens).
#[tauri::command]
pub async fn github_login_start(app: AppHandle) -> Result<Code, String> {
    let a = account(&app);
    if a.client_id.is_empty() {
        return Err("enter your GitHub OAuth app's client id first".into());
    }
    let v = post_form("https://github.com/login/device/code", form(&[("client_id", &a.client_id), ("scope", "")])).await?;
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        let hint = if e == "device_flow_disabled" { " (turn on Device Flow in the OAuth app's settings)" } else { "" };
        return Err(format!("GitHub: {e}{hint}"));
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let n = |k: &str, d: u64| v.get(k).and_then(|x| x.as_u64()).unwrap_or(d);
    let code = Code { user_code: s("user_code"), verification_uri: s("verification_uri"), expires_in: n("expires_in", 900) };
    *app.state::<AppState>().github.now.lock() = Some((s("device_code"), n("interval", 5)));
    let _ = crate::open_url(code.verification_uri.clone());
    Ok(code)
}

/// Wait for the user to enter the code at GitHub; then this PC is signed in,
/// and its login owns this Divixi when it serves.
#[tauri::command]
pub async fn github_login_wait(app: AppHandle) -> Result<AccountView, String> {
    let (device_code, mut interval) = app.state::<AppState>().github.now.lock().clone().ok_or("no sign-in underway")?;
    let mut a = account(&app);
    let token = loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        // Another sign-in started meanwhile: this one is over.
        if app.state::<AppState>().github.now.lock().as_ref().map(|(d, _)| d.as_str()) != Some(device_code.as_str()) {
            return Err("the sign-in was replaced".into());
        }
        let v = post_form(
            "https://github.com/login/oauth/access_token",
            form(&[("client_id", &a.client_id), ("device_code", &device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")]),
        )
        .await?;
        if let Some(t) = v.get("access_token").and_then(|t| t.as_str()) {
            break t.to_string();
        }
        match v.get("error").and_then(|e| e.as_str()).unwrap_or_default() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            "expired_token" => return Err("the code expired: sign in again".into()),
            "access_denied" => return Err("the sign-in was cancelled at GitHub".into()),
            other => return Err(format!("GitHub: {other}")),
        }
    };
    *app.state::<AppState>().github.now.lock() = None;
    a.login = login_of(&token).await?;
    a.token = token;
    keep(&app, &a)?;
    let st = app.state::<AppState>();
    set_owner(&st.store, &st.remote.auth, Some(&a.login)).map_err(|e| e.to_string())?;
    Ok(view(&app))
}

/// Sign out: the token goes, and so does this Divixi's owner (nobody comes
/// in by GitHub until someone signs in again).
#[tauri::command]
pub fn github_logout(app: AppHandle) -> Result<AccountView, String> {
    let mut a = account(&app);
    a.token.clear();
    a.login.clear();
    keep(&app, &a)?;
    let st = app.state::<AppState>();
    set_owner(&st.store, &st.remote.auth, None).map_err(|e| e.to_string())?;
    Ok(view(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_encoded() {
        assert_eq!(form(&[("client_id", "Ov23li.x_Y"), ("scope", "")]), "client_id=Ov23li.x_Y&scope=");
        assert_eq!(form(&[("grant_type", "urn:ietf:x")]), "grant_type=urn%3Aietf%3Ax");
    }

    #[test]
    fn a_new_owner_drops_everyone() {
        let store = orchestra_store::Store::in_memory().unwrap();
        let auth = super::super::auth::Auth::with_key([1u8; 32]);
        set_owner(&store, &auth, Some("octocat")).unwrap();
        let (access, _) = auth.admit(&store, "Laptop", Some("octocat".into()), crate::remote::auth::Scope::Full, crate::remote::auth::REFRESH_SECS);
        set_owner(&store, &auth, Some("OctoCat")).unwrap();
        assert!(auth.check(&store, &access, owner(&store).as_deref()).is_ok(), "the same login, other case: nothing changes");
        set_owner(&store, &auth, Some("someone")).unwrap();
        assert!(auth.check(&store, &access, owner(&store).as_deref()).is_err());
        assert_eq!(owner(&store).as_deref(), Some("someone"));
        set_owner(&store, &auth, None).unwrap();
        assert_eq!(owner(&store), None);
    }
}
