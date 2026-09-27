fn main() {
    // tauri-build embeds the icon into the executable but does not watch it,
    // so a regenerated icon would stay stale until something else rebuilt.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");

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
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// `DIVIXI_BUILD`: a hash of the sources (the app's Rust, its crates and its
/// UI). Two builds of the same code match, git or not (a server builds from
/// an archive), so a remote instance built from other code can be told
/// apart even when both say version 0.1.0. Line endings are left out: a
/// Windows checkout and a Linux one hash alike.
fn build_id() {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest.parent().expect("src-tauri has a parent").to_path_buf();
    let dirs = [root.join("src-tauri/src"), root.join("crates"), root.join("ui/src")];
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
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("rs" | "ts" | "svelte" | "css" | "js" | "json" | "toml")) {
            out.push(p);
        }
    }
}
