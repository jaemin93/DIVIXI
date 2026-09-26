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
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
