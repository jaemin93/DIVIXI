//! The conductor loop without the desktop shell, in the app's asynchronous
//! shape: `spawn_lane` returns at once, the lane works in the background,
//! and its report is handed to the conductor as a new `[레인 보고]` turn.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example orchestra -- claude_code
//!
//! Checks: a greeting gets no lane; a task gets a lane and a one-line
//! "delegated" reply; the report turn gets a summary.

use std::sync::Arc;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::LaneEvent;
use orchestra_mcp::{McpServer, Tool};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const PREAMBLE: &str = r#"당신은 Orchestra의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 레인(lane)이라는 별도의 에이전트 세션에 맡깁니다.

규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 레인을 열지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_lane`으로 레인을 열어 맡깁니다. 레인 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 레인이 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- `spawn_lane`은 레인이 일을 받는 즉시 돌아옵니다. 결과를 기다리지 말고, 사람에게 무엇을 맡겼는지 한 문장으로 알린 뒤 턴을 끝냅니다. 레인이 끝나면 `[레인 보고]`로 시작하는 메시지가 당신에게 옵니다. 그때 무슨 일이 있었는지 한두 문단으로 사람에게 설명합니다.
- 한국어로 말합니다. 짧게, 명확하게.
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

    // Finished lane reports land here; the main loop feeds them to the conductor.
    let (report_tx, mut report_rx) = mpsc::unbounded_channel::<String>();

    let spawn_spec = spec.clone();
    let spawn_cwd = cwd.clone();
    let spawn = Tool::new(
        "spawn_lane",
        "Open a new lane and give it a task. Returns at once; the lane's report arrives later as a [레인 보고] message.",
        json!({ "type": "object", "properties": { "name": { "type": "string" }, "task": { "type": "string" } }, "required": ["name", "task"] }),
        move |args: Value| {
            let spec = spawn_spec.clone();
            let cwd = spawn_cwd.clone();
            let report_tx = report_tx.clone();
            async move {
                let name = args["name"].as_str().unwrap_or("lane").to_string();
                let task = args["task"].as_str().unwrap_or("").to_string();
                println!("    ▸ spawn_lane {name}: {}", task.chars().take(90).collect::<String>());
                let started = std::time::Instant::now();
                let lane = name.clone();
                tokio::spawn(async move {
                    let name = lane;
                    let outcome = async {
                        let session = AgentSession::open(&spec, SessionOptions { cwd, ..Default::default() }).await?;
                        let out = lane_turn(&session, &task).await?;
                        session.close().await;
                        Ok::<_, anyhow::Error>(out)
                    }
                    .await;
                    let report = match outcome {
                        Ok(out) => format!("[레인 보고] lane={name} status=done duration_ms={}\n\n{out}", started.elapsed().as_millis()),
                        Err(err) => format!("[레인 보고] lane={name} status=failed error={err}"),
                    };
                    println!("    ◂ lane {name} finished in {}ms", started.elapsed().as_millis());
                    let _ = report_tx.send(report);
                });
                Ok(json!({ "lane": name, "status": "running", "note": "Tell the human what you delegated and end your turn." }))
            }
        },
    );
    let server = McpServer::start("orchestra", vec![spawn]).await?;

    println!("opening conductor on {}…", kind.name());
    let conductor = Arc::new(
        AgentSession::open(
            &spec,
            SessionOptions {
                cwd: cwd.clone(),
                mcp_servers: vec![McpHttp { name: "orchestra".into(), url: server.url(), headers: vec![server.auth_header()] }.into()],
                ..Default::default()
            },
        )
        .await?,
    );

    for (i, msg) in ["안녕하세요", "이 저장소 README.md의 첫 번째 제목 줄이 뭔지 알려줘"].iter().enumerate() {
        let text = if i == 0 { format!("{PREAMBLE}\n\n---\n\n{msg}") } else { msg.to_string() };
        println!("\n[나] {msg}");
        let (reply, tools) = turn(&conductor, &text).await?;
        println!("[지휘자] {reply}");
        println!("[tools used] {tools:?}");
    }

    // The report comes in on its own time; feed it to the conductor as a turn.
    println!("\n… waiting for the lane report …");
    let report = tokio::time::timeout(std::time::Duration::from_secs(600), report_rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no report"))?;
    println!("[보고] {}", report.lines().next().unwrap_or(""));
    let (reply, tools) = turn(&conductor, &report).await?;
    println!("[지휘자] {reply}");
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
                LaneEvent::Message { text } => out.push_str(&text),
                LaneEvent::ToolCall { title, .. } => tools.push(title),
                LaneEvent::Finished { .. } | LaneEvent::Failed { .. } => break,
                _ => {}
            }
        }
        (out.trim().to_string(), tools)
    };
    let (result, out) = tokio::join!(prompt, collect);
    result?;
    Ok(out)
}

async fn lane_turn(session: &AgentSession, text: &str) -> anyhow::Result<String> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let prompt = session.prompt(text.to_string(), tx);
    let collect = async {
        let mut out = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                LaneEvent::Message { text } => out.push_str(&text),
                LaneEvent::Finished { .. } | LaneEvent::Failed { .. } => break,
                _ => {}
            }
        }
        out.trim().to_string()
    };
    let (result, out) = tokio::join!(prompt, collect);
    result?;
    Ok(out)
}
