//! Cancel a turn in flight, as the composer's stop button does.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example cancel -- claude_code 4000
//!
//! Opens a session on the agent, sends a task that takes a while, cancels
//! after the given milliseconds, and prints how the turn ended. A working
//! cancel ends the run with a `Cancelled` stop reason well before the task
//! could have finished, and the session still answers a second prompt.

use std::sync::Arc;
use std::time::{Duration, Instant};

use orchestra_acp::{scrub_inherited_session_env, AgentSession, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::LaneEvent;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_acp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "claude_code".to_string());
    let after_ms: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;

    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let status = detect(kind, &opts).await;
    let agent = status
        .spec
        .ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;

    let session = Arc::new(
        AgentSession::open(
            &agent,
            SessionOptions {
                cwd: std::env::current_dir()?,
                ..Default::default()
            },
        )
        .await?,
    );
    println!("session {} · {} slash commands known", session.session_id(), session.commands().len());

    // A task long enough to be cancelled: count slowly, one number per line.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let s = session.clone();
    let turn = tokio::spawn(async move {
        s.prompt(
            "Write the numbers from 1 to 300, one per line, with a short sentence about each number. Do not use tools.".into(),
            tx,
        )
        .await
    });
    let canceller = session.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(after_ms)).await;
        println!("\n--- cancel after {after_ms}ms ---");
        canceller.cancel();
    });

    let started = Instant::now();
    let mut chars = 0usize;
    while let Some(event) = rx.recv().await {
        match event {
            LaneEvent::Message { text } | LaneEvent::Thought { text } => chars += text.chars().count(),
            other => println!("[{:>6}ms] {:<8} {other:?}", started.elapsed().as_millis(), other.label()),
        }
    }
    turn.await??;
    println!("turn ended after {}ms with {chars} chars of output", started.elapsed().as_millis());

    // The session is still alive and answers.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let s = session.clone();
    let turn = tokio::spawn(async move { s.prompt("Reply with exactly: STILL HERE".into(), tx).await });
    let mut reply = String::new();
    while let Some(event) = rx.recv().await {
        if let LaneEvent::Message { text } = event {
            reply.push_str(&text);
        }
    }
    turn.await??;
    println!("after cancel, the session replied: {}", reply.trim());
    Ok(())
}
