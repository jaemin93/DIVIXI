//! Install the npm ACP adapters (Claude Code, Codex) into a folder, as the
//! app does into its adapter folder, and say where their entry scripts are.
//!
//!   cargo run -p orchestra-agents --example adapters -- <folder>

use orchestra_agents::{install_npm_adapter, AgentKind};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: adapters <folder>"))?;
    for kind in [AgentKind::ClaudeCode, AgentKind::Codex] {
        let started = std::time::Instant::now();
        let script = install_npm_adapter(kind, std::path::Path::new(&dir)).await?;
        println!("{}: {} ({:.1} s)", kind.name(), script.display(), started.elapsed().as_secs_f32());
    }
    Ok(())
}
