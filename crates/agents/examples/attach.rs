//! Hand an agent files with a prompt, as the composer's attachments do.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example attach -- claude_code <image.png> <note.txt>
//!
//! First turn: the image alone, asking for its colour — an agent that takes
//! pictures answers from the pixels. Second turn: the text file, asking for
//! the codeword inside — the agent has to follow the file link and read it.

use std::path::PathBuf;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::AgentEvent;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,orchestra_acp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn turn(session: &AgentSession, text: &str, files: Vec<PathBuf>) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let collect = tokio::spawn(async move {
        let mut out = String::new();
        let mut tools = 0;
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::Message { text } => out.push_str(&text),
                AgentEvent::ToolCall { title, .. } => {
                    tools += 1;
                    println!("  tool: {title}");
                }
                AgentEvent::Finished { stop_reason } => println!("  finished: {stop_reason} ({tools} tools)"),
                AgentEvent::Failed { error } => println!("  failed: {error}"),
                _ => {}
            }
        }
        out
    });
    session.prompt_with(text.to_string(), files, tx).await?;
    Ok(collect.await?)
}

async fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "claude_code".to_string());
    let image = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("pass an image path"))?);
    let note = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("pass a text file path"))?);
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;

    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let agent = detect(kind, &opts)
        .await
        .spec
        .ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;
    let session = AgentSession::open(&agent, SessionOptions { cwd: std::env::current_dir()?, ..Default::default() }).await?;

    println!("[image] {}", image.display());
    let colour = turn(&session, "What single colour fills the attached image? Answer with one word.", vec![image]).await?;
    println!("  answer: {}", colour.trim());

    println!("[file] {}", note.display());
    let word = turn(&session, "Read the attached file and reply with only the codeword it contains.", vec![note]).await?;
    println!("  answer: {}", word.trim());
    Ok(())
}
