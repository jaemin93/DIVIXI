//! A real agent, given a worker task with the report instructions, ends its
//! reply with a report block the app can read.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example report -- claude_code
//!
//! Makes a folder with a broken `calc.js`, asks the agent (as the app asks a
//! worker) to fix it and verify with node, allows what it asks, and parses
//! the reply with `report::parse`. If the block is missing it sends the
//! reminder once, as the app does. Checks: a structured report, status done,
//! calc.js among the changes, at least one check.

use orchestra_acp::{scrub_inherited_session_env, AgentSession, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::report;
use orchestra_core::AgentEvent;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

/// One turn: the reply text and the paths its edit tools touched.
async fn ask(session: &AgentSession, prompt: String) -> anyhow::Result<(String, Vec<String>)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let turn = session.prompt(prompt, tx);
    let collect = async {
        let mut text = String::new();
        let mut edits = Vec::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text: t } => text.push_str(&t),
                AgentEvent::ToolCall { title, tool_kind, paths, .. } => {
                    println!("  tool  {tool_kind}  {title}  {paths:?}");
                    if tool_kind == "edit" {
                        edits.extend(paths);
                    }
                }
                AgentEvent::Permission { request, title, options, .. } => {
                    let allow = options.iter().find(|o| o.kind == "allow_once").map(|o| o.id.clone());
                    println!("  allow {title}");
                    let _ = session.answer_permission(&request, allow.as_deref());
                }
                _ => {}
            }
        }
        (text, edits)
    };
    let (done, out) = tokio::join!(turn, collect);
    done?;
    Ok(out)
}

async fn run() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "claude_code".to_string());
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let agent = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;

    let cwd = std::env::temp_dir().join("divixi-report");
    let _ = std::fs::remove_dir_all(&cwd);
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("calc.js"), "function add(a, b) {\n  return a - b;\n}\nmodule.exports = { add };\n")?;
    let session = AgentSession::open(&agent, SessionOptions { cwd: cwd.clone(), ..Default::default() }).await?;
    println!("session on {} in {}", kind.name(), cwd.display());

    let task = "calc.js의 add가 두 수를 더하지 않고 뺍니다. 고치고, node로 add(2, 3)이 5인지 확인하세요.";
    let started = std::time::Instant::now();
    let (reply, edits) = ask(&session, format!("{task}\n\n{}", report::instructions())).await?;
    println!("--- reply ({:.0}s) ---\n{reply}\n---", started.elapsed().as_secs_f32());
    let parsed = match report::parse(&reply) {
        Ok(r) => Ok(r),
        Err(problem) => {
            println!("no usable report ({problem}); asking again");
            let (again, _) = ask(&session, report::reminder(&problem)).await?;
            println!("--- second reply ---\n{again}\n---");
            report::parse(&again)
        }
    };
    let mut r = parsed.map_err(|e| anyhow::anyhow!("REPORT: FAILED ({e})"))?;
    r.edits_seen = edits;
    println!("--- for the conductor ---\n{}\n---", r.for_conductor("t1"));

    let fixed = std::fs::read_to_string(cwd.join("calc.js"))?.contains("a + b");
    let ok = r.structured && r.status == report::Standing::Done && r.changes.iter().any(|c| c.path.contains("calc.js")) && !r.checks.is_empty() && fixed;
    println!("{}", if ok { "REPORT: OK" } else { "REPORT: CHECK FAILED" });
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
