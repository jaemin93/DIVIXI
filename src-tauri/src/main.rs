// Release builds must not pop a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Must happen before any thread exists: a session's agent cannot inherit this
    // process's Claude Code session markers or it refuses to start.
    orchestra_acp::scrub_inherited_session_env();

    // Logging is not started here: it goes to a file under the app's data
    // folder, and where that is on this platform is Tauri's to say, so the
    // app starts it as the first thing in `setup` (src/logging.rs).
    orchestra_app::run();
}
