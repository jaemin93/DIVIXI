//! Full-text search that works for Korean, Japanese and Chinese, after Kiro
//! Crew's `_sqlite_compat`: indexed text has every CJK character as its own
//! token, and a CJK word in a query matches as any of its adjacent pairs, so
//! "지식을" finds "지식" without a morphological analyser.

/// Characters of scripts written without spaces between words (and Hangul,
/// whose words carry particles).
pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Text as it goes into the index: a space around every CJK character.
pub fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    for ch in text.chars() {
        if is_cjk(ch) {
            out.push(' ');
            out.push(ch);
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "about", "be", "been", "by", "do", "does", "did", "for", "from", "how", "in", "into", "is",
    "it", "its", "of", "on", "or", "related", "that", "the", "their", "them", "then", "there", "these", "this", "those", "to",
    "was", "were", "what", "when", "where", "which", "who", "why", "will", "with", "i", "we", "you",
];

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Runs of `token` that are CJK and runs that are not, in order.
fn runs(token: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for ch in token.chars() {
        let cjk = is_cjk(ch);
        match out.last_mut() {
            Some((s, c)) if *c == cjk => s.push(ch),
            _ => out.push((ch.to_string(), cjk)),
        }
    }
    out
}

/// A query as an FTS5 expression that cannot fail to parse: stopwords
/// dropped, each word a quoted phrase (a CJK word any of its adjacent pairs),
/// the words OR-ed for recall. `None` when nothing is left to match.
pub fn match_query(query: &str) -> Option<String> {
    let raw: Vec<&str> = query.split_whitespace().collect();
    let kept: Vec<&str> = raw.iter().copied().filter(|t| !STOPWORDS.contains(&t.to_lowercase().as_str())).collect();
    let tokens = if kept.is_empty() { raw } else { kept };
    let mut groups = Vec::new();
    for token in tokens {
        let mut parts = Vec::new();
        for (run, cjk) in runs(token) {
            if !cjk {
                if run.chars().any(char::is_alphanumeric) {
                    parts.push(quote(&run));
                }
                continue;
            }
            let chars: Vec<char> = run.chars().collect();
            if chars.len() == 1 {
                parts.push(quote(&run));
            } else {
                let pairs: Vec<String> = chars.windows(2).map(|w| quote(&format!("{} {}", w[0], w[1]))).collect();
                parts.push(if pairs.len() == 1 { pairs[0].clone() } else { format!("({})", pairs.join(" OR ")) });
            }
        }
        match parts.len() {
            0 => {}
            1 => groups.push(parts.remove(0)),
            _ => groups.push(format!("({})", parts.join(" AND "))),
        }
    }
    (!groups.is_empty()).then(|| groups.join(" OR "))
}

/// Entity-name candidates in a query: its words, adjacent word pairs, and
/// the substrings of CJK words (longest first), so "지식그래프를" can reach
/// an entity named "지식그래프".
pub fn entity_candidates(query: &str) -> Vec<String> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut out: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    for pair in words.windows(2) {
        out.push(format!("{} {}", pair[0], pair[1]));
    }
    for word in &words {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < 2 || !chars.iter().any(|c| is_cjk(*c)) {
            continue;
        }
        let mut n = 0;
        'sizes: for size in (2..=chars.len().min(6)).rev() {
            for start in 0..=chars.len() - size {
                let piece = &chars[start..start + size];
                if piece.len() == chars.len() || !piece.iter().all(|c| is_cjk(*c)) {
                    continue;
                }
                out.push(piece.iter().collect());
                n += 1;
                if n >= 24 {
                    break 'sizes;
                }
            }
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_cjk() {
        assert_eq!(segment("Rust 지식"), "Rust  지  식 ");
    }

    #[test]
    fn builds_queries() {
        assert_eq!(match_query("what is the borrow checker").as_deref(), Some(r#""borrow" OR "checker""#));
        assert_eq!(match_query("지식을").as_deref(), Some(r#"("지 식" OR "식 을")"#));
        assert_eq!(match_query("Rust가").as_deref(), Some(r#"("Rust" AND "가")"#));
        assert_eq!(match_query("the").as_deref(), Some(r#""the""#));
        assert_eq!(match_query(r#"say "hi""#).as_deref(), Some(r#""say" OR """hi""""#));
        assert!(match_query("  ... ").is_none());
    }

    #[test]
    fn candidates_reach_inside_cjk_words() {
        let c = entity_candidates("지식그래프를 보자");
        assert!(c.contains(&"지식그래프".to_string()));
        assert!(c.contains(&"지식그래프를 보자".to_string()));
    }
}
