//! Vectors for the knowledge library from a remote embedding API, so the
//! library can search by meaning without a local model resident in memory
//! (devterm's local llama-server held about 5 GB while idle). Any endpoint
//! that speaks OpenAI's `POST {base}/embeddings` works: OpenAI, Voyage,
//! Gemini's OpenAI-compatible endpoint, Ollama or LM Studio, a llama-server
//! on another machine.
//!
//! Every vector is stamped with the space it belongs to (endpoint, model,
//! dimensions); changing any of them re-embeds the library in the
//! background and, until then, searches only the vectors already in the
//! new space.

use std::time::Duration;

use serde_json::{json, Value};

use crate::{AppState, SETTING_PREFIX};

pub const SETTING_ENABLED: &str = "knowledge.embed.enabled";
pub const SETTING_URL: &str = "knowledge.embed.url";
pub const SETTING_MODEL: &str = "knowledge.embed.model";
pub const SETTING_KEY: &str = "knowledge.embed.key";
pub const SETTING_DIMS: &str = "knowledge.embed.dims";
pub const SETTING_RATE: &str = "knowledge.embed.rate";

/// Texts per request.
pub const BATCH: usize = 32;
/// Requests per minute unless the settings say otherwise (Kiro Crew's
/// default embedding rate limit); 0 means no limit.
pub const DEFAULT_PER_MINUTE: u64 = 120;
const TIMEOUT: Duration = Duration::from_secs(60);
/// A search waits this long for its query's vector before going without.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Where vectors come from.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Endpoint {
    pub url: String,
    pub model: String,
    pub key: String,
    /// Requested dimensions, for models that can shorten their vectors.
    pub dims: Option<u32>,
}

/// Everything but the key, which is the user's API key for the endpoint.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("url", &redact_url(&self.url))
            .field("model", &self.model)
            .field("key", &if self.key.is_empty() { "unset" } else { "hidden" })
            .field("dims", &self.dims)
            .finish()
    }
}

/// An endpoint address as it may be logged or shown in an error: without
/// the query string, where some providers want the key (`?api-key=…`),
/// and without any `user:password@` in front of the host.
pub fn redact_url(url: &str) -> String {
    let (rest, query) = url.split_once('?').map_or((url, false), |(rest, _)| (rest, true));
    let mut out = match rest.split_once("://") {
        Some((scheme, after)) => {
            let (authority, path) = after.split_once('/').map_or((after, ""), |(a, p)| (a, p));
            // Only the authority: a path may hold an `@` of its own.
            match authority.rsplit_once('@') {
                Some((_, host)) => format!("{scheme}://…@{host}/{path}"),
                None => rest.to_string(),
            }
        }
        None => rest.to_string(),
    };
    if query {
        out.push('?');
        out.push('…');
    }
    out
}

impl Endpoint {
    /// The vector space this endpoint's vectors are in.
    pub fn signature(&self) -> String {
        let dims = self.dims.map(|d| d.to_string()).unwrap_or_default();
        orchestra_knowledge::read::hash(&format!("{}|{}|{}", self.url.trim_end_matches('/'), self.model, dims))[..16].to_string()
    }
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.store.get_meta(&format!("{SETTING_PREFIX}{key}")).ok().flatten().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Embedding requests allowed per minute; 0 means no limit.
pub fn per_minute(state: &AppState) -> u64 {
    setting(state, SETTING_RATE).and_then(|r| r.parse().ok()).unwrap_or(DEFAULT_PER_MINUTE)
}

/// The pause after each request that keeps within the rate limit.
pub fn spacing(per_minute: u64) -> Duration {
    60_000u64.checked_div(per_minute).map(Duration::from_millis).unwrap_or(Duration::ZERO)
}

/// The endpoint in the settings, when embeddings are switched on and set up.
pub fn endpoint(state: &AppState) -> Option<Endpoint> {
    if setting(state, SETTING_ENABLED).as_deref() != Some("on") {
        return None;
    }
    let url = setting(state, SETTING_URL)?;
    let model = setting(state, SETTING_MODEL)?;
    Some(Endpoint {
        url,
        model,
        key: setting(state, SETTING_KEY).unwrap_or_default(),
        dims: setting(state, SETTING_DIMS).and_then(|d| d.parse().ok()).filter(|d| *d > 0),
    })
}

/// Why a request failed, and whether trying again later can help.
#[derive(Debug, Clone)]
pub struct EmbedError {
    pub message: String,
    /// The endpoint refused these texts themselves (400, 413, 422): the
    /// same texts will fail again. Auth, address and model problems, rate
    /// limits, server errors and unreadable replies are not the texts'
    /// fault and are retried once fixed.
    pub permanent: bool,
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl EmbedError {
    fn passing(message: String) -> Self {
        Self { message, permanent: false }
    }
}

/// One request: a vector per text, in order.
pub async fn embed(ep: &Endpoint, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let mut body = json!({ "model": ep.model, "input": texts });
    if let Some(d) = ep.dims {
        body["dimensions"] = json!(d);
    }
    let url = format!("{}/embeddings", ep.url.trim_end_matches('/'));
    // What the failures below name. `url` itself may carry the key.
    let shown = redact_url(&url);
    let mut req = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .timeout(TIMEOUT)
        .body(body.to_string());
    if !ep.key.is_empty() {
        req = req.header("authorization", format!("Bearer {}", ep.key));
    }
    let res = req.send().await.map_err(|e| EmbedError::passing(format!("{shown}: {e}")))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| EmbedError::passing(e.to_string()))?;
    if !status.is_success() {
        let detail: String = text.chars().take(300).collect();
        let refused = matches!(status.as_u16(), 400 | 413 | 422);
        return Err(EmbedError { message: format!("{shown}: {status} {detail}"), permanent: refused });
    }
    parse(&text, texts.len()).map_err(|message| EmbedError::passing(format!("{shown}: {message}")))
}

/// Read `{data: [{index, embedding}]}`, putting vectors back in input order.
fn parse(text: &str, expected: usize) -> Result<Vec<Vec<f32>>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let data = v.get("data").and_then(Value::as_array).ok_or("no data in the reply")?;
    let mut out: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (i, d) in data.iter().enumerate() {
        let at = d.get("index").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(i);
        let vec: Vec<f32> = d
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or("an item has no embedding (is the encoding float?)")?
            .iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect();
        if let Some(slot) = out.get_mut(at) {
            *slot = Some(vec);
        }
    }
    let vecs: Vec<Vec<f32>> = out.into_iter().collect::<Option<_>>().ok_or("fewer vectors than texts")?;
    if vecs.iter().any(Vec::is_empty) || vecs.windows(2).any(|w| w[0].len() != w[1].len()) {
        return Err("vectors of different or zero length".to_string());
    }
    Ok(vecs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_openai_shaped_replies_in_input_order() {
        let r = r#"{"object":"list","data":[{"index":1,"embedding":[0.5,0.5]},{"index":0,"embedding":[1,0]}],"model":"m"}"#;
        assert_eq!(parse(r, 2).unwrap(), vec![vec![1.0, 0.0], vec![0.5, 0.5]]);
        assert!(parse(r, 3).is_err());
        assert!(parse(r#"{"error":"nope"}"#, 1).is_err());
    }

    #[test]
    fn rate_limit_spaces_requests() {
        assert_eq!(spacing(120), Duration::from_millis(500));
        assert_eq!(spacing(0), Duration::ZERO);
        assert_eq!(spacing(1), Duration::from_secs(60));
    }

    #[test]
    fn nothing_secret_is_printable() {
        let ep = Endpoint { url: "https://me:pw@api.example.com/v1?api-key=sk-secret".into(), model: "m".into(), key: "sk-secret".into(), dims: Some(256) };
        let shown = format!("{ep:?}");
        assert!(!shown.contains("sk-secret"), "neither the key nor one in the address: {shown}");
        assert!(!shown.contains("pw@"), "nor a password in the address: {shown}");
        assert!(shown.contains("hidden") && shown.contains("api.example.com"), "{shown}");
        assert!(format!("{:?}", Endpoint::default()).contains("unset"), "no key set is worth saying");

        assert_eq!(redact_url("https://api.example.com/v1/embeddings"), "https://api.example.com/v1/embeddings", "a plain address is left alone");
        assert_eq!(redact_url("https://api.example.com/v1/embeddings?api-key=sk"), "https://api.example.com/v1/embeddings?…");
        assert_eq!(redact_url("https://me:pw@api.example.com/v1"), "https://…@api.example.com/v1");
        assert_eq!(redact_url("http://localhost:1234/v1/@scope/x"), "http://localhost:1234/v1/@scope/x", "an @ in the path is not a password");
    }

    #[test]
    fn signature_follows_the_space() {
        let a = Endpoint { url: "https://x/v1/".into(), model: "m".into(), key: "k1".into(), dims: None };
        let b = Endpoint { key: "k2".into(), url: "https://x/v1".into(), ..a.clone() };
        assert_eq!(a.signature(), b.signature(), "the key and a trailing slash do not change the space");
        assert_ne!(a.signature(), Endpoint { dims: Some(256), ..a.clone() }.signature());
    }
}
