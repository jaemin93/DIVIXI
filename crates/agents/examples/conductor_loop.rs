//! The conductor loop without the desktop shell, in the app's asynchronous
//! shape: `spawn_worker` returns at once, the worker works in the background,
//! and its report is handed to the conductor as a new `[worker report]` turn.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example conductor_loop -- claude_code
//!
//! Checks: a greeting gets no worker; a task gets a worker and a one-line
//! "delegated" reply; the report turn gets a summary.

use std::sync::Arc;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::AgentEvent;
use orchestra_mcp::{McpServer, Tool};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const PREAMBLE: &str = r#"You are Divixi's conductor. You are the only one the human talks to, and the actual work goes to separate agent sessions called workers.

Rules:
- If the human's message is a question or small talk, answer it yourself. Do not open a worker.
- If it needs real work — reading, changing or investigating code — call `spawn_worker` and hand it over. Name the worker in short lowercase latin letters (fix-parser, say), and write the task specifically enough that the worker can finish it alone.
- `spawn_worker` returns as soon as the worker has the task. Do not wait for the result: tell the human in one sentence what you delegated, and end your turn. When the worker finishes, a message beginning with `[worker report]` arrives. Explain what happened to the human then, in a paragraph or two.
- Keep it short and clear.
"#;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "claude_code".to_string());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let spec: AgentSpec = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("no adapter"))?;
    let cwd = std::env::current_dir()?;

    // Finished worker reports land here; the main loop feeds them to the conductor.
    let (report_tx, mut report_rx) = mpsc::unbounded_channel::<String>();

    let spawn_spec = spec.clone();
    let spawn_cwd = cwd.clone();
    let spawn = Tool::new(
        "spawn_worker",
        "Open a new worker and give it a task. Returns at once; the worker's report arrives later as a [worker report] message.",
        json!({ "type": "object", "properties": { "name": { "type": "string" }, "task": { "type": "string" } }, "required": ["name", "task"] }),
        move |args: Value| {
            let spec = spawn_spec.clone();
            let cwd = spawn_cwd.clone();
            let report_tx = report_tx.clone();
            async move {
                let name = args["name"].as_str().unwrap_or("worker").to_string();
                let task = args["task"].as_str().unwrap_or("").to_string();
                println!("    ▸ spawn_worker {name}: {}", task.chars().take(90).collect::<String>());
                let started = std::time::Instant::now();
                let worker = name.clone();
                tokio::spawn(async move {
                    let name = worker;
                    let outcome = async {
                        let session = AgentSession::open(&spec, SessionOptions { cwd, ..Default::default() }).await?;
                        let out = worker_turn(&session, &task).await?;
                        session.close().await;
                        Ok::<_, anyhow::Error>(out)
                    }
                    .await;
                    let report = match outcome {
                        Ok(out) => format!("[worker report] worker={name} status=done duration_ms={}\n\n{out}", started.elapsed().as_millis()),
                        Err(err) => format!("[worker report] worker={name} status=failed error={err}"),
                    };
                    println!("    ◂ worker {name} finished in {}ms", started.elapsed().as_millis());
                    let _ = report_tx.send(report);
                });
                Ok(json!({ "worker": name, "status": "running", "note": "Tell the human what you delegated and end your turn." }))
            }
        },
    );
    let server = McpServer::start("divixi", vec![spawn]).await?;

    println!("opening conductor on {}…", kind.name());
    let conductor = Arc::new(
        AgentSession::open(
            &spec,
            SessionOptions {
                cwd: cwd.clone(),
                mcp_servers: vec![McpHttp { name: "divixi".into(), url: server.url(), headers: vec![server.auth_header()] }],
                ..Default::default()
            },
        )
        .await?,
    );

    for (i, msg) in ["hello there", "tell me the first heading line of README.md in this repository"].iter().enumerate() {
        let text = if i == 0 { format!("{PREAMBLE}\n\n---\n\n{msg}") } else { msg.to_string() };
        println!("\n[human] {msg}");
        let (reply, tools) = turn(&conductor, &text).await?;
        println!("[conductor] {reply}");
        println!("[tools used] {tools:?}");
    }

    // The report comes in on its own time; feed it to the conductor as a turn.
    println!("\n… waiting for the worker report …");
    let report = tokio::time::timeout(std::time::Duration::from_secs(600), report_rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no report"))?;
    println!("[report] {}", report.lines().next().unwrap_or(""));
    let (reply, tools) = turn(&conductor, &report).await?;
    println!("[conductor] {reply}");
    println!("[tools used] {tools:?}");
    Ok(())
}

async fn turn(session: &AgentSession, text: &str) -> anyhow::Result<(String, Vec<String>)> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let prompt = session.prompt(text.to_string(), tx);
    let collect = async {
        let mut out = String::new();
        let mut tools = Vec::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text } => out.push_str(&text),
                AgentEvent::ToolCall { title, .. } => tools.push(title),
                AgentEvent::Finished { .. } | AgentEvent::Failed { .. } => break,
                _ => {}
            }
        }
        (out.trim().to_string(), tools)
    };
    let (result, out) = tokio::join!(prompt, collect);
    result?;
    Ok(out)
}

async fn worker_turn(session: &AgentSession, text: &str) -> anyhow::Result<String> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let prompt = session.prompt(text.to_string(), tx);
    let collect = async {
        let mut out = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text } => out.push_str(&text),
                AgentEvent::Finished { .. } | AgentEvent::Failed { .. } => break,
                _ => {}
            }
        }
        out.trim().to_string()
    };
    let (result, out) = tokio::join!(prompt, collect);
    result?;
    Ok(out)
}
