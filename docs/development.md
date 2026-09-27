# Working on Divixi

Rust + Tauri 2 + Svelte 5. The crates still carry their first name, `orchestra-*`.

## Layout

```
crates/orchestra/  domain: AgentEvent, what reaches the timeline
crates/acp/        ACP client: launching agents, sessions, streaming
crates/agents/     agent catalog: finding CLIs, ACP probe and login, adapter installs
crates/mcp/        in-process HTTP MCP server: the conductor's tools
crates/store/      SQLite event store: append-only log, runs projection, FTS5
src-tauri/         the app: commands, sessions, remote instances (src/remote), divixi-server
ui/                Svelte 5 UI
docs/design/       design notes
```

## Principles

- **The timeline shows what needs the human.** It carries reports, decisions, running work
  and the conversation. Transcripts, diffs and tool calls stay in the worker view.
  `AgentEvent::above_membrane()` draws that line in code.
- **Mechanism in Rust, policy in the agents.** The core starts sessions and carries
  events. What to delegate, and how, is the conductor's decision, in its preamble
  (`src-tauri/src/conductor.rs`). The conductor gets the app's tools (`spawn_worker`,
  `ask_worker`, `read_report`, `request_decision`, …) from an MCP server bound to its
  Track.
- **ACP lives in one place.** Only `crates/acp` speaks the protocol. Everything above it
  sees `AgentEvent`.

## Running

```bash
npm install
npm run app                          # tauri dev
cargo test -p orchestra-app --lib    # the app's tests (some are Windows-only)
npx svelte-check                     # UI types
cargo build -p orchestra-app --features server --bin divixi-server
```

Useful examples:

```bash
cargo run -p orchestra-acp --example smoke -- "Reply with exactly: OK"
cargo run -p orchestra-agents --example detect                # what detection finds
cargo run -p orchestra-agents --example prompt -- codex "…"   # one run on one agent
cargo run -p orchestra-agents --example adapters -- <folder>  # install the npm adapters
```

## Things to know

- **Running inside a Claude Code session breaks Claude Code workers.** Claude Code treats
  inherited `CLAUDECODE` / `CLAUDE_CODE_*` variables as a nested session and exits at
  once. You see "Query closed before response received" at `session/new`.
  `scrub_inherited_session_env()` clears them at the top of `main`, before any thread
  starts.
- **Tauri is pinned to 2.11.x, and `@tauri-apps/api` to ~2.11.** The remote bridge uses
  unstable APIs (`InvokeRequest`, `Webview::on_message`, multiple webviews per window).
  Check `Cargo.lock` and `npm ls @tauri-apps/api` after adding a plugin, so neither side
  moves: the CLI refuses mismatched versions.
- **Adapters.** Claude Code and Codex speak ACP through npm adapters, pinned in
  `crates/acp` and `crates/agents`.
  - A development build uses the repo's `node_modules`.
  - An installed one installs them into `<data>/adapters` and falls back to `npx` until
    then.
  - Antigravity's ACP server is a download.
- **Agents run in their most autonomous mode.** Humans see the escalations agents raise,
  not tool approvals. Per-track settings can choose otherwise.
- **The event store.**
  - Events are appended to `events`. `runs` is their fold when a run ends, and the
    timeline reads only `runs`.
  - A run left open by a crash is closed as failed on the next start.
  - The database is `divixi.db` in the data folder, and `DIVIXI_DB` overrides it.
- **Text chunks from agents are sent in 40 ms frames**, to the webview and to the store
  alike.
