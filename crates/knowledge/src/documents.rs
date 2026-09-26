//! Text out of documents that are not text: PDF (its text layer), Word
//! (.docx, with its headings kept as markdown headings), PowerPoint
//! (.pptx, slide by slide) and Excel (.xlsx, sheet by sheet, a row a line).

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

/// One part of an Office archive as text (at most 64 MB of it).
fn part(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut xml = String::new();
    let mut p = archive.by_name(name).ok()?;
    p.by_ref().take(64 * 1024 * 1024).read_to_string(&mut xml).ok()?;
    Some(xml)
}

/// Parts named `<prefix><n>.xml`, in the order of n.
fn numbered(archive: &zip::ZipArchive<std::io::Cursor<&[u8]>>, prefix: &str) -> Vec<(u32, String)> {
    let mut found: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|n| n.strip_prefix(prefix)?.strip_suffix(".xml")?.parse::<u32>().ok().map(|i| (i, n.to_string())))
        .collect();
    found.sort();
    found
}

/// The text inside each `<tag>…</tag>` (or `<tag …>…</tag>`) of `s`, unescaped.
fn tag_texts(s: &str, tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let close = format!("</{tag}>");
    let mut rest = s;
    while let Some(start) = find_tag(rest, tag) {
        let after = &rest[start..];
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(unescape(&body[..end]));
        rest = &body[end + close.len()..];
    }
    out
}

/// Each `<tag>…</tag>` element of `s` (self-closing ones as they are).
fn elements<'a>(s: &'a str, tag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let close = format!("</{tag}>");
    let mut rest = s;
    while let Some(start) = find_tag(rest, tag) {
        let after = &rest[start..];
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            out.push(&after[..gt + 1]);
            rest = &after[gt + 1..];
            continue;
        }
        let end = after.find(&close).map(|e| e + close.len()).unwrap_or(after.len());
        out.push(&after[..end]);
        rest = &after[end..];
    }
    out
}

/// An attribute's value in an element's opening tag.
fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let open = &element[..element.find('>').unwrap_or(element.len())];
    let key = format!(" {name}=\"");
    let at = open.find(&key)? + key.len();
    let rest = &open[at..];
    Some(&rest[..rest.find('"')?])
}

/// The text of a .pptx: each slide's paragraphs under a "Slide n" heading.
pub fn pptx_text(bytes: &[u8]) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("not a PowerPoint file: {e}"))?;
    let slides = numbered(&archive, "ppt/slides/slide");
    if slides.is_empty() {
        anyhow::bail!("not a PowerPoint file");
    }
    let mut out = String::new();
    for (n, name) in slides {
        let Some(xml) = part(&mut archive, &name) else { continue };
        let lines: Vec<String> = elements(&xml, "a:p").iter().map(|p| tag_texts(p, "a:t").concat()).filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("## Slide {n}\n\n{}\n\n", lines.join("\n")));
    }
    if out.trim().is_empty() {
        anyhow::bail!("the presentation has no text");
    }
    Ok(out.trim_end().to_string() + "\n")
}

/// The text of a .xlsx: each sheet's rows, cells separated by tabs.
pub fn xlsx_text(bytes: &[u8]) -> anyhow::Result<String> {
    const MAX_ROWS: usize = 2_000;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("not an Excel file: {e}"))?;
    let shared: Vec<String> = part(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| elements(&xml, "si").iter().map(|si| tag_texts(si, "t").concat()).collect())
        .unwrap_or_default();
    let sheets = numbered(&archive, "xl/worksheets/sheet");
    if sheets.is_empty() {
        anyhow::bail!("not an Excel file");
    }
    let mut out = String::new();
    for (n, name) in sheets {
        let Some(xml) = part(&mut archive, &name) else { continue };
        let mut rows = Vec::new();
        for row in elements(&xml, "row").into_iter().take(MAX_ROWS) {
            let cells: Vec<String> = elements(row, "c")
                .into_iter()
                .map(|c| {
                    let value = tag_texts(c, "v").into_iter().next();
                    match attr(c, "t") {
                        Some("s") => value.and_then(|v| v.trim().parse::<usize>().ok()).and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                        Some("inlineStr") => tag_texts(c, "t").concat(),
                        _ => value.unwrap_or_default(),
                    }
                })
                .collect();
            let line = cells.join("\t");
            if !line.trim().is_empty() {
                rows.push(line.trim_end().to_string());
            }
        }
        if !rows.is_empty() {
            out.push_str(&format!("## Sheet {n}\n\n{}\n\n", rows.join("\n")));
        }
    }
    if out.trim().is_empty() {
        anyhow::bail!("the workbook has no text");
    }
    Ok(out.trim_end().to_string() + "\n")
}

/// Paragraphs of WordprocessingML, headings marked.
fn docx_xml_text(xml: &str) -> String {
    let mut out = String::new();
    // Each <w:p …>…</w:p> is a paragraph; the paragraph's own properties say its style.
    let mut rest = xml;
    while let Some(start) = find_tag(rest, "w:p") {
        let after = &rest[start..];
        // An empty paragraph (<w:p/>) closes itself; it must not run on into the next one.
        let open_end = after.find('>').map(|e| e + 1).unwrap_or(after.len());
        if after[..open_end].ends_with("/>") {
            rest = &after[open_end..];
            continue;
        }
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
            <w:p><w:pPr><w:pStyle w:val="Heading3"/></w:pPr><w:r><w:t>Next</w:t></w:r></w:p>
            <w:p w:rsidR="00AB"/>
            <w:proofErr w:type="spellStart"/><w:p><w:r><w:t>line one</w:t><w:br/><w:t>line two</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = docx_text(&docx(xml)).unwrap();
        assert_eq!(text, "# 설계 개요\n\nBronze & Silver\ttiers\n\n## Gold\n\n### Next\n\nline one\nline two\n");
    }

    #[test]
    fn refuses_what_is_not_there() {
        assert!(docx_text(b"not a zip").is_err());
        assert!(docx_text(&docx("<w:document><w:body><w:p/></w:body></w:document>")).is_err());
        assert!(pdf_text(b"%PDF-1.4 garbage").is_err());
    }

    fn zipped(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, body) in parts {
                z.start_file(*name, opts).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn slides_in_order() {
        let pptx = zipped(&[
            ("ppt/slides/slide10.xml", r#"<p:sld><a:p><a:r><a:t>Last</a:t></a:r></a:p></p:sld>"#),
            ("ppt/slides/slide2.xml", r#"<p:sld><a:p><a:r><a:t>로드맵 </a:t></a:r><a:r><a:t>&amp; 일정</a:t></a:r></a:p><a:p/><a:p><a:r><a:t>Q3</a:t></a:r></a:p></p:sld>"#),
        ]);
        assert_eq!(pptx_text(&pptx).unwrap(), "## Slide 2\n\n로드맵 & 일정\nQ3\n\n## Slide 10\n\nLast\n");
    }

    #[test]
    fn sheets_as_rows() {
        let xlsx = zipped(&[
            ("xl/sharedStrings.xml", r#"<sst><si><t>이름</t></si><si><r><t>가</t></r><r><t>격</t></r></si></sst>"#),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><cols><col min="1"/></cols><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>사과</t></is></c><c r="B2"><v>1200</v></c></row><row r="3"/></sheetData></worksheet>"#,
            ),
        ]);
        assert_eq!(xlsx_text(&xlsx).unwrap(), "## Sheet 1\n\n이름\t가격\n사과\t1200\n");
        assert!(xlsx_text(b"nope").is_err());
    }

    #[test]
    fn tidies_extracted_text() {
        assert_eq!(tidy("a  \r\n\r\n\r\n b\n\n"), "a\n\n b\n");
    }
}
