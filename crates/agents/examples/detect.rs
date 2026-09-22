//! Detect every agent on this machine and print what the setup screen would show.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example detect
//!   cargo run -p orchestra-agents --example detect -- --download   # also fetch the Antigravity server
//!   cargo run -p orchestra-agents --example detect -- --no-probe   # locate only

use orchestra_agents::{detect_all, DetectOptions, Readiness};

fn main() -> anyhow::Result<()> {
    orchestra_acp::scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_agents=info,orchestra_acp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = args.iter().any(|a| a == "--no-probe");

    if args.iter().any(|a| a == "--download") {
        let path = orchestra_agents::download_antigravity_with(&adapters_dir, |p| {
            let total = p.total.map(|t| format!("{:.1} MB", t as f64 / 1048576.0)).unwrap_or_else(|| "?".into());
            println!("  {:?} {:.1} MB / {total}", p.phase, p.received as f64 / 1048576.0);
        })
        .await?;
        println!("antigravity server: {}", path.display());
    }

    let started = std::time::Instant::now();
    let mut statuses = detect_all(&opts).await;
    println!("detected in {}ms\n", started.elapsed().as_millis());

    // `--login <agent>[:<method>]` runs ACP `authenticate` for one agent.
    if let Some(target) = args.iter().position(|a| a == "--login").and_then(|i| args.get(i + 1)) {
        let (id, method) = match target.split_once(':') {
            Some((id, m)) => (id, Some(m)),
            None => (target.as_str(), None),
        };
        if let Some(i) = statuses.iter().position(|s| s.kind.id() == id) {
            println!("logging in {id} via {}…\n", method.unwrap_or("first advertised method"));
            statuses[i] = orchestra_agents::login(&statuses[i], method, &opts).await;
        } else {
            println!("unknown agent {id}\n");
        }
    }

    for s in &statuses {
        let mark = match s.readiness {
            Readiness::Ready => "READY",
            Readiness::NeedsLogin => "LOGIN",
            Readiness::NeedsDownload => "DOWNLOAD",
            Readiness::NotInstalled => "MISSING",
            Readiness::Error => "ERROR",
        };
        println!("{:<10} {:<15} {}", mark, s.kind.id(), s.name);
        match &s.cli {
            Some(c) => println!("           cli      {}  ({})", c.path, c.version.as_deref().unwrap_or("version unknown")),
            None => println!("           cli      not found"),
        }
        println!("           adapter  {:?}", s.adapter);
        if let Some(p) = &s.probe {
            println!(
                "           probe    {} {} · protocol {} · loadSession={} · mcpHttp={} · session {:?}",
                p.agent_name.as_deref().unwrap_or("?"),
                p.agent_version.as_deref().unwrap_or(""),
                p.protocol,
                p.load_session,
                p.mcp_http,
                p.session
            );
            for m in &p.auth_methods {
                println!("           auth     {} — {}{}", m.id, m.name, m.terminal_command.as_deref().map(|c| format!("  [{c}]")).unwrap_or_default());
            }
            for o in &p.config_options {
                println!("           option   [{}] {} = {}  ({} choices)", o.category, o.id, o.current, o.choices.len());
                for c in &o.choices {
                    println!("                      {:<36} {}{}", c.id, c.name, c.group.as_deref().map(|g| format!("  · {g}")).unwrap_or_default());
                }
            }
        }
        if let Some(e) = &s.error {
            println!("           error    {e}");
        }
        println!("           login    {}", s.login_hint);
        println!();
    }
    Ok(())
}
