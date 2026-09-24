//! A document described by a real agent, stored, and searched.
//!
//! Run with:
//!   cargo run -p orchestra-agents --example knowledge -- claude_code [path.md]
//!
//! Chunks the file (default: this repo's HANDOFF.md, first 3 chunks), asks
//! the agent the knowledge extraction prompt for each chunk in a read-only
//! mode, parses the replies, asks for the document summary, stores it all
//! in an in-memory library and runs a Korean and an English search. Checks:
//! every reply parses, entities were found, both searches hit.

use orchestra_acp::{scrub_inherited_session_env, AgentSession, SessionOptions};
use orchestra_agents::{detect, AgentKind, DetectOptions};
use orchestra_core::AgentEvent;
use orchestra_knowledge::{chunk_document, extract, read, FileState, KnowledgeDb, NewItem};

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn ask(session: &AgentSession, prompt: String) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let turn = session.prompt(prompt, tx);
    let collect = async {
        let mut text = String::new();
        let mut tools = 0;
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Message { text: t } => text.push_str(&t),
                AgentEvent::ToolCall { .. } => tools += 1,
                AgentEvent::Permission { request, .. } => {
                    let _ = session.answer_permission(&request, None);
                }
                _ => {}
            }
        }
        (text, tools)
    };
    let (done, (text, tools)) = tokio::join!(turn, collect);
    done?;
    if tools > 0 {
        println!("  (the agent called {tools} tool(s))");
    }
    Ok(text)
}

async fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "claude_code".to_string());
    let path = args.next().map(std::path::PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap().join("HANDOFF.md"));
    let kind = AgentKind::parse(&id).ok_or_else(|| anyhow::anyhow!("unknown agent {id}"))?;
    let adapters_dir = std::env::temp_dir().join("orchestra-adapters");
    let mut opts = DetectOptions::new(&adapters_dir, std::env::current_dir()?);
    opts.skip_probe = true;
    let agent = detect(kind, &opts).await.spec.ok_or_else(|| anyhow::anyhow!("{} has no launchable adapter", kind.name()))?;
    let cwd = std::env::temp_dir().join("divixi-knowledge");
    std::fs::create_dir_all(&cwd)?;
    let session = AgentSession::open(&agent, SessionOptions { cwd, mode: Some("plan".into()), ..Default::default() }).await?;
    println!("session on {}", kind.name());

    let file = read::read_file(&path)?;
    let shape = read::shape_of(&path).unwrap();
    let mut chunks = chunk_document(shape, &file.text);
    println!("{}: {} chunks, describing the first 3", path.display(), chunks.len());
    chunks.truncate(3);

    let mut items = Vec::new();
    let mut parsed = 0;
    let mut entities = 0;
    for chunk in chunks {
        let started = std::time::Instant::now();
        let reply = ask(&session, extract::extraction_prompt(&chunk.content, "n0nce")).await?;
        let x = extract::parse_extraction(&reply);
        match &x {
            Some(x) => {
                parsed += 1;
                entities += x.entities.len();
                println!(
                    "  [{}] {:.1}s {} ({}) · {} entities, {} relations\n      {}",
                    chunk.index,
                    started.elapsed().as_secs_f32(),
                    x.title,
                    x.category,
                    x.entities.len(),
                    x.relations.len(),
                    x.summary
                );
            }
            None => println!("  [{}] unparsed reply: {}", chunk.index, reply.chars().take(300).collect::<String>()),
        }
        items.push(NewItem { chunk, extraction: x, tags: vec!["content_type:markdown".into()] });
    }
    let summaries: Vec<String> = items.iter().filter_map(|i| i.extraction.as_ref().map(|x| x.summary.clone())).collect();
    let summary = extract::parse_summary(&ask(&session, extract::summary_prompt(&summaries.join("\n"))).await?);
    println!("summary: {summary:?}");

    let db = KnowledgeDb::in_memory()?;
    db.add_source("ar001", "local_file", &path.to_string_lossy())?;
    let n = items.len();
    db.replace_items("ar001", &items, parsed > 0, &FileState { hash: file.hash.clone(), mtime_ms: file.mtime_ms, size: file.size as i64 })?;
    println!("stored: {:?}", db.stats()?);
    let ko = db.search("지휘자 작업자", 3, None, None)?;
    let en = db.search("conductor", 3, None, None)?;
    for h in ko.iter().chain(&en) {
        println!("  hit {:.4} {} · {}", h.score, h.match_type, h.item.title);
    }
    let ok = parsed == n && entities > 0 && !ko.is_empty() && !en.is_empty() && summary.is_some();
    println!("\nparsed {parsed}/{n} · entities {entities} · ko hits {} · en hits {}", ko.len(), en.len());
    println!("{}", if ok { "KNOWLEDGE: OK" } else { "KNOWLEDGE: CHECK FAILED" });
    Ok(())
}
