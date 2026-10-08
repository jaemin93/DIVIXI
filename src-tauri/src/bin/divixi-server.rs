//! Divixi with no window, for a server (built with `--features server`).
//!
//!   divixi-server                 run: the app on Tauri's mock runtime, a remote
//!                                 instance on port 7488 (the remote.port setting)
//!   divixi-server -v | -vv        log more: info, or debug (agents' stderr as well)
//!   divixi-server token           print a pairing link (five minutes, once)
//!   divixi-server owner <login>   the GitHub account that may come in at an address (--none: nobody)
//!   divixi-server listen all|local  answer on every network, or this machine only
//!   divixi-server phone status|on|off|link
//!                                 phone access over Tailscale, decided here (see
//!                                 src/remote/phone_cli.rs)
//!
//! The desktop app reaches it through an SSH tunnel (signing in with a
//! pairing token over SSH), or at its address as the owner's GitHub account.
//!
//! There is no settings window here, so `-v`/`-vv` (or `DIVIXI_LOG`) is how
//! the level is set. The first two lines it prints say which build this is,
//! the level it is logging at and where the log file is — both go to stdout
//! as well as to that file, so a service manager keeps them too.

const USAGE: &str = "usage: divixi-server [-v|-vv] [serve | token | owner <github-login>|--none | listen all|local | phone status|on|off|link]";

fn done(r: anyhow::Result<String>, what: &str) {
    match r {
        Ok(msg) => println!("{msg}"),
        Err(err) => {
            eprintln!("{what}: {err:#}");
            std::process::exit(1);
        }
    }
}

/// `-v` and `-vv`, wherever they appear: the log level for this run, set the
/// way `DIVIXI_LOG` sets it so that there is still one gate (src/logging.rs).
/// A flag typed now wins over what the environment already said.
///
/// Returns whether the argument was one of them, so the caller can drop it
/// before reading the rest as a command.
fn verbosity(arg: &str) -> bool {
    let level = match arg {
        "-v" | "--verbose" => "info",
        "-vv" => "debug",
        _ => return false,
    };
    // Before any thread exists, as with the session variables above.
    std::env::set_var(orchestra_app::LEVEL_ENV, level);
    true
}

fn main() {
    // Must happen before any thread exists: a session's agent cannot inherit this
    // process's Claude Code session markers or it refuses to start.
    orchestra_acp::scrub_inherited_session_env();

    // Before anything logs, and before the arguments are read as commands:
    // a level asked for on the command line is the level this run starts at.
    let args: Vec<String> = std::env::args().skip(1).filter(|arg| !verbosity(arg)).collect();

    match args.first().map(String::as_str) {
        None | Some("serve") => {
            // Logging starts inside `run()`: to stdout as here before, and to
            // a file under the app's data folder (src/logging.rs), at the level
            // `-v`/`-vv` or DIVIXI_LOG asked for. Its first lines say which
            // level that is and where that file is.
            orchestra_app::run();
        }
        Some("token") => match orchestra_app::pair_link() {
            Ok(url) => println!("{url}"),
            Err(err) => {
                eprintln!("divixi-server token: {err:#}");
                std::process::exit(1);
            }
        },
        Some("owner") => done(orchestra_app::set_owner(args.get(1).map(String::as_str)), "divixi-server owner"),
        Some("listen") => done(orchestra_app::set_listen(args.get(1).map(String::as_str)), "divixi-server listen"),
        Some("phone") => done(orchestra_app::phone(&args[1..]), "divixi-server phone"),
        Some("-h" | "--help" | "help") => println!("{USAGE}"),
        Some(other) => {
            eprintln!("divixi-server: unknown command {other:?}\n{USAGE}");
            std::process::exit(2);
        }
    }
}
