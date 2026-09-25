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
        // Its charset is decided per file (see `handle`).
        "html" | "htm" => "text/html",
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
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// The app's scrollbar (tokens.css), in the theme the panel is in
/// (`?theme=lt` for light). Put first in the head, so a page that styles
/// its own scrollbars still wins.
fn scrollbar_style(light: bool) -> String {
    let thumb = if light { "#d3cfc7" } else { "#242424" };
    format!(
        "<style data-divixi>::-webkit-scrollbar{{width:9px;height:9px}}::-webkit-scrollbar-thumb{{background:{thumb}}}\
         ::-webkit-scrollbar-track{{background:transparent}}::-webkit-scrollbar-corner{{background:transparent}}</style>"
    )
}

/// An HTML page with `style` put right after its `<head>` (or at its very
/// start when it has none).
/// Works on the bytes, so a page in another encoding (EUC-KR, Shift_JIS)
/// is left exactly as it was around the insert.
fn with_style(html: &[u8], style: &str) -> Vec<u8> {
    let lower = html.to_ascii_lowercase();
    let at = lower
        .windows(5)
        .position(|w| w == b"<head")
        .and_then(|i| lower[i..].iter().position(|b| *b == b'>').map(|j| i + j + 1))
        .unwrap_or(0);
    let mut out = Vec::with_capacity(html.len() + style.len());
    out.extend_from_slice(&html[..at]);
    out.extend_from_slice(style.as_bytes());
    out.extend_from_slice(&html[at..]);
    out
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

pub const BOARD_SCHEME: &str = "board";

/// `<design>/<file>` → a file of a design board's `files/` folder, for
/// its cards (`http://board.localhost/…` on Windows). Only plain names in
/// that folder are served.
pub fn handle_board<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let (design, file) = path.split_once('/').unwrap_or((path, ""));
    let (design, file) = (decode(design), decode(file));
    let plain = |s: &str| !s.is_empty() && !s.starts_with('.') && !s.contains(['/', '\\', ':']);
    if !plain(&design) || !plain(&file) {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    }
    let Some(state) = ctx.app_handle().try_state::<AppState>() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "starting");
    };
    let full = state.artifacts_dir.join(&design).join("files").join(&file);
    let Ok(bytes) = std::fs::read(&full) else {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    };
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, mime(&ext))
        .header("X-Content-Type-Options", "nosniff")
        .header(header::CACHE_CONTROL, "no-store");
    // An SVG or a page opened from here runs nothing (pictures and PDFs are left to their viewers).
    if matches!(ext.as_str(), "svg" | "html" | "htm") {
        res = res.header("Content-Security-Policy", "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'");
    }
    res
        .body(Cow::Owned(bytes))
        .unwrap_or_else(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "could not build the response"))
}

/// `<track>/<path…>` → the file's bytes, typed by extension.
pub fn handle<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let (track, rel) = path.split_once('/').unwrap_or((path, ""));
    let (track, rel) = (decode(track), decode(rel));
    if track.is_empty() || rel.is_empty() {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    }
    let Some(state) = ctx.app_handle().try_state::<AppState>() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "starting");
    };
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
    let html = matches!(ext.as_str(), "html" | "htm");
    // UTF-8 is said outright; any other page names its own charset in a <meta>.
    let content_type = if html && std::str::from_utf8(&bytes).is_ok() { "text/html; charset=utf-8" } else { mime(&ext) };
    let bytes = if html {
        let light = request.uri().query().is_some_and(|q| q.split('&').any(|p| p == "theme=lt"));
        with_style(&bytes, &scrollbar_style(light))
    } else {
        bytes
    };
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
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
    fn puts_the_scrollbar_style_first_in_the_head() {
        let s = "<s/>";
        assert_eq!(String::from_utf8(with_style(b"<html><HEAD lang=x><title>t</title></head>", s)).unwrap(), "<html><HEAD lang=x><s/><title>t</title></head>");
        assert_eq!(String::from_utf8(with_style(b"<p>bare</p>", s)).unwrap(), "<s/><p>bare</p>");
        // EUC-KR bytes for "한" around the insert come through untouched.
        let euc = [b"<head>".as_slice(), &[0xC7, 0xD1]].concat();
        assert_eq!(with_style(&euc, s), [b"<head><s/>".as_slice(), &[0xC7, 0xD1]].concat());
        assert!(scrollbar_style(true).contains("#d3cfc7") && scrollbar_style(false).contains("#242424"));
    }

    #[test]
    fn decodes_paths() {
        assert_eq!(decode("docs/%ED%95%9C%EA%B8%80.html"), "docs/한글.html");
        assert_eq!(decode("a%20b%2"), "a b%2");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%한글"), "%한글", "a % before non-ASCII is kept, not sliced through");
    }
}
