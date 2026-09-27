//! The floor under the log: anything token-shaped is hidden before it is
//! written anywhere.
//!
//! The first line of defence is that secrets are never handed to the log at
//! all — the `Debug` impls in embed.rs, remote/auth.rs, remote/github.rs and
//! `AgentSpec` exist for that. This is the floor beneath it, for the text
//! nobody here wrote: `DIVIXI_LOG=debug` records what the agents print on
//! their stderr, and what an agent prints is the agent's business.
//!
//! It runs on the **rendered line**, in the writer every sink is wrapped in
//! ([`Masked`]), rather than on the event: by then a field has been through
//! `Display` or `Debug` and the same pass covers both. A sink added later
//! that is not wrapped is a plain-text hole, which is why there is one
//! wrapper and not one filter per sink.
//!
//! The spelling is deliberately wider than any check that blocks a request:
//! a false positive costs one hidden word in a log, a miss costs a live key
//! on disk. The formats matched are the vendors' own published ones (RFC
//! 6750 for `Bearer`, RFC 7515 for a JWT's segments, AWS's key-id prefixes,
//! GitHub's and Slack's token prefixes, RFC 7468 for a PEM block).
//!
//! # What it does not catch
//!
//! - A secret that looks like nothing: a short, lowercase, wordlike value
//!   with no name beside it and no vendor prefix is indistinguishable from
//!   ordinary text, and guessing would hide the log instead of the secret.
//! - A long run of lowercase hex is left alone **on purpose**. Run, track,
//!   session and device ids are uuids and hashes, they are in nearly every
//!   line, and a log without them cannot be followed. A token that mixes
//!   case and digits is a token; `0f3a…` is an id.
//! - `key` on its own is not a secret name (`key=theme`, a store key, a
//!   keyboard key). The compound spellings below are.

use std::io::Write;

use tracing_subscriber::fmt::writer::MakeWriter;

/// What is left where a secret was.
const HIDDEN: &str = "[hidden]";

/// A PEM block's opening line. Nothing after it on the line is safe: the
/// body is the key, and it is base64 that may or may not reach the
/// thresholds below.
const PEM: &str = "-----BEGIN";

/// Vendor prefixes, kept as the vendors write them (so case matters). What
/// follows the prefix is hidden; the prefix itself is public and says which
/// credential to go and rotate.
const PREFIXES: [&str; 16] = [
    "sk-",          // OpenAI, Anthropic
    "sk_",          // Stripe secret keys
    "rk_",          // Stripe restricted keys
    "ghp_",         // GitHub personal access token (classic)
    "gho_",         // GitHub OAuth
    "ghu_",         // GitHub app user-to-server
    "ghs_",         // GitHub app server-to-server
    "ghr_",         // GitHub refresh
    "github_pat_",  // GitHub fine-grained
    "xoxb-",        // Slack bot
    "xoxp-",        // Slack user
    "xoxa-",        // Slack app
    "xoxs-",        // Slack session
    "npm_",         // npm automation token
    "AIza",         // Google API key
    "glpat-",       // GitLab personal access token
];

/// AWS access key ids: one of these four, then sixteen more characters.
const AWS_PREFIXES: [&str; 4] = ["AKIA", "ASIA", "ABIA", "ACCA"];

/// Names that make whatever follows them a secret. Matched without regard to
/// case, anywhere in the word, and only when a `=` or `:` follows.
const NAMES: [&str; 14] = [
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "api-key",
    "apikey",
    "authorization",
    "cookie",
    "private_key",
    "credential",
    "access_key",
    "session_key",
    "client_secret",
];

/// A word after one of these is the credential itself (RFC 6750 and RFC 7617).
const SCHEMES: [&str; 2] = ["bearer", "basic"];

/// How long an unbroken run of token characters has to be before its shape
/// alone is enough. Forty is a GitHub classic token's length and shorter
/// than any base64 secret worth stealing.
const RUN: usize = 40;


/// Hide every secret-shaped run in one rendered log line.
///
/// `None` means the line is already clean, which is nearly every line: the
/// caller then writes the bytes it already has and nothing is allocated.
///
/// Only lines the level filter let through reach this, so at the default
/// level a session scans a handful of lines; at `debug` it is one pass of
/// cheap byte comparisons per line, and the costly part (looking for the
/// names above) needs a `=` or `:` in the word before it runs at all.
pub fn scrub(line: &str) -> Option<String> {
    // Invisible characters come out first. Doing it afterwards would be
    // worse than not doing it: `s<zero width space>k-live…` would pass the
    // scan and then be written in a form a terminal shows as `sk-live…`.
    let stripped = strip_invisible(line);
    let text: &str = stripped.as_deref().unwrap_or(line);

    let mut out: Option<String> = None;
    // How much of `text` has been copied into `out` already.
    let mut kept = 0usize;
    let mut previous = "";
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_break(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_break(bytes[i]) {
            i += 1;
        }
        let word = &text[start..i];
        if word.starts_with(PEM) {
            let out = out.get_or_insert_with(String::new);
            out.push_str(&text[kept..start]);
            out.push_str(HIDDEN);
            kept = text.len();
            break;
        }
        if let Some(at) = hide_from(word, previous) {
            let out = out.get_or_insert_with(String::new);
            out.push_str(&text[kept..start + at]);
            out.push_str(HIDDEN);
            kept = start + word.len();
        }
        previous = word;
    }

    match out {
        Some(mut out) => {
            out.push_str(&text[kept..]);
            Some(out)
        }
        // Nothing was token-shaped, but invisible characters were dropped.
        None => stripped,
    }
}

/// Where a word stops being safe to write, if it does: everything from that
/// byte on is replaced.
fn hide_from(word: &str, previous: &str) -> Option<usize> {
    // `Authorization: Bearer <this>`.
    if SCHEMES.iter().any(|s| previous.eq_ignore_ascii_case(s)) {
        return Some(0);
    }
    // A named value, first because it hides the most: everything from the
    // separator to the end of the word.
    if let Some(at) = named_value(word) {
        return Some(at);
    }
    // A vendor's own prefix, at the start of the word or of a value inside
    // it (`{"api_key":"sk-…"}`). Not just anywhere in the word: `sk-` turns
    // up inside `task-list` and a path is not a secret.
    for at in value_starts(word) {
        let rest = &word[at..];
        if let Some(prefix) = PREFIXES.iter().find(|p| rest.starts_with(**p)) {
            if rest.len() >= prefix.len() + 4 {
                return Some(at + prefix.len());
            }
        }
        if rest.len() >= 20 && AWS_PREFIXES.iter().any(|p| rest.starts_with(p)) {
            return Some(at + 4);
        }
        // A JWT or JWE: the `{"` of its header, base64url'd, then its segments.
        if rest.len() >= 20 && rest.starts_with("eyJ") && rest.contains('.') {
            return Some(at);
        }
    }
    // Shape alone: a long run that mixes case and digits the way base64 and
    // vendor tokens do, and ids and hashes in this log do not.
    longest_run(word)
}

/// Where the value of a secret-sounding name begins, if the word holds one.
///
/// The `=` or `:` has to come after the name, or `/docs/api-key` would hide
/// the end of its own path.
fn named_value(word: &str) -> Option<usize> {
    if !word.bytes().any(|b| b == b'=' || b == b':') {
        return None;
    }
    let after = NAMES.iter().filter_map(|n| find_ignore_case(word, n).map(|at| at + n.len())).min()?;
    word[after..].find(['=', ':']).map(|sep| after + sep + 1)
}

/// Every offset a value can start at inside a word: its start, and whatever
/// follows a separator or a quote.
fn value_starts(word: &str) -> impl Iterator<Item = usize> + '_ {
    let inner = word.bytes().enumerate().filter(|(_, b)| matches!(b, b'=' | b':' | b'"' | b'\'')).map(|(at, _)| at + 1);
    std::iter::once(0).chain(inner)
}

/// The start of the first run of token characters that is long enough and
/// mixed enough to be a secret whatever it is called.
fn longest_run(word: &str) -> Option<usize> {
    let bytes = word.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !is_run(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let (mut upper, mut lower, mut digit) = (false, false, false);
        while i < bytes.len() && is_run(bytes[i]) {
            upper |= bytes[i].is_ascii_uppercase();
            lower |= bytes[i].is_ascii_lowercase();
            digit |= bytes[i].is_ascii_digit();
            i += 1;
        }
        if i - start >= RUN && upper && lower && digit {
            return Some(start);
        }
    }
    None
}

/// Word characters. Structure — whitespace, and the punctuation JSON and
/// tracing's own field format put between values — ends a word, so hiding
/// one value cannot swallow the next one. Quotes stay inside a word: a
/// `Debug` field arrives as `token="…"` and the quoted part is the secret.
fn is_break(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b',' | b';' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'<' | b'>' | b'|' | b'`')
}

/// Characters an unbroken token run is made of: base64, base64url and hex.
/// Not `/` or `\`, or a long path would read as one run; not `=`, so that
/// `name=<token>` keeps its name and base64 padding ends a run rather than
/// extending it.
fn is_run(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+')
}

/// Where `needle` (which must be ASCII lowercase) sits in `haystack`,
/// whatever case it is written in. No allocation: the alternative is
/// lowercasing every word of every line.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&at| h[at..at + n.len()].eq_ignore_ascii_case(n))
}

/// The line without the characters that are there to make text read as
/// something other than what it is: zero widths, the bidi overrides, the
/// word joiners, a byte-order mark. `None` when there were none, which
/// leaves the common line untouched.
///
/// All of them start `E2 80`, `E2 81` or `EF BB` in UTF-8, so a line of
/// Korean never reaches the slow path; a line with `…` in it (`E2 80 A6`)
/// does, finds nothing, and pays for one copy.
fn strip_invisible(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let suspicious = bytes.windows(2).any(|w| (w[0] == 0xE2 && (w[1] == 0x80 || w[1] == 0x81)) || (w[0] == 0xEF && w[1] == 0xBB));
    if !suspicious {
        return None;
    }
    let out: String = line.chars().filter(|c| !is_invisible(*c)).collect();
    (out.len() != line.len()).then_some(out)
}

fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// A `MakeWriter` whose lines go through [`scrub`] on the way out.
///
/// `fmt::Layer` renders one whole event into its buffer and writes it in a
/// single call, so each write is one complete line to scan.
pub struct Masked<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Masked<M> {
    type Writer = MaskedWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        MaskedWriter(self.0.make_writer())
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        MaskedWriter(self.0.make_writer_for(meta))
    }
}

pub struct MaskedWriter<W>(W);

impl<W: Write> Write for MaskedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Not UTF-8 at all: written as it came rather than dropped. Every
        // line tracing renders is UTF-8, so this is the impossible branch.
        let Ok(line) = std::str::from_utf8(buf) else { return self.0.write(buf) };
        match scrub(line) {
            // The whole buffer was taken, whatever its masked length is.
            Some(clean) => self.0.write_all(clean.as_bytes()).map(|()| buf.len()),
            None => self.0.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a scrubbed line looks like, for the assertions below.
    fn s(line: &str) -> String {
        scrub(line).unwrap_or_else(|| line.to_string())
    }

    #[test]
    fn vendor_tokens_are_hidden_and_say_which_vendor() {
        assert_eq!(s("key=sk-ant-api03-AAAABBBBCCCCDDDD"), "key=sk-[hidden]");
        assert_eq!(s("ghp_0123456789abcdefghijABCDEFGHIJ0123"), "ghp_[hidden]");
        assert_eq!(s("github_pat_11ABCDE0000aaaaBBBBccccDD"), "github_pat_[hidden]");
        assert_eq!(s("xoxb-123456789012-abcdefABCDEF"), "xoxb-[hidden]");
        assert_eq!(s("AKIAIOSFODNN7EXAMPLE and the rest"), "AKIA[hidden] and the rest");
        assert_eq!(s("AIzaSyA0000000000000000000000000000000"), "AIza[hidden]");
        // RFC 6750, and the scheme may be written in any case.
        assert_eq!(s("authorization: bearer abc123DEF456"), "authorization:[hidden] bearer [hidden]");
        assert_eq!(s("Authorization Bearer abc123DEF456"), "Authorization Bearer [hidden]");
        // A JWT outside any scheme: its own header gives it away.
        assert_eq!(s("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln"), "[hidden]");
    }

    #[test]
    fn a_named_value_is_hidden_without_taking_the_next_one() {
        assert_eq!(s(r#"token="0123456789abcdef""#), "token=[hidden]");
        assert_eq!(s("refresh_token=abcdef p=2"), "refresh_token=[hidden] p=2");
        assert_eq!(s(r#"{"api_key":"SECRET","region":"eu-west-1"}"#), r#"{"api_key":[hidden],"region":"eu-west-1"}"#);
        assert_eq!(s("password=hunter2"), "password=[hidden]");
        assert_eq!(s("https://api.example.com/v1?api-key=sk-live-xyz"), "https://api.example.com/v1?api-key=[hidden]");
        // The name has to come before the separator, or a path hides itself.
        assert_eq!(s("https://example.com/docs/api-key"), "https://example.com/docs/api-key");
    }

    #[test]
    fn a_long_mixed_run_is_hidden_whatever_it_is_called() {
        let token = "aB3dEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEf";
        assert_eq!(token.len(), 42);
        assert_eq!(s(&format!("opaque {token} end")), "opaque [hidden] end");
        assert_eq!(s(&format!("hash={token}")), "hash=[hidden]", "the name is kept, the value is not");
    }

    #[test]
    fn a_pem_block_takes_the_rest_of_the_line_with_it() {
        assert_eq!(s("read -----BEGIN RSA PRIVATE KEY----- MIIEpQ base64 tail"), "read [hidden]");
    }

    #[test]
    fn invisible_characters_cannot_smuggle_a_token_past() {
        assert_eq!(s("sk\u{200B}-liveAAAABBBBCCCC"), "sk-[hidden]");
        assert_eq!(scrub("경고: 파일을 옮기지 못했습니다"), None, "korean text is not suspicious");
        assert_eq!(scrub("url=https://x/v1?…"), None, "an ellipsis is looked at and left alone");
    }

    #[test]
    fn the_lines_this_app_actually_logs_are_left_alone() {
        for line in [
            "2026-09-27T09:00:00.123456Z  INFO orchestra_app: opening event store",
            r"path=C:\Users\someone\AppData\Roaming\app.divixi\divixi.db",
            "run=019a2f4c8b7d4e2f9a1b3c5d7e9f0a1b track=tr003 worker=wk12",
            "sig=0f3a9c2e7b1d4856 file=divixi.2026-09-27.log",
            "session=c3d4e5f60718293a4b5c6d7e8f901234 replayed=12 kept=3",
            "program=node args=[\"node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js\"]",
            "mcp=http://127.0.0.1:52344/mcp resume=None",
            "client_id=Iv1.0123456789abcdef login=someone",
            "key=theme value=night",
            "AgentSpec { program: \"claude\", args: [\"--acp\"], env: [\"ANTHROPIC_API_KEY\"] }",
        ] {
            assert_eq!(scrub(line), None, "nothing to hide in: {line}");
        }
    }

    #[test]
    fn the_writer_masks_what_reaches_a_sink() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let sink = Sink::default();
        let mut writer = MaskedWriter(sink.clone());
        let line = "WARN agent_stderr: export ANTHROPIC_API_KEY=sk-ant-0123456789\n";
        assert_eq!(writer.write(line.as_bytes()).unwrap(), line.len(), "the caller is told the whole line was taken");
        let written = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert_eq!(written, "WARN agent_stderr: export ANTHROPIC_API_KEY=[hidden]\n");
    }
}
