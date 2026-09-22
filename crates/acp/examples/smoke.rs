//! Smoke test: prove the ACP handshake and streaming work end to end.
//!
//! Run with:
//!   cargo run -p orchestra-acp --example smoke -- "say hello in five words"

use orchestra_acp::{run_lane, scrub_inherited_session_env, AgentSpec, LaneSpec};
use orchestra_core::LaneEvent;

fn main() -> anyhow::Result<()> {
    // Before the runtime exists, so the mutation is single-threaded.
    scrub_inherited_session_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_acp=debug".into()),
        )
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: ORCHESTRA OK".to_string());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let task = tokio::spawn(run_lane(
        LaneSpec {
            agent: AgentSpec::claude_code(),
            cwd: std::env::current_dir()?,
            prompt,
            mode: Some("bypassPermissions".to_string()),
            config: Vec::new(),
        },
        tx,
    ));

    let started = std::time::Instant::now();
    while let Some(event) = rx.recv().await {
        let ms = started.elapsed().as_millis();
        let side = if event.above_membrane() { "^" } else { " " };
        match &event {
            LaneEvent::Message { text } | LaneEvent::Thought { text } => {
                use std::io::Write;
                print!("{text}");
                std::io::stdout().flush().ok();
            }
            other => println!("\n[{ms:>6}ms] {side} {:<8} {other:?}", event.label()),
        }
    }

    task.await??;
    println!("\n--- lane closed ---");
    Ok(())
}
