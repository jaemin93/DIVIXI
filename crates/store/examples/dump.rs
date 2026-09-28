//! Print what a store file holds: schema version, tracks, sessions per track.
//!
//! ```bash
//! cargo run -p orchestra-store --example dump -- path/to/orchestra.db
//! ```
//!
//! Copy the app's file first if the app is running (WAL mode is fine to
//! read, but a copy keeps the two from stepping on each other).

use orchestra_store::Store;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: dump <divixi.db>"))?;
    let store = Store::open(&path)?;
    println!("schema_version: {}", store.get_meta("schema_version")?.unwrap_or_default());
    for track in store.tracks()? {
        println!(
            "{}  {:<24} agent={} worker={:?} color={:?} tags={:?} runs={}",
            track.id, track.name, track.agent, track.worker_agent, track.color, track.tags, track.runs
        );
        for s in store.sessions(&track.id)? {
            println!("      session {:<16} {} runs, last {} {:?}", s.name, s.runs, s.last_run, s.last_status);
        }
    }
    Ok(())
}
