# Working on DIVIXI

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
npm run build                        # once, before any cargo command: see below
npm run app                          # the app, in development mode (tauri dev)
cargo build -p orchestra-app --features server --bin divixi-server
```

Node 22.18+, as `engines` in package.json says. Not just vite's floor:
`npm test` runs `node --test` over `.ts` files directly, which needs Node to
expand the glob (21+) and to strip types without a flag (22.18+). On an older
Node the other three checks pass and that one fails with
`Unknown file extension ".ts"`.

`tauri-build` reads the built UI from `dist/`, which is not in git, so on a fresh
clone every cargo command needs `npm run build` to have run once. `npm run app`
does it for you; a bare `cargo test` does not.

`divixi-server` carries the built UI inside the binary, since that is what a
phone loads from it, so build it after `npm run build` and again after a UI
change. Its `server` feature turns on Tauri's `custom-protocol` for that.
Without it Tauri builds in development mode, embeds nothing, and reads `dist/`
from the build machine's checkout at run time: the binary works where it was
built and answers every page with "no such thing" anywhere else.
`scripts/check-server-ui.sh target/debug/divixi-server` runs a built binary
away from the checkout and checks that the UI comes back, as CI does on Linux.

The checks, which are the same five CI runs (`.github/workflows/ci.yml`):

```bash
npm run check                        # UI types (svelte-check)
npm test                             # UI unit tests (node --test)
npm run check:i18n                   # every string exists in both ko and en
npm run check:readme                 # README.md and README.ko.md keep the same shape
cargo test --workspace               # Rust (a couple of tests are Windows-only)
```

Useful examples:

```bash
cargo run -p orchestra-acp --example smoke -- "Reply with exactly: OK"
cargo run -p orchestra-agents --example detect                # what detection finds
cargo run -p orchestra-agents --example prompt -- codex "…"   # one run on one agent
cargo run -p orchestra-agents --example adapters -- <folder>  # install the npm adapters
```

## The dev app beside the installed one

`npm run app` starts a debug build that runs **alongside** an installed
DIVIXI. Nothing to set up: start it with the app already open and a second
window comes up, titled **DIVIXI (dev)**.

Two things make that work, and both are debug-only — a release build behaves
exactly as it always did.

- **No single-instance handover.** A release build hands over to the running
  DIVIXI and exits (`one_instance_only` in `src-tauri/src/lib.rs`), which is
  what a second launch of an app that lives in the tray should do. It is also
  what made `npm run app` look like it did nothing at all: it started, found
  the installed app, showed *that* window and quit.
- **Its own data folder.** A debug build works in `<app data>-dev`, beside
  the installed app's folder rather than in it:

  | | installed | dev |
  | --- | --- | --- |
  | Windows | `%APPDATA%\app.divixi` | `%APPDATA%\app.divixi-dev` |
  | macOS | `~/Library/Application Support/app.divixi` | `…/app.divixi-dev` |
  | Linux | `~/.local/share/app.divixi` | `…/app.divixi-dev` |

  Everything hangs off that one path, so all of it is separate: the store
  (`divixi.db`), the logs and the crash file (`logs/`), the downloaded ACP
  adapters, the workers' checkouts, the knowledge library and the
  remote-access keys. Two instances on one folder would share a SQLite file
  and interleave their lines in one log.

A dev instance therefore starts empty — no tracks, no agent detection, no
settings. That is the point: it cannot touch the work in the installed app.
To try a change against real data, copy the folder across first (with both
apps closed), or point the dev build at a copy:

```bash
DIVIXI_DATA_DIR=/path/to/a/copy npm run app     # bash
$env:DIVIXI_DATA_DIR = "C:\tmp\divixi-try"; npm run app   # PowerShell
```

`DIVIXI_DATA_DIR` overrides the folder outright, in release builds too, and
the `divixi-server` subcommands read the same setting. `DIVIXI_DB` still
moves just the store, on top of whichever folder is chosen.

Two copies of DIVIXI will happily run the same agent CLIs at the same time. Watch the
disk: each worker checkout is a full copy of the repository.

## Things to know

- **The product name is `DIVIXI`; everything that identifies the app is still lowercase.**
  `productName` names the window, the installers (`DIVIXI_<version>_x64-setup.exe`), the
  install folder, the shortcuts and the macOS `.app`. The identifier (`app.divixi`, and with
  it the data folder), the binaries (`divixi`, `divixi-server`), `~/.divixi`, the
  localStorage keys and the environment variables did not change with it, so data and
  remote servers carry over. The MSI's `upgradeCode` in `tauri.conf.json` is pinned to the
  one Tauri derived from the old name `Divixi` (`npx tauri inspect wix-upgrade-code`):
  Tauri derives it from `productName` case-sensitively, and without the pin a new MSI
  would install beside the old one instead of replacing it. Do not change or remove it.
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
- **An app opened from Finder does not get your shell's PATH.** launchd starts it with
  `/usr/bin:/bin:/usr/sbin:/sbin`, where there is no `node` for the adapters, so on macOS
  the app takes the login shell's PATH at startup (`src-tauri/src/shell_path.rs`).
  `npm run app`, and `open` from a terminal, hand the shell's PATH through: to see what a
  user sees, open the bundle from Finder.
- **Agents run in their most autonomous mode.** Humans see the escalations agents raise,
  not tool approvals. Per-track settings can choose otherwise.
- **The event store.**
  - Events are appended to `events`. `runs` is their fold when a run ends, and the
    timeline reads only `runs`.
  - A run left open by a crash is closed as failed on the next start.
  - The database is `divixi.db` in the data folder, and `DIVIXI_DB` overrides it.
- **Text chunks from agents are sent in 40 ms frames**, to the webview and to the store
  alike.
