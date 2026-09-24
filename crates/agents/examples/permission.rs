//! An agent that asks before it acts, answered through the session.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example permission -- claude_code default
//!
//! Opens the agent in a mode that asks (Claude Code's `default`), has it
//! write a file, and answers the permission question it sends with the first
//! "allow" option. Checks: a question arrives with its options; after the
//! answer the file is written; a second write answered "cancelled" is not.

use orchestra_acp::{scrub_inherited_session_env, AgentSession, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::AgentEvent;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

/// One turn; every permission question is answered by \`decide\`.
async fn turn(
    session: Arc<AgentSession>,
    text: &str,
    decide: fn(&[orchestra_core::PermissionChoice]) -> Option<String>,
) -> anyhow::Result<(String, usize)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let s = session.clone();
    let text = text.to_string();
    let run = tokio::spawn(async move { s.prompt(text, tx).await });
    let mut reply = String::new();
    let mut asked = 0;
    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::Permission { request, title, tool_kind, options, .. } => {
                asked += 1;
                let names: Vec<String> = options.iter().map(|o| format!("{} ({})", o.name, o.kind)).collect();
                println!("  asks [{request}] {tool_kind}: {title}\n    options: {}", names.join(", "));
                let answer = decide(&options);
                println!("    answer: {}", answer.as_deref().unwrap_or("cancelled"));
                session.answer_permission(&request, answer.as_deref())?;
            }
            AgentEvent::Message { text } => reply.push_str(&text),
            AgentEvent::Finished { stop_reason } => println!("  finished: {stop_reason}"),
            AgentEvent::Failed { error } => println!("  failed: {error}"),
            _ => {}
        }
    }
    run.await??;
    Ok((reply, asked))
}

fn allow(options: &[orchestra_core::PermissionChoice]) -> Option<String> {
    options.iter().find(|o| o.kind == "allow_once").or_else(|| options.iter().find(|o| o.kind.starts_with("allow"))).map(|o| o.id.clone())
}

fn cancel(_: &[orchestra_core::PermissionChoice]) -> Option<String> {
    None
}

async fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "claude_code".to_string());
    let mode = args.next().unwrap_or_else(|| "default".to_string());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let agent = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;
    let cwd = std::env::temp_dir().join("divixi-permission");
    std::fs::create_dir_all(&cwd)?;
    let session = Arc::new(AgentSession::open(&agent, SessionOptions { cwd: cwd.clone(), mode: Some(mode.clone()), ..Default::default() }).await?);
    println!("session on {} in mode {mode}", kind.name());

    let _ = std::fs::remove_file(cwd.join("allowed.txt"));
    let _ = std::fs::remove_file(cwd.join("denied.txt"));
    println!("[allowed] write a file");
    let (reply, asked) = turn(session.clone(), "Create a file named allowed.txt in the current directory containing the word ok. Use your file writing tool, not the shell. Then say done.", allow).await?;
    println!("  reply: {}", reply.trim());
    let ran = cwd.join("allowed.txt").exists();

    println!("[cancelled] write another");
    let (reply2, asked2) = turn(session.clone(), "Create a file named denied.txt in the current directory containing the word no. Use your file writing tool, not the shell. If you are not allowed, just say so.", cancel).await?;
    println!("  reply: {}", reply2.trim());
    let blocked = !cwd.join("denied.txt").exists();

    let ok = asked >= 1 && ran && asked2 >= 1 && blocked;
    println!("\nasked {asked} then {asked2} · ran after allow: {ran} · held back after cancel: {blocked}");
    println!("{}", if ok { "PERMISSION: OK" } else { "PERMISSION: CHECK FAILED" });
    Ok(())
}
