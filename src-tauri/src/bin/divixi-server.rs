//! Divixi with no window, for a server (built with `--features server`).
//!
//!   divixi-server            run: the app on Tauri's mock runtime, remote
//!                            access on 127.0.0.1 (port: the remote.port setting, 7488)
//!   divixi-server token      print a pairing link (five minutes, once)
//!
//! Reach it through an SSH tunnel (the desktop app opens one) or tailscale serve.

fn main() {
    // Must happen before any thread exists: a session's agent cannot inherit this
    // process's Claude Code session markers or it refuses to start.
    orchestra_acp::scrub_inherited_session_env();

    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "warn,orchestra_app=info,orchestra_acp=info,orchestra_mcp=info".into()),
                )
                .init();
            orchestra_app::run();
        }
        Some("token") => match orchestra_app::pair_link() {
            Ok(url) => println!("{url}"),
            Err(err) => {
                eprintln!("divixi-server token: {err:#}");
                std::process::exit(1);
            }
        },
        Some("-h" | "--help" | "help") => println!("usage: divixi-server [serve | token]"),
        Some(other) => {
            eprintln!("divixi-server: unknown command {other:?}\nusage: divixi-server [serve | token]");
            std::process::exit(2);
        }
    }
}
