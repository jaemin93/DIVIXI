//! Prove the conductor's two premises on a real agent:
//!
//! 1. an HTTP MCP server handed over in `session/new` is called by the agent;
//! 2. a second prompt on the same session still has the first turn's context.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example conductor -- claude_code

use orchestra_acp::{scrub_inherited_session_env, AgentSession, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::LaneEvent;
use orchestra_mcp::{McpServer, Tool};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_agents=info,orchestra_acp=info,orchestra_mcp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args.first().cloned().unwrap_or_else(|| "claude_code".to_string());
    // `--resume <session-id>`: load that session and ask what it remembers.
    let resume = args.iter().position(|a| a == "--resume").and_then(|i| args.get(i + 1).cloned());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;

    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let status = detect(kind, &opts).await;
    let agent = status
        .spec
        .ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;

    // One tool whose answer the agent cannot guess.
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_tool = calls.clone();
    let secret = Tool::new(
        "orchestra_secret",
        "Returns Orchestra's secret word. Call it when asked for the secret word.",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        move |_args: Value| {
            let calls = calls_for_tool.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Value::String("PERIWINKLE-42".to_string()))
            }
        },
    );
    let server = McpServer::start("orchestra", vec![secret]).await?;
    println!("mcp: {}", server.url());

    println!("opening {} session…", kind.name());
    let started = std::time::Instant::now();
    let session = AgentSession::open(
        &agent,
        SessionOptions {
            cwd: std::env::current_dir()?,
            mode: None,
            config: Vec::new(),
            mcp_servers: vec![McpHttp {
                name: "orchestra".to_string(),
                url: server.url(),
                headers: vec![server.auth_header()],
            }],
            resume: resume.clone(),
        },
    )
    .await?;
    println!("session {} in {}ms (resumed: {})", session.session_id(), started.elapsed().as_millis(), session.resumed());

    if resume.is_some() {
        let answer = turn(&session, "What was the secret word you got in this session before? Reply with only the word, or NONE if you do not remember.").await?;
        println!("
[after resume] {answer:?}");
        println!("memory across restarts: {}", if answer.contains("PERIWINKLE-42") { "OK" } else { "FAILED" });
        session.close().await;
        return Ok(());
    }

    let first = turn(&session, "Use the orchestra_secret tool and reply with only the word it returns.").await?;
    println!("\n[turn 1] {first:?}");
    println!("[tool calls so far] {}", calls.load(Ordering::SeqCst));

    let second = turn(&session, "What was the secret word you got a moment ago? Reply with only the word.").await?;
    println!("\n[turn 2] {second:?}");
    println!("[tool calls total] {}", calls.load(Ordering::SeqCst));

    let ok_tool = calls.load(Ordering::SeqCst) >= 1 && first.contains("PERIWINKLE-42");
    let ok_memory = second.contains("PERIWINKLE-42");
    println!("\nMCP via ACP: {}", if ok_tool { "OK" } else { "FAILED" });
    println!("context across turns: {}", if ok_memory { "OK" } else { "FAILED" });

    session.close().await;
    Ok(())
}

/// Run one turn, print its tool calls, return the message text.
async fn turn(session: &AgentSession, text: &str) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let prompt = session.prompt(text.to_string(), tx);
    let collect = async {
        let mut out = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                LaneEvent::Message { text } => out.push_str(&text),
                LaneEvent::ToolCall { title, tool_kind, .. } => println!("  tool  {tool_kind}  {title}"),
                LaneEvent::Finished { .. } | LaneEvent::Failed { .. } => break,
                _ => {}
            }
        }
        out
    };
    let (result, out) = tokio::join!(prompt, collect);
    result?;
    Ok(out.trim().to_string())
}
