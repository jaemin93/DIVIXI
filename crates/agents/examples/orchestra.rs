//! The conductor loop without the desktop shell: does the conductor answer
//! chit-chat itself and delegate real work to a lane?
//!
//! Run with:
//!   cargo run -p orchestra-agents --example orchestra -- claude_code
//!
//! Sends two messages: a greeting (expect: no lane) and a small task
//! (expect: spawn_lane, then a summary of the lane's report).

use std::collections::HashMap;
use std::sync::Arc;

use orchestra_acp::{scrub_inherited_session_env, AgentSession, AgentSpec, McpHttp, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::LaneEvent;
use orchestra_mcp::{McpServer, Tool};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const PREAMBLE: &str = r#"당신은 Orchestra의 지휘자(conductor)입니다. 사람과 대화하는 유일한 상대이며, 실제 작업은 레인(lane)이라는 별도의 에이전트 세션에 맡깁니다.

규칙:
- 사람의 메시지가 질문이나 잡담이면 직접 답합니다. 레인을 열지 않습니다.
- 코드를 읽거나 고치거나 조사하는 일처럼 실제 작업이 필요하면 `spawn_lane`으로 레인을 열어 맡깁니다. 레인 이름은 짧은 영문 소문자(예: fix-parser)로 짓고, task에는 레인이 혼자 끝낼 수 있을 만큼 구체적으로 적습니다.
- 같은 레인에 이어서 시킬 일은 `ask_lane`으로 보냅니다. 레인은 이전 대화를 기억합니다.
- 레인의 보고를 받으면 사람에게 무슨 일이 있었는지 한두 문단으로 설명합니다. 보고를 그대로 붙여넣지 말고 요점만 말합니다.
- 한국어로 말합니다. 짧게, 명확하게.
"#;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_mcp=info".into()),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

type Lanes = Arc<Mutex<HashMap<String, Arc<AgentSession>>>>;

async fn run() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "claude_code".to_string());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let spec: AgentSpec = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("no adapter"))?;
    let cwd = std::env::current_dir()?;

    let lanes: Lanes = Arc::new(Mutex::new(HashMap::new()));
    let spawn_lanes = lanes.clone();
    let spawn_spec = spec.clone();
    let spawn_cwd = cwd.clone();
    let ask_lanes = lanes.clone();

    let spawn = Tool::new(
        "spawn_lane",
        "Open a new lane (a separate agent session) and give it a task. Blocks until the lane finishes and returns its output. Use for real work; answer questions yourself instead.",
        json!({ "type": "object", "properties": { "name": { "type": "string" }, "task": { "type": "string" } }, "required": ["name", "task"] }),
        move |args: Value| {
            let lanes = spawn_lanes.clone();
            let spec = spawn_spec.clone();
            let cwd = spawn_cwd.clone();
            async move {
                let name = args["name"].as_str().unwrap_or("lane").to_string();
                let task = args["task"].as_str().unwrap_or("").to_string();
                println!("    ▸ spawn_lane {name}: {}", task.chars().take(90).collect::<String>());
                let session = AgentSession::open(&spec, SessionOptions { cwd, ..Default::default() })
                    .await
                    .map_err(|e| e.to_string())?;
                let session = Arc::new(session);
                lanes.lock().await.insert(name.clone(), session.clone());
                let out = lane_turn(&session, &task).await.map_err(|e| e.to_string())?;
                println!("    ◂ lane {name} done ({} chars)", out.len());
                Ok(json!({ "lane": name, "status": "done", "output": out }))
            }
        },
    );
    let ask = Tool::new(
        "ask_lane",
        "Send a follow-up message to an existing lane; it remembers earlier turns. Returns its output.",
        json!({ "type": "object", "properties": { "name": { "type": "string" }, "message": { "type": "string" } }, "required": ["name", "message"] }),
        move |args: Value| {
            let lanes = ask_lanes.clone();
            async move {
                let name = args["name"].as_str().unwrap_or("").to_string();
                let message = args["message"].as_str().unwrap_or("").to_string();
                let session = lanes.lock().await.get(&name).cloned().ok_or_else(|| format!("no lane {name}"))?;
                println!("    ▸ ask_lane {name}: {}", message.chars().take(90).collect::<String>());
                let out = lane_turn(&session, &message).await.map_err(|e| e.to_string())?;
                Ok(json!({ "lane": name, "status": "done", "output": out }))
            }
        },
    );
    let server = McpServer::start("orchestra", vec![spawn, ask]).await?;

    println!("opening conductor on {}…", kind.name());
    let conductor = AgentSession::open(
        &spec,
        SessionOptions {
            cwd: cwd.clone(),
            mcp_servers: vec![McpHttp { name: "orchestra".into(), url: server.url(), headers: vec![server.auth_header()] }],
            ..Default::default()
        },
    )
    .await?;

    for (i, msg) in ["안녕하세요", "이 저장소 README.md의 첫 번째 제목 줄이 뭔지 알려줘"].iter().enumerate() {
        let text = if i == 0 { format!("{PREAMBLE}\n\n---\n\n{msg}") } else { msg.to_string() };
        println!("\n[나] {msg}");
        let (reply, tools) = conductor_turn(&conductor, &text).await?;
        println!("[지휘자] {reply}");
        println!("[tools used] {tools:?}");
    }

    println!("\nlanes opened: {:?}", lanes.lock().await.keys().collect::<Vec<_>>());
    Ok(())
}

async fn conductor_turn(session: &AgentSession, text: &str) -> anyhow::Result<(String, Vec<String>)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
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
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
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
