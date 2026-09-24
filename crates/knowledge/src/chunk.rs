//! Splitting a document into chunks the size of one extraction call, after
//! Kiro Crew's `HeadingAwareChunker`: markdown on headings, code on
//! function and class boundaries, anything else on blank lines, then lines,
//! then spaces. Sizes are in approximate tokens (a word is 1.3).

/// Target chunk size, in approximate tokens.
pub const CHUNK_TOKENS: usize = 800;
/// Words of the previous chunk repeated at the start of the next, in tokens.
pub const CHUNK_OVERLAP: usize = 200;
/// A file never yields more chunks than this; the rest is not indexed.
pub const MAX_CHUNKS: usize = 50;

const SEPARATORS: [&str; 3] = ["\n\n", "\n", " "];

/// One piece of a document and where it sits in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub content: String,
    /// The markdown heading the chunk starts under, if any.
    pub section: Option<String>,
    pub index: usize,
    /// 1-based source lines the chunk covers.
    pub line_start: usize,
    pub line_end: usize,
}

/// Approximate tokens: 1.3 per word, and never fewer than a sixth of the
/// characters, so text written without spaces is not one giant "word".
pub fn tokens(text: &str) -> usize {
    let words = text.split_whitespace().count() * 13 / 10;
    words.max(text.chars().count() / 6)
}

/// How a file of this extension is split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Markdown,
    Code,
    Text,
}

/// Chunk a document the way its extension suggests.
pub fn chunk_document(shape: Shape, text: &str) -> Vec<Chunk> {
    let chunker = Chunker::default();
    match shape {
        Shape::Markdown => chunker.markdown(text),
        Shape::Code => chunker.code(text),
        Shape::Text => chunker.text(text),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Chunker {
    pub target: usize,
    pub overlap: usize,
}

impl Default for Chunker {
    fn default() -> Self {
        Self { target: CHUNK_TOKENS, overlap: CHUNK_OVERLAP }
    }
}

impl Chunker {
    /// Plain text: recursive separator splitting, with the previous chunk's
    /// last words carried into the next for context.
    pub fn text(&self, text: &str) -> Vec<Chunk> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        let mut raw = self.split(text, &SEPARATORS);
        raw.truncate(MAX_CHUNKS);
        let mut out = Vec::with_capacity(raw.len());
        let mut line = 1;
        for (i, piece) in raw.iter().enumerate() {
            let line_start = line;
            let line_end = line_start + piece.matches('\n').count();
            line = line_end + 2;
            let mut content = piece.clone();
            if i > 0 && self.overlap > 0 {
                content = format!("{}\n{}", tail(&raw[i - 1], self.overlap), piece);
            }
            out.push(Chunk { content, section: None, index: i, line_start, line_end });
        }
        out
    }

    /// Split on the first separator, merging small pieces and recursing into
    /// pieces still too large with the finer separators.
    fn split(&self, text: &str, seps: &[&str]) -> Vec<String> {
        if tokens(text) <= self.target {
            let t = text.trim();
            return if t.is_empty() { Vec::new() } else { vec![t.to_string()] };
        }
        let Some((sep, rest)) = seps.split_first() else {
            return self.force_split(text);
        };
        let mut chunks = Vec::new();
        let mut current = String::new();
        for piece in text.split(sep) {
            let candidate = if current.is_empty() { piece.to_string() } else { format!("{current}{sep}{piece}") };
            if tokens(&candidate) <= self.target {
                current = candidate;
                continue;
            }
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
            }
            if tokens(piece) > self.target {
                chunks.extend(self.split(piece, rest));
                current.clear();
            } else {
                current = piece.to_string();
            }
        }
        if !current.trim().is_empty() {
            chunks.push(current.trim().to_string());
        }
        chunks
    }

    /// No separator left: cut by words, or by characters when there are none.
    fn force_split(&self, text: &str) -> Vec<String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() > 1 {
            let per = (self.target * 10 / 13).max(1);
            return words.chunks(per).map(|w| w.join(" ")).filter(|c| !c.trim().is_empty()).collect();
        }
        let chars: Vec<char> = text.trim().chars().collect();
        chars.chunks((self.target * 6).max(1)).map(|c| c.iter().collect::<String>()).collect()
    }

    /// Code: blocks start at function and class keywords; small blocks merge,
    /// oversized ones are cut by lines.
    pub fn code(&self, text: &str) -> Vec<Chunk> {
        const STARTS: [&str; 14] = [
            "def ", "class ", "function ", "fn ", "fun ", "func ", "pub fn ", "object ", "interface ", "internal ", "public ",
            "private ", "protected ", "impl ",
        ];
        if text.trim().is_empty() {
            return Vec::new();
        }
        let lines: Vec<&str> = text.split('\n').collect();
        let mut blocks: Vec<(usize, Vec<&str>)> = Vec::new();
        let mut start = 1;
        let mut current: Vec<&str> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if STARTS.iter().any(|s| trimmed.starts_with(s)) && !current.is_empty() {
                blocks.push((start, std::mem::take(&mut current)));
                start = i + 1;
            }
            current.push(line);
        }
        if !current.is_empty() {
            blocks.push((start, current));
        }

        let mut merged: Vec<(usize, Vec<&str>)> = Vec::new();
        for (start, block) in blocks {
            let size = tokens(&block.join("\n"));
            if let Some(last) = merged.last_mut() {
                if tokens(&last.1.join("\n")) + size <= self.target {
                    last.1.extend(block);
                    continue;
                }
            }
            if size <= self.target * 2 {
                merged.push((start, block));
                continue;
            }
            let mut piece: Vec<&str> = Vec::new();
            let mut piece_start = start;
            for line in block {
                piece.push(line);
                if tokens(&piece.join("\n")) >= self.target {
                    let n = piece.len();
                    merged.push((piece_start, std::mem::take(&mut piece)));
                    piece_start += n;
                }
            }
            if !piece.is_empty() {
                merged.push((piece_start, piece));
            }
        }
        merged
            .into_iter()
            .filter(|(_, b)| b.iter().any(|l| !l.trim().is_empty()))
            .take(MAX_CHUNKS)
            .enumerate()
            .map(|(i, (start, block))| Chunk {
                line_end: start + block.len() - 1,
                content: block.join("\n"),
                section: None,
                index: i,
                line_start: start,
            })
            .collect()
    }

    /// Markdown: headings are the natural boundaries. Small sections merge
    /// up to the target size; oversized ones are split like plain text.
    /// Headings inside fenced code blocks do not count.
    pub fn markdown(&self, text: &str) -> Vec<Chunk> {
        let headings = headings(text);
        if headings.is_empty() {
            return self.text(text);
        }
        // (title, first line, body up to the next heading)
        let mut sections: Vec<(Option<String>, usize, &str)> = Vec::new();
        if headings[0].0 > 0 {
            let pre = text[..headings[0].0].trim();
            if !pre.is_empty() {
                sections.push((None, 1, pre));
            }
        }
        for (i, (at, title)) in headings.iter().enumerate() {
            let end = headings.get(i + 1).map(|h| h.0).unwrap_or(text.len());
            let line = text[..*at].matches('\n').count() + 1;
            sections.push((Some(title.clone()), line, &text[*at..end]));
        }

        let end_line = |start: usize, body: &str| start + body.trim_end().matches('\n').count();
        let mut out: Vec<Chunk> = Vec::new();
        let mut title: Option<String> = None;
        let mut body = String::new();
        let mut start = 1;
        let mut end = 1;
        let flush = |out: &mut Vec<Chunk>, title: &Option<String>, body: &str, start: usize, end: usize| {
            if tokens(body) > self.target {
                let mut offset = 0;
                for sub in self.split(body, &SEPARATORS) {
                    let s = start + offset;
                    let lines = sub.matches('\n').count();
                    out.push(Chunk { section: title.clone(), index: out.len(), line_start: s, line_end: s + lines, content: sub });
                    offset += lines + 1;
                }
            } else if !body.trim().is_empty() {
                out.push(Chunk { content: body.trim().to_string(), section: title.clone(), index: out.len(), line_start: start, line_end: end });
            }
        };
        for (t, line, b) in sections {
            if body.is_empty() {
                title = t;
                body = b.to_string();
                start = line;
                end = end_line(line, b);
            } else if tokens(&body) + tokens(b) <= self.target {
                body.push('\n');
                body.push_str(b);
                end = end_line(line, b);
                if title.is_none() {
                    title = t;
                }
            } else {
                flush(&mut out, &title, &body, start, end);
                title = t;
                body = b.to_string();
                start = line;
                end = end_line(line, b);
            }
        }
        if !body.trim().is_empty() {
            flush(&mut out, &title, &body, start, end);
        }
        out.truncate(MAX_CHUNKS);
        out
    }
}

/// The end of `prev` worth about `budget` tokens: its last words, or its
/// last characters when it has no spaces to cut at.
fn tail(prev: &str, budget: usize) -> String {
    let words: Vec<&str> = prev.split_whitespace().collect();
    let keep = (budget * 10 / 13).max(1);
    let t = words[words.len().saturating_sub(keep)..].join(" ");
    if tokens(&t) <= budget {
        return t;
    }
    let chars: Vec<char> = t.chars().collect();
    chars[chars.len().saturating_sub(budget * 6)..].iter().collect()
}

/// Byte offsets and titles of the ATX headings outside code fences.
fn headings(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end_matches(['\n', '\r']);
        let lead = t.trim_start();
        let marker = if lead.starts_with("```") { Some("```") } else if lead.starts_with("~~~") { Some("~~~") } else { None };
        if let Some(m) = marker {
            fence = match fence {
                Some(open) if open == m => None,
                None => Some(m),
                other => other,
            };
        } else if fence.is_none() && t.starts_with('#') {
            let hashes = t.chars().take_while(|c| *c == '#').count();
            let rest = &t[hashes..];
            if (1..=6).contains(&hashes) && rest.starts_with([' ', '\t']) && !rest.trim().is_empty() {
                out.push((at, rest.trim().trim_end_matches('#').trim().to_string()));
            }
        }
        at += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize, w: &str) -> String {
        vec![w; n].join(" ")
    }

    #[test]
    fn short_text_is_one_chunk() {
        let c = Chunker::default().text("hello world\n\nsecond paragraph");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].line_start, 1);
        assert_eq!(c[0].line_end, 3);
    }

    #[test]
    fn long_text_splits_with_overlap() {
        let text = format!("{}\n\n{}", words(500, "alpha"), words(500, "beta"));
        let c = Chunker::default().text(&text);
        assert_eq!(c.len(), 2);
        assert!(c[1].content.starts_with("alpha"), "second chunk carries the first one's tail");
        assert!(c[1].content.ends_with("beta"));
    }

    #[test]
    fn markdown_merges_small_sections_and_keeps_titles() {
        let md = "intro line\n# One\nshort\n## Two\nalso short\n```\n# not a heading\n```\n";
        let c = Chunker::default().markdown(md);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].section.as_deref(), Some("One"));
        assert!(c[0].content.contains("# not a heading"));
        assert_eq!(headings(md).len(), 2);
    }

    #[test]
    fn markdown_keeps_sections_apart_when_together_too_big() {
        let md = format!("# A\n{}\n# B\n{}\n", words(500, "a"), words(500, "b"));
        let c = Chunker::default().markdown(&md);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].section.as_deref(), Some("A"));
        assert_eq!(c[1].section.as_deref(), Some("B"));
        assert_eq!(c[1].line_start, 3);
    }

    #[test]
    fn code_splits_on_functions() {
        let body = |name: &str| format!("fn {name}() {{\n{}\n}}\n", words(500, "x;"));
        let code = format!("use a;\n{}{}", body("one"), body("two"));
        let c = Chunker::default().code(&code);
        assert_eq!(c.len(), 2);
        assert!(c[1].content.starts_with("fn two"));
        assert_eq!(c[0].line_start, 1);
    }

    #[test]
    fn spaceless_text_is_still_cut() {
        let text: String = "가".repeat(10_000);
        let c = Chunker::default().text(&text);
        assert!(c.len() > 1);
        assert!(c.iter().all(|x| tokens(&x.content) <= CHUNK_TOKENS + CHUNK_OVERLAP));
    }

    #[test]
    fn caps_the_chunk_count() {
        let text = (0..200).map(|i| format!("{}\n\n", words(700, &format!("w{i}")))).collect::<String>();
        assert_eq!(Chunker::default().text(&text).len(), MAX_CHUNKS);
    }
}
