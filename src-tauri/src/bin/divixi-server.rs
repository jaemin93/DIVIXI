//! Divixi with no window, for a server (built with `--features server`).
//!
//!   divixi-server                 run: the app on Tauri's mock runtime, a remote
//!                                 instance on port 7488 (the remote.port setting)
//!   divixi-server token           print a pairing link (five minutes, once)
//!   divixi-server owner <login>   the GitHub account that may come in at an address (--none: nobody)
//!   divixi-server listen all|local  answer on every network, or this machine only
//!
//! The desktop app reaches it through an SSH tunnel (signing in with a
//! pairing token over SSH), or at its address as the owner's GitHub account.

const USAGE: &str = "usage: divixi-server [serve | token | owner <github-login>|--none | listen all|local]";

fn done(r: anyhow::Result<String>, what: &str) {
    match r {
        Ok(msg) => println!("{msg}"),
        Err(err) => {
            eprintln!("{what}: {err:#}");
            std::process::exit(1);
        }
    }
}

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
        Some("owner") => done(orchestra_app::set_owner(std::env::args().nth(2).as_deref()), "divixi-server owner"),
        Some("listen") => done(orchestra_app::set_listen(std::env::args().nth(2).as_deref()), "divixi-server listen"),
        Some("-h" | "--help" | "help") => println!("{USAGE}"),
        Some(other) => {
            eprintln!("divixi-server: unknown command {other:?}\n{USAGE}");
            std::process::exit(2);
        }
    }
}
