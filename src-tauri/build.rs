fn main() {
    // tauri-build embeds the icon into the executable but does not watch it,
    // so a regenerated icon would stay stale until something else rebuilt.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    tauri_build::build()
}
