// Release builds must not pop a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Must happen before any thread exists: a lane's agent cannot inherit this
    // process's Claude Code session markers or it refuses to start.
    orchestra_acp::scrub_inherited_session_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_app=info,orchestra_acp=debug".into()),
        )
        .init();

    orchestra_app::run();
}
