//! A design's agent on a real board, without the window.
//!
//! Run with:
//!   cargo run -p orchestra-app --example design_agent -- claude_code
//!
//! The board starts with one note of the human's. The agent gets the design
//! preamble and the same two MCP tools the app gives it (`board_read`,
//! `board_write`, same descriptions and schema), and is asked to rough the
//! idea out. Checks: it wrote through the tool; the board gained a goal,
//! constraints and a question; they hang off the human's note by arrows;
//! every agent item is a suggestion waiting on the human.

use std::sync::Arc;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_app::design::{self, Actor, Doc};
use orchestra_core::AgentEvent;
use orchestra_mcp::{McpServer, Tool};
use parking_lot::Mutex;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "claude_code".to_string());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;

    // The human's board: one note, no tags.
    let board = Arc::new(Mutex::new(Doc::default()));
    board.lock().apply(
        serde_json::from_value(json!([{ "op": "create_note", "x": 0, "y": 0, "text": "A shared todo list for my kids, on the family tablet" }]))?,
        &Actor::Human,
    );

    let (r, w, t) = (board.clone(), board.clone(), board.clone());
    let tools = vec![
        Tool::new(design::BOARD_READ, design::BOARD_READ_DESC, json!({ "type": "object", "properties": {} }), move |_| {
            let b = r.clone();
            async move { Ok(b.lock().outline()) }
        }),
        Tool::new(design::BOARD_WRITE, design::BOARD_WRITE_DESC, design::board_write_schema(), move |args| {
            let b = w.clone();
            async move {
                let ops = design::parse_commands(&args)?;
                let results = b.lock().apply(ops, &Actor::Agent { run: None });
                println!("  board_write → {}", serde_json::to_string(&results).unwrap_or_default());
                Ok(json!({ "results": results }))
            }
        }),
        // One item with words, its words at the top level (see design::board_text_schema).
        Tool::new(design::BOARD_TEXT, design::BOARD_TEXT_DESC, design::board_text_schema(), move |args| {
            let b = t.clone();
            async move {
                let ops = design::parse_commands(&design::text_command(&args))?;
                let results = b.lock().apply(ops, &Actor::Agent { run: None });
                println!("  board_text → {}", serde_json::to_string(&results).unwrap_or_default());
                Ok(results.into_iter().next().unwrap_or(serde_json::Value::Null))
            }
        }),
    ];
    let server = McpServer::start("divixi", tools).await?;

    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let agent = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;
    let cwd = std::env::temp_dir().join("divixi-design-agent");
    std::fs::create_dir_all(&cwd)?;
    let session = AgentSession::open(
        &agent,
        SessionOptions {
            cwd,
            mcp_servers: vec![McpHttp { name: "divixi".to_string(), url: server.url(), headers: vec![server.auth_header()] }],
            ..Default::default()
        },
    )
    .await?;

    let outline = board.lock().outline();
    let prompt = format!(
        "{}\n\n---\n\nRough this idea out with me: put the goal, two constraints and one open question on the board as tagged notes, and connect them to my note.\n\n[board]\n{outline}",
        design::preamble("en", "Family todo")
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let print = tokio::spawn(async move {
        let mut reply = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text } => reply.push_str(&text),
                AgentEvent::ToolCall { title, .. } => println!("  tool: {title}"),
                AgentEvent::Finished { stop_reason } => println!("  finished: {stop_reason}"),
                AgentEvent::Failed { error } => println!("  failed: {error}"),
                _ => {}
            }
        }
        reply
    });
    session.prompt(prompt, tx).await?;
    let reply = print.await?;
    println!("\n[reply]\n{}\n", reply.trim());

    let d = board.lock().clone();
    let tags = |t: &str| d.nodes.iter().filter(|n| n.tag == t).count();
    let human = &d.nodes[0].id;
    let attached = d.edges.iter().filter(|e| &e.from == human || &e.to == human).count();
    let agent_items = d.nodes.iter().filter(|n| n.by == "agent").count() + d.edges.iter().filter(|e| e.by == "agent").count();
    println!("[board]\n{}", serde_json::to_string_pretty(&d.outline())?);
    println!(
        "\ngoals {} · constraints {} · questions {} · arrows on my note {} · suggestions {}/{}",
        tags("goal"),
        tags("constraint"),
        tags("question"),
        attached,
        d.changes.len(),
        agent_items
    );
    let ok = tags("goal") >= 1 && tags("constraint") >= 2 && tags("question") >= 1 && attached >= 1 && d.changes.len() == agent_items;
    println!("{}", if ok { "DESIGN AGENT: OK" } else { "DESIGN AGENT: CHECK FAILED" });
    Ok(())
}
