//! A design's agent drawing on a real Excalidraw scene, without the window.
//!
//! Run with:
//!   cargo run -p orchestra-app --example design_agent -- claude_code
//!
//! The board starts with one labelled rectangle of the human's. The agent
//! gets the design preamble and the same two MCP tools the app gives it
//! (`board_read`, `board_write`, same descriptions and schema), and is asked
//! to draw the idea out as a diagram. Checks: it drew through the tool; the
//! scene gained several labelled shapes, a frame and arrows bound at both
//! ends, one of them to the human's rectangle. The scene is written to a
//! `.excalidraw` file to open in excalidraw.com.

use std::sync::Arc;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_app::design::{self, Scene};
use orchestra_core::AgentEvent;
use orchestra_mcp::{McpServer, Tool};
use parking_lot::Mutex;
use serde_json::{json, Value};

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

    // The human's board: one rectangle.
    let board = Arc::new(Mutex::new(Scene::default()));
    let (res, _) = board.lock().apply(design::parse_commands(&json!({ "commands": [
        { "op": "add", "type": "rectangle", "x": 0, "y": 0, "width": 260, "height": 120, "label": "Family todo app on the tablet" }
    ] })).map_err(anyhow::Error::msg)?);
    let human = res[0]["id"].as_str().unwrap_or_default().to_string();

    let (r, w) = (board.clone(), board.clone());
    let tools = vec![
        Tool::new(design::BOARD_READ, design::BOARD_READ_DESC, json!({ "type": "object", "properties": {} }), move |_| {
            let b = r.clone();
            async move { Ok(Value::String(b.lock().outline())) }
        }),
        Tool::new(design::BOARD_WRITE, design::BOARD_WRITE_DESC, design::board_write_schema(), move |args| {
            let b = w.clone();
            async move {
                let ops = design::parse_commands(&args)?;
                let (results, _) = b.lock().apply(ops);
                println!("  board_write → {}", serde_json::to_string(&results).unwrap_or_default());
                Ok(json!({ "results": results }))
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
            cwd: cwd.clone(),
            mcp_servers: vec![McpHttp { name: "divixi".to_string(), url: server.url(), headers: vec![server.auth_header()] }],
            ..Default::default()
        },
    )
    .await?;

    let outline = board.lock().outline();
    let prompt = format!(
        "{}\n\n---\n\nDraw this out with me as a diagram: the main screens as labelled shapes in a frame, how a todo flows between them with arrows, and one decision as a diamond. Connect it to my rectangle.\n\n[board]\n{outline}",
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

    let sc = board.lock().clone();
    println!("[board]\n{}\n", sc.outline());
    let live: Vec<&Value> = sc.elements.iter().filter(|e| e["isDeleted"] != true).collect();
    let count = |t: &str| live.iter().filter(|e| e["type"] == t).count();
    let shapes = count("rectangle") + count("ellipse") + count("diamond");
    let bound = live
        .iter()
        .filter(|e| e["type"] == "arrow" && e["startBinding"]["elementId"].is_string() && e["endBinding"]["elementId"].is_string())
        .count();
    let to_mine = live
        .iter()
        .filter(|e| e["type"] == "arrow" && (e["startBinding"]["elementId"] == human.as_str() || e["endBinding"]["elementId"] == human.as_str()))
        .count();
    let labelled = live.iter().filter(|e| e["type"] == "text" && e["containerId"].is_string()).count();
    let file = cwd.join("board.excalidraw");
    std::fs::write(&file, serde_json::to_string_pretty(&json!({ "type": "excalidraw", "version": 2, "source": "divixi", "elements": sc.elements, "appState": {}, "files": sc.files }))?)?;
    println!(
        "shapes {shapes} · diamonds {} · frames {} · bound arrows {bound} · arrows on mine {to_mine} · labels {labelled}\nscene: {}",
        count("diamond"),
        count("frame"),
        file.display()
    );
    let ok = shapes >= 4 && count("diamond") >= 1 && count("frame") >= 1 && bound >= 2 && to_mine >= 1 && labelled >= 3;
    println!("{}", if ok { "DESIGN AGENT: OK" } else { "DESIGN AGENT: CHECK FAILED" });
    Ok(())
}
