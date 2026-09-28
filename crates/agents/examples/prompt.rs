//! Run one prompt in a session on a chosen agent.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example prompt -- codex "Reply with exactly: DIVIXI OK"
//!
//! Agent ids: claude_code, codex, copilot, antigravity. The Antigravity
//! server must already be downloaded (see the `detect` example).

use orchestra_acp::{run_prompt, scrub_inherited_session_env, PromptSpec};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::AgentEvent;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_agents=info,orchestra_acp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "claude_code".to_string());
    let prompt = args.next().unwrap_or_else(|| "Reply with exactly: DIVIXI OK".to_string());
    // Optional third argument: a model value id for the agent's `model` option.
    let config: Vec<(String, String)> = args.next().map(|m| vec![("model".to_string(), m)]).unwrap_or_default();
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;

    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let status = detect(kind, &opts).await;
    let agent = status
        .spec
        .ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter: {:?}", kind.name(), status.adapter))?;
    println!("agent: {} via {} {:?}", kind.name(), agent.program, agent.args);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(run_prompt(
        PromptSpec {
            agent,
            cwd: std::env::current_dir()?,
            prompt,
            mode: None,
            config,
        },
        tx,
    ));

    let started = std::time::Instant::now();
    while let Some(event) = rx.recv().await {
        let ms = started.elapsed().as_millis();
        let side = if event.above_membrane() { "^" } else { " " };
        match &event {
            AgentEvent::Message { text } | AgentEvent::Thought { text } => {
                use std::io::Write;
                print!("{text}");
                std::io::stdout().flush().ok();
            }
            other => println!("\n[{ms:>6}ms] {side} {:<8} {other:?}", event.label()),
        }
    }
    task.await??;
    println!("\n--- session closed ---");
    Ok(())
}
