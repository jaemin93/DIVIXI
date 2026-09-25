//! Text out of documents that are not text: PDF (its text layer) and Word
//! (.docx, with its headings kept as markdown headings).

use std::io::Read;

/// The text of a PDF, page by page. A scanned PDF (pictures only) has
/// none, which is an error: there is nothing to index.
pub fn pdf_text(bytes: &[u8]) -> anyhow::Result<String> {
    // The PDF parser panics on some malformed files; a bad file must not
    // take the sync loop down with it.
    let text = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes))
        .map_err(|_| anyhow::anyhow!("the PDF could not be read"))?
        .map_err(|e| anyhow::anyhow!("the PDF could not be read: {e}"))?;
    let text = tidy(&text);
    if text.trim().is_empty() {
        anyhow::bail!("the PDF has no text layer (a scan?)");
    }
    Ok(text)
}

/// The text of a .docx: one paragraph per line, blank lines between, and
/// paragraphs in a heading style as markdown headings.
pub fn docx_text(bytes: &[u8]) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("not a Word document: {e}"))?;
    let mut xml = String::new();
    {
        let mut part = archive.by_name("word/document.xml").map_err(|_| anyhow::anyhow!("not a Word document"))?;
        // A document's XML beyond 64 MB is not one worth indexing.
        part.by_ref().take(64 * 1024 * 1024).read_to_string(&mut xml)?;
    }
    let text = docx_xml_text(&xml);
    if text.trim().is_empty() {
        anyhow::bail!("the document has no text");
    }
    Ok(text)
}

/// Paragraphs of WordprocessingML, headings marked.
fn docx_xml_text(xml: &str) -> String {
    let mut out = String::new();
    // Each <w:p …>…</w:p> is a paragraph; the paragraph's own properties say its style.
    let mut rest = xml;
    while let Some(start) = find_tag(rest, "w:p") {
        let after = &rest[start..];
        let end = after.find("</w:p>").map(|e| e + "</w:p>".len()).unwrap_or(after.len());
        let para = &after[..end];
        let text = runs_text(para);
        if !text.trim().is_empty() {
            let level = heading_level(para);
            if level > 0 {
                out.push_str(&"#".repeat(level));
                out.push(' ');
            }
            out.push_str(text.trim());
            out.push_str("\n\n");
        }
        rest = &after[end.min(after.len())..];
        if end == 0 {
            break;
        }
    }
    out.trim_end().to_string() + "\n"
}

/// Where the next `<tag>` or `<tag …>` starts (not `<tagX…>`).
fn find_tag(s: &str, tag: &str) -> Option<usize> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = s[from..].find(&open) {
        let at = from + i;
        match s.as_bytes().get(at + open.len()) {
            Some(b'>') | Some(b' ') | Some(b'/') => return Some(at),
            _ => from = at + open.len(),
        }
    }
    None
}

/// The text runs of a paragraph: `<w:t>` contents, tabs and breaks.
fn runs_text(para: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(lt) = para[i..].find('<') {
        let at = i + lt;
        let Some(gt) = para[at..].find('>') else { break };
        let tag = &para[at + 1..at + gt];
        i = at + gt + 1;
        if tag == "w:tab/" || tag.starts_with("w:tab ") {
            out.push('\t');
        } else if tag == "w:br/" || tag.starts_with("w:br ") || tag == "w:cr/" {
            out.push('\n');
        } else if (tag == "w:t" || tag.starts_with("w:t ")) && !tag.ends_with('/') {
            if let Some(close) = para[i..].find("</w:t>") {
                out.push_str(&unescape(&para[i..i + close]));
                i += close + "</w:t>".len();
            }
        }
    }
    out
}

/// A paragraph's heading level from its style (Heading1…Heading6, Title), else 0.
fn heading_level(para: &str) -> usize {
    let Some(at) = para.find("<w:pStyle ") else { return 0 };
    let tag = &para[at..at + para[at..].find('>').unwrap_or(0)];
    let Some(v) = tag.find("w:val=\"") else { return 0 };
    let val = &tag[v + 7..];
    let val = &val[..val.find('"').unwrap_or(val.len())];
    let lower = val.to_ascii_lowercase();
    if lower == "title" {
        return 1;
    }
    lower
        .strip_prefix("heading")
        .and_then(|n| n.trim().parse::<usize>().ok())
        .filter(|n| (1..=6).contains(n))
        .unwrap_or(0)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Line endings as \n, runs of blank lines folded, trailing spaces dropped.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    // No blank lines left hanging at the end.
    let end = out.trim_end().len();
    out.truncate(end);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn docx(document_xml: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("word/document.xml", opts).unwrap();
            z.write_all(document_xml.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn word_paragraphs_and_headings() {
        let xml = r#"<?xml version="1.0"?><w:document><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>설계 개요</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">Bronze </w:t></w:r><w:r><w:t>&amp; Silver</w:t></w:r><w:r><w:tab/><w:t>tiers</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Gold</w:t></w:r></w:p>
            <w:p/>
            <w:proofErr w:type="spellStart"/><w:p><w:r><w:t>line one</w:t><w:br/><w:t>line two</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = docx_text(&docx(xml)).unwrap();
        assert_eq!(text, "# 설계 개요\n\nBronze & Silver\ttiers\n\n## Gold\n\nline one\nline two\n");
    }

    #[test]
    fn refuses_what_is_not_there() {
        assert!(docx_text(b"not a zip").is_err());
        assert!(docx_text(&docx("<w:document><w:body><w:p/></w:body></w:document>")).is_err());
        assert!(pdf_text(b"%PDF-1.4 garbage").is_err());
    }

    #[test]
    fn tidies_extracted_text() {
        assert_eq!(tidy("a  \r\n\r\n\r\n b\n\n"), "a\n\n b\n");
    }
}
