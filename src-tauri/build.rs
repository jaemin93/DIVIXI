fn main() {
    // tauri-build embeds the icon into the executable but does not watch it,
    // so a regenerated icon would stay stale until something else rebuilt.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    // The built UI is embedded at compile time, and it is what the phone
    // server hands out -- a dev build too, whose own window shows the dev
    // server instead. Unwatched, a `dist` built since the last Rust change
    // stayed out, and a phone got the UI from before it.
    if std::path::Path::new("../dist").exists() {
        println!("cargo:rerun-if-changed=../dist");
    }

    // On Windows the app needs Common Controls v6 (Tauri's own manifest). Given
    // to the linker rather than the app's resource file, it reaches the test
    // executables too: without it, tests that link Tauri's mock runtime exit
    // with STATUS_ENTRYPOINT_NOT_FOUND before they start.
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        attributes = attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo")).join("windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    build_id();
    release_tag();
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// `DIVIXI_RELEASE`: which release this binary is, or empty for a build that
/// is not one.
///
/// The manifests keep plain semver (0.1.0 in package.json and Cargo.toml) and
/// move on their own schedule, while a release is named by date
/// (`v2026-09-28`, and `v2026-09-28.2` for a second one the same day). So
/// `CARGO_PKG_VERSION` cannot answer "is there a newer release than the one I
/// am": every build of every day says 0.1.0. The release workflow knows the
/// tag it is building, and this is how the tag reaches the binary.
///
/// The workflow passes it as `DIVIXI_RELEASE_TAG` in the environment of the
/// bundling step. Nothing else sets it, so a developer's `cargo build` or
/// `npm run app` leaves it empty and the app calls itself a development build
/// -- which is honest, and the only thing it can say: a build made from
/// working copy has no release to compare against.
///
/// `rerun-if-env-changed` so a rebuild in the same tree picks up a tag that
/// was not there before (and drops one that has gone) without a `cargo clean`.
fn release_tag() {
    println!("cargo:rerun-if-env-changed=DIVIXI_RELEASE_TAG");
    let tag = std::env::var("DIVIXI_RELEASE_TAG").unwrap_or_default();
    let tag = tag.trim();
    // A stray quote or newline in a workflow value would end up inside the
    // compiled string and then fail to parse at runtime, far from its cause.
    // Refuse the characters that could only be a mistake, rather than ship a
    // binary that reports a tag nobody can read.
    if tag.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\') {
        panic!("DIVIXI_RELEASE_TAG={tag:?} holds whitespace or a quote; it should be a release tag such as v2026-09-28");
    }
    // And it has to be the tag, not the release title and not the version.
    // Checked here rather than left to the app, which can only report a build
    // it cannot do anything about: passing `${{ ... outputs.title }}` instead
    // of `outputs.tag` should fail the release, loudly, on the machine that
    // did it. The shape is the one `update::parse` accepts; this is the
    // second, deliberate copy of it, since a build script cannot call into
    // the crate it is building.
    if !tag.is_empty() && !is_release_tag(tag) {
        panic!("DIVIXI_RELEASE_TAG={tag:?} is not a release tag. Expected vYYYY-MM-DD, or vYYYY-MM-DD.N for the Nth release of that day (v2026-09-28, v2026-09-28.2) -- pass the workflow's tag output, not its title or the manifest version.");
    }
    println!("cargo:rustc-env=DIVIXI_RELEASE={tag}");
}

/// `vYYYY-MM-DD`, with an optional `.N` counting from 1. Kept in step with
/// `parse` in src/update.rs, whose tests are the ones that pin the shape.
fn is_release_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else { return false };
    let (date, seq) = match rest.split_once('.') {
        Some((date, seq)) => (date, Some(seq)),
        None => (rest, None),
    };
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |s: &str| -> Option<u32> { (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten() };
    let (Some(_), Some(month), Some(day)) = (num(&date[0..4]), num(&date[5..7]), num(&date[8..10])) else { return false };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    match seq {
        None => true,
        Some(s) => num(s).is_some_and(|n| n >= 1),
    }
}

/// `DIVIXI_BUILD`: a hash of the Rust sources (the app's and its crates').
/// Two builds of the same code match, git or not (a server builds from an
/// archive), so a remote instance built from other code can be told apart
/// even when both say version 0.1.0. The UI is left out: a remote instance
/// runs commands and the page is always this PC's, so a change to the page
/// alone asks nothing of the remote. Line endings are left out too: a
/// Windows checkout and a Linux one hash alike.
fn build_id() {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest.parent().expect("src-tauri has a parent").to_path_buf();
    let dirs = [root.join("src-tauri/src"), root.join("crates")];
    let mut files = Vec::new();
    for d in &dirs {
        println!("cargo:rerun-if-changed={}", d.display());
        walk(d, &mut files);
    }
    let rel = |p: &std::path::Path| p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    files.sort_by_key(|p| rel(p));
    // FNV-1a, 64 bits: stable everywhere (std's hasher is not promised to be).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            if b == b'\r' {
                continue;
            }
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for f in &files {
        feed(rel(f).as_bytes());
        feed(&[0]);
        feed(&std::fs::read(f).unwrap_or_default());
        feed(&[0]);
    }
    println!("cargo:rustc-env=DIVIXI_BUILD={:012x}", h & 0xffff_ffff_ffff);
}

/// Source files under `dir` (built output and dependencies left out).
fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("rs" | "toml")) {
            out.push(p);
        }
    }
}
