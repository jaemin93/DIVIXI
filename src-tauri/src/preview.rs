//! Serving a track's files to the side panel's HTML preview, on an origin
//! of its own (`preview://localhost/<track>/<path>`, which Windows spells
//! `http://preview.localhost/…`). A page there runs its scripts and loads
//! its stylesheets, images and scripts from beside it, while the frame's
//! sandbox and this origin keep it away from the app: it cannot reach the
//! app's page or its commands, cannot leave the track's folder, and its own
//! CSP keeps it off the network, so what it reads stays on this machine.

use std::borrow::Cow;

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext};

use crate::AppState;

pub const SCHEME: &str = "preview";

/// What a previewed page may do: everything from its own folder, inline
/// scripts and styles, nothing from the network, no forms, no popups (the
/// frame's sandbox already denies those and top navigation).
const PAGE_CSP: &str = "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; img-src 'self' data: blob:; \
    media-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; form-action 'none'";

fn mime(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "txt" | "md" | "csv" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// `%XX` escapes decoded; invalid ones are kept as they are.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn refuse(status: StatusCode, why: &str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Cow::Owned(why.as_bytes().to_vec()))
        .unwrap_or_else(|_| Response::new(Cow::Owned(Vec::new())))
}

/// `<track>/<path…>` → the file's bytes, typed by extension.
pub fn handle<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let (track, rel) = path.split_once('/').unwrap_or((path, ""));
    let (track, rel) = (decode(track), decode(rel));
    if track.is_empty() || rel.is_empty() {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    }
    let state = ctx.app_handle().state::<AppState>();
    let full = match crate::track_root(&state, &track).and_then(|root| crate::workspace::resolve(&root, &rel)) {
        Ok(full) => full,
        Err(_) => return refuse(StatusCode::NOT_FOUND, "no such file"),
    };
    if !full.is_file() {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    }
    let bytes = match std::fs::read(&full) {
        Ok(b) => b,
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let ext = full.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    Response::builder()
        .header(header::CONTENT_TYPE, mime(&ext))
        .header("Content-Security-Policy", PAGE_CSP)
        .header("X-Content-Type-Options", "nosniff")
        // Saved edits show on the next load.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Cow::Owned(bytes))
        .unwrap_or_else(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "could not build the response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_paths() {
        assert_eq!(decode("docs/%ED%95%9C%EA%B8%80.html"), "docs/한글.html");
        assert_eq!(decode("a%20b%2"), "a b%2");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%한글"), "%한글", "a % before non-ASCII is kept, not sliced through");
    }
}
