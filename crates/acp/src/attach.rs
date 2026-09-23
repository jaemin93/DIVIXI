//! Files the human hands to an agent with a prompt.
//!
//! Every ACP agent must accept a `resource_link` block, so a file goes as
//! a link to its path the agent can read with its own tools. Images go
//! inline as `image` blocks when the agent says it takes them (so it sees
//! the picture, not just a path), up to a size worth sending over stdio.

use std::path::Path;

use agent_client_protocol::schema::v1::{ContentBlock, ImageContent, ResourceLink};
use base64::Engine;

/// Images above this go as links rather than inline.
const MAX_INLINE_IMAGE: u64 = 5 * 1024 * 1024;

/// The media type for a file extension, when it is one worth naming.
pub fn mime_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        _ => return None,
    })
}

/// `file:///C:/a%20b/c.png` for `C:\a b\c.png`; `file:///a/b` on Unix.
pub fn file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let raw = raw.strip_prefix("//?/").unwrap_or(&raw);
    let mut out = String::from("file://");
    if !raw.starts_with('/') {
        out.push('/');
    }
    for ch in raw.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            _ => out.push(ch),
        }
    }
    out
}

/// The prompt as content blocks: the text, then one block per file.
pub fn prompt_blocks(text: &str, files: &[impl AsRef<Path>], images: bool) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = vec![text.to_string().into()];
    for file in files {
        let path = file.as_ref();
        let uri = file_uri(path);
        let mime = mime_of(path);
        let size = std::fs::metadata(path).map(|m| m.len()).ok();
        let inline = images && mime.is_some_and(|m| m.starts_with("image/") && m != "image/svg+xml") && size.is_some_and(|s| s <= MAX_INLINE_IMAGE);
        if inline {
            if let (Ok(bytes), Some(mime)) = (std::fs::read(path), mime) {
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                blocks.push(ContentBlock::Image(ImageContent::new(data, mime).uri(uri)));
                continue;
            }
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| uri.clone());
        let mut link = ResourceLink::new(name, uri);
        if let Some(mime) = mime {
            link = link.mime_type(mime.to_string());
        }
        if let Some(size) = size {
            link = link.size(size as i64);
        }
        blocks.push(ContentBlock::ResourceLink(link));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_absolute_file_urls() {
        assert_eq!(file_uri(Path::new(r"C:\a b\c#1.png")), "file:///C:/a%20b/c%231.png");
        assert_eq!(file_uri(Path::new("/tmp/x.md")), "file:///tmp/x.md");
    }

    #[test]
    fn images_inline_only_when_the_agent_takes_them() {
        let dir = std::env::temp_dir().join(format!("divixi-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("p.png");
        let md = dir.join("n.md");
        std::fs::write(&png, [137u8, 80, 78, 71]).unwrap();
        std::fs::write(&md, "# hi").unwrap();

        let with = prompt_blocks("look", &[&png, &md], true);
        assert!(matches!(with[0], ContentBlock::Text(_)));
        assert!(matches!(&with[1], ContentBlock::Image(i) if i.mime_type == "image/png" && i.data == "iVBORw=="));
        assert!(matches!(&with[2], ContentBlock::ResourceLink(l) if l.name == "n.md" && l.size == Some(4)));

        let without = prompt_blocks("look", &[&png], false);
        assert!(matches!(&without[1], ContentBlock::ResourceLink(l) if l.mime_type.as_deref() == Some("image/png")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
