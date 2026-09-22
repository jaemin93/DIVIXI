//! Persistence smoke test: run one real lane, record every event, reopen the
//! store and read the run back the way the app does at startup.
//!
//! Run with:
//!   cargo run -p orchestra-store --example persist -- "say hello in five words"
//!
//! `ORCHESTRA_DB` picks the database file; the default is a temp file that is
//! deleted afterwards.

use orchestra_acp::{run_lane, scrub_inherited_session_env, AgentSpec, LaneSpec};
use orchestra_store::Store;

fn main() -> anyhow::Result<()> {
    scrub_inherited_session_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,orchestra_acp=info,orchestra_store=info".into()),
        )
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: ORCHESTRA OK".to_string());

    let (path, ephemeral) = match std::env::var_os("ORCHESTRA_DB") {
        Some(p) => (std::path::PathBuf::from(p), false),
        None => (
            std::env::temp_dir().join(format!("orchestra-persist-{}.db", std::process::id())),
            true,
        ),
    };
    println!("db: {}", path.display());

    // Process 1: record.
    let run_id = {
        let store = Store::open(&path)?;
        let cwd = std::env::current_dir()?;
        let run_id = store.begin_run("solo", "claude_code", &prompt, &cwd.display().to_string())?;
        println!("run: {run_id}");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(run_lane(
            LaneSpec {
                agent: AgentSpec::claude_code(),
                cwd,
                prompt,
                mode: Some("bypassPermissions".to_string()),
                config: Vec::new(),
            },
            tx,
        ));

        let started = std::time::Instant::now();
        let mut count = 0usize;
        while let Some(event) = rx.recv().await {
            store.append(&run_id, started.elapsed().as_millis() as u64, &event)?;
            count += 1;
        }
        task.await??;
        println!("recorded {count} events");
        run_id
    };

    // Process 2: what the app sees on the next launch.
    let store = Store::open(&path)?;
    let runs = store.runs()?;
    println!("\n--- timeline (above the membrane) ---");
    for r in &runs {
        println!(
            "{} {:<9} {:>6}ms  tools={} plan={} output={:?}",
            r.id,
            r.status.as_str(),
            r.duration_ms.unwrap_or(0),
            r.tool_count,
            r.plan.len(),
            r.output.chars().take(60).collect::<String>(),
        );
    }

    let events = store.events(&run_id)?;
    println!("\n--- lane log for {run_id} ({} events, below the membrane) ---", events.len());
    for e in &events {
        println!("[{:>6}ms] {}", e.at_ms, e.event.kind());
    }

    let hits = store.search("ORCHESTRA")?;
    println!("\n--- search \"ORCHESTRA\" ---");
    for h in &hits {
        println!("{}  {}", h.run, h.snippet);
    }

    drop(store);
    if ephemeral {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
    Ok(())
}
