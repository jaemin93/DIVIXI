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
//! # A name and its value are not always one word
//!
//! `password=x` is one word and `password: x` is two, and the second is how
//! YAML, pretty-printed JSON and a derived `Debug` all write it. So a name
//! whose value has not arrived yet does not hide anything — it sets
//! [`Expect`] and the *next* word goes instead. Inserting `[hidden]` after
//! the name and leaving the value in place, which is what this module did
//! before, is worse than doing nothing: the line then reads as though the
//! masking had worked.
//!
//! # What it does not catch
//!
//! - A secret that looks like nothing: a short, lowercase, wordlike value
//!   with no name beside it and no vendor prefix is indistinguishable from
//!   ordinary text, and guessing would hide the log instead of the secret.
//! - A long run of lowercase hex is left alone **on purpose**. Run, track,
//!   session and device ids are uuids, and commits and file digests are
//!   hex; they are in nearly every line, and a log without them cannot be
//!   followed. A token that mixes case and digits is a token; `0f3a…` is an
//!   id.
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
/// case, anywhere in the word, and only when a `=` or `:` follows — in this
/// word or the next.
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
/// alone is enough.
///
/// Thirty-two is base64 of twenty-four bytes, and short enough to catch a
/// secret nobody named. It cannot be the defence for an AWS secret access
/// key: the published example is forty characters *including two slashes*,
/// and a slash ends a run here (or every long path would read as one), so
/// no threshold reaches it. The name beside it is what catches that one.
const RUN: usize = 32;

/// What the word after this one is, as far as the word before it said.
///
/// Secrets arrive in three shapes: inside one word (`password=x`), one word
/// after the name (`password: x`), and two words after it (`token = x`).
/// This is how the second and third are followed without looking ahead.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Nothing in particular: the word speaks for itself.
    Nothing,
    /// A lone separator, and then the value.
    Separator,
    /// The value itself.
    Value,
}

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
    let mut expect = Expect::Nothing;
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
        let (hide, next) = decide(word, expect);
        expect = next;
        if let Some(at) = hide {
            let out = out.get_or_insert_with(String::new);
            out.push_str(&text[kept..start + at]);
            out.push_str(HIDDEN);
            kept = start + word.len();
        }
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

/// Where this word stops being safe to write (everything from that byte on
/// is replaced), and what the word after it will mean.
fn decide(word: &str, expect: Expect) -> (Option<usize>, Expect) {
    match expect {
        Expect::Value => {
            // `Authorization: Bearer <token>`: the scheme is not the secret,
            // the word after it is. Hiding both is right either way.
            let next = if is_scheme(word) { Expect::Value } else { Expect::Nothing };
            return (Some(0), next);
        }
        Expect::Separator if is_separator(word) => return (None, Expect::Value),
        // A name with no separator after it after all: this word is ordinary.
        _ => {}
    }
    if is_scheme(word) {
        return (None, Expect::Value);
    }
    match named_value(word) {
        // The name and its separator are here, the value is not.
        Some(at) if at == word.len() => (None, Expect::Value),
        Some(at) => (Some(at), Expect::Nothing),
        None if holds_name(word) => (None, Expect::Separator),
        None => (shaped(word), Expect::Nothing),
    }
}

fn is_scheme(word: &str) -> bool {
    SCHEMES.iter().any(|s| word.eq_ignore_ascii_case(s))
}

/// A word that is nothing but the punctuation between a name and its value.
fn is_separator(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| matches!(c, '=' | ':' | '"' | '\'' | '>'))
}

/// Hidden for its shape rather than for what it is called: a vendor's own
/// prefix, a JWT, or a run long and mixed enough to be nothing else.
fn shaped(word: &str) -> Option<usize> {
    // A prefix at the start of the word or of a value inside it
    // (`{"api_key":"sk-…"}`). Not just anywhere in the word: `sk-` turns up
    // inside `task-list` and a path is not a secret.
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
    longest_run(word)
}

/// Where the value of a secret-sounding name begins, if the word holds one.
///
/// `word.len()` means the name and its separator are both here but the value
/// is not — it is in the next word, and the caller hides that instead of
/// inserting anything here.
///
/// The `=` or `:` has to come after the name, or `/docs/api-key` would hide
/// the end of its own path.
fn named_value(word: &str) -> Option<usize> {
    if !word.bytes().any(|b| b == b'=' || b == b':') {
        return None;
    }
    let after = name_ends(word)?;
    word[after..].find(['=', ':']).map(|sep| after + sep + 1)
}

/// A secret-sounding name with no separator in the word at all: the
/// separator, if one comes, is the next word (`token = x`).
fn holds_name(word: &str) -> bool {
    name_ends(word).is_some()
}

/// Where the first of [`NAMES`] in this word ends.
fn name_ends(word: &str) -> Option<usize> {
    NAMES.iter().filter_map(|n| find_ignore_case(word, n).map(|at| at + n.len())).min()
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
/// something other than what it is. `None` when there were none, which
/// leaves the common line untouched.
///
/// Every line that is pure ASCII is left alone without looking at it twice,
/// which is nearly all of them. A line with any other byte in it — Korean,
/// an `…`, a path from someone's home directory — is filtered once and
/// handed back unchanged unless something actually came out. That is one
/// allocation more than a list of suspicious byte pairs would cost, and it
/// cannot go stale as the set below grows, which that list did: it named
/// three pairs and three of these characters walked past it.
fn strip_invisible(line: &str) -> Option<String> {
    if line.is_ascii() {
        return None;
    }
    let out: String = line.chars().filter(|c| !is_invisible(*c)).collect();
    (out.len() != line.len()).then_some(out)
}

/// Characters that take up no space of their own: Unicode's format
/// characters (category `Cf` — the zero widths, the bidi controls, the
/// tags), the marks that render as nothing (`Mn`: the variation selectors
/// and the combining grapheme joiner), and the Mongolian selectors. Enough
/// of them to break `sk-` into something this module does not recognise
/// while a terminal still shows `sk-`.
///
/// Not the C0 controls: a tab or a newline in an agent's output is text,
/// and cutting them would change the line for the reader without hiding
/// anything.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}'                      // soft hyphen (Cf)
        | '\u{034F}'                    // combining grapheme joiner (Mn)
        | '\u{061C}'                    // Arabic letter mark (Cf)
        | '\u{180B}'..='\u{180F}'       // Mongolian selectors, vowel separator
        | '\u{200B}'..='\u{200F}'       // zero widths, LRM and RLM
        | '\u{202A}'..='\u{202E}'       // bidi embeddings and overrides
        | '\u{2060}'..='\u{2064}'       // word joiner, the invisible operators
        | '\u{2066}'..='\u{2069}'       // bidi isolates
        | '\u{FE00}'..='\u{FE0F}'       // variation selectors (Mn)
        | '\u{FEFF}'                    // byte-order mark
        | '\u{FFF9}'..='\u{FFFB}'       // interlinear annotation
        | '\u{1D173}'..='\u{1D17A}'     // musical format controls
        | '\u{E0000}'..='\u{E007F}'     // tags, and the deprecated language tags
        | '\u{E0100}'..='\u{E01EF}'     // variation selectors supplement
    )
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
        // A JWT outside any scheme: its own header gives it away.
        assert_eq!(s("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln"), "[hidden]");
    }

    /// The shapes a name and its value arrive in. `password=x` is one word,
    /// `password: x` is two and `token = x` is three, and the last two are
    /// how YAML, pretty JSON and a derived `Debug` all write it.
    #[test]
    fn a_named_value_is_hidden_wherever_the_value_sits() {
        // In the word.
        assert_eq!(s(r#"token="0123456789abcdef""#), "token=[hidden]");
        assert_eq!(s("refresh_token=abcdef p=2"), "refresh_token=[hidden] p=2");
        assert_eq!(s(r#"{"api_key":"SECRET","region":"eu-west-1"}"#), r#"{"api_key":[hidden],"region":"eu-west-1"}"#);
        // One word along.
        assert_eq!(s("password: hunter2"), "password: [hidden]");
        assert_eq!(s(r#""api_key": "x""#), r#""api_key": [hidden]"#);
        assert_eq!(s(r#"Conf { token: "hunter2", url: "x" }"#), r#"Conf { token: [hidden], url: "x" }"#);
        assert_eq!(s(r#"{"api_key": "hunter2","region":"eu-west-1"}"#), r#"{"api_key": [hidden],"region":"eu-west-1"}"#, "and not the field beside it");
        // Two words along.
        assert_eq!(s("token = hunter2"), "token = [hidden]");
        // The whole header value, scheme and all.
        assert_eq!(s("Authorization: Bearer abc123DEF456"), "Authorization: [hidden] [hidden]");
        assert_eq!(s("Authorization Bearer abc123DEF456"), "Authorization Bearer [hidden]");
        // A name with nothing after it hides nothing, and says nothing.
        assert_eq!(s("password:"), "password:");
        assert_eq!(s("the token expired an hour ago"), "the token expired an hour ago");
        // The name has to come before the separator, or a path hides itself.
        assert_eq!(s("https://example.com/docs/api-key"), "https://example.com/docs/api-key");
    }

    /// AWS's own published example of a secret access key, in the form an
    /// `export` line or a YAML file puts it in. No vendor prefix, and the
    /// canonical spelling has slashes in it, so no run is ever long enough:
    /// the name beside it is the only thing that catches this one.
    #[test]
    fn an_aws_secret_is_hidden_by_its_name() {
        for secret in ["wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"] {
            let line = format!("AWS_SECRET_ACCESS_KEY: {secret}");
            assert_eq!(s(&line), "AWS_SECRET_ACCESS_KEY: [hidden]", "{line}");
            assert_eq!(s(&format!("export AWS_SECRET_ACCESS_KEY={secret}")), "export AWS_SECRET_ACCESS_KEY=[hidden]");
        }
        assert_eq!("wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY".len(), 38, "shorter than any run threshold could be");
    }

    #[test]
    fn a_long_mixed_run_is_hidden_whatever_it_is_called() {
        let token = "aB3dEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEf";
        assert_eq!(token.len(), 42);
        assert_eq!(s(&format!("opaque {token} end")), "opaque [hidden] end");
        assert_eq!(s(&format!("hash={token}")), "hash=[hidden]", "the name is kept, the value is not");
        // Base64 of twenty-four bytes is the shortest shape worth hiding.
        assert_eq!(s("opaque aB3dEfGhIjKlMnOpQrStUvWx01234567 end"), "opaque [hidden] end");
    }

    #[test]
    fn a_pem_block_takes_the_rest_of_the_line_with_it() {
        assert_eq!(s("read -----BEGIN RSA PRIVATE KEY----- MIIEpQ base64 tail"), "read [hidden]");
    }

    /// A character that takes up no space can break `sk-` into something
    /// this module does not recognise while a terminal still shows `sk-`.
    #[test]
    fn invisible_characters_cannot_smuggle_a_token_past() {
        for (what, invisible) in [
            ("zero width space", '\u{200B}'),
            ("soft hyphen", '\u{00AD}'),
            ("bidi isolate", '\u{2066}'),
            ("variation selector", '\u{FE0F}'),
            ("combining grapheme joiner", '\u{034F}'),
            ("byte-order mark", '\u{FEFF}'),
        ] {
            let line = format!("sk{invisible}-liveAAAABBBBCCCC");
            assert_eq!(s(&line), "sk-[hidden]", "{what} went past");
        }
        assert_eq!(scrub("경고: 파일을 옮기지 못했습니다"), None, "korean text comes back untouched");
        assert_eq!(scrub("url=https://x/v1?…"), None, "and so does a line with an ellipsis in it");
    }

    #[test]
    fn the_lines_this_app_actually_logs_are_left_alone() {
        for line in [
            "2026-09-27T09:00:00.123456Z  INFO orchestra_app: opening event store",
            r"path=C:\Users\someone\AppData\Roaming\app.divixi\divixi.db",
            "run=019a2f4c8b7d4e2f9a1b3c5d7e9f0a1b track=tr003 worker=wk12",
            "sig=0f3a9c2e7b1d4856 file=divixi.2026-09-27.log",
            "session=c3d4e5f60718293a4b5c6d7e8f901234 replayed=12 kept=3",
            "commit=9998131f0a4c7b2e8d5a6f3b1c0e9d8a7b6c5d4e branch=main",
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
