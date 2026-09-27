<p align="center">
  <img src="docs/images/divixi.png" alt="" width="120" height="120">
</p>

<h1 align="center">Divixi</h1>

<p align="center"><strong>One conductor, many agents.</strong></p>

<p align="center"><strong>English</strong> | <a href="README.ko.md">한국어</a></p>

A desktop app for directing coding agents: you talk to one conductor, and it divides the
work among worker sessions that run side by side.

Divixi drives the agents you already use (Claude Code, Codex, GitHub Copilot, Antigravity)
over the [Agent Client Protocol](https://agentclientprotocol.com), under your own logins.
Each piece of work is a **Track**: a folder, a long-lived conductor session, and the workers
it opens. What reaches you is what needs you: decisions to make, reports to read, changes
to merge. Transcripts and tool calls stay one click away.

The name comes from *divisi*, the score marking that splits one section into independent
parts. The x is for crossing providers.

## Features

- **Conductor and workers.** The conductor plans and delegates. Workers run in parallel
  in their own folders or git worktrees, report back in a set shape (what changed, what
  was checked, what is open), and you merge their changes file by file.
- **Decisions, not prompts.** Agents run in their most autonomous mode. What needs you
  arrives as a decision card in the conversation, and a bell at the top right keeps the
  rest.
- **Context beside the chat.** A file panel with search, previews and editing. A terminal.
  Design boards for notes, sketches, frames and references. A knowledge library of PDFs,
  Office files and notes that you pull into a message with `@kb`.
- **Remote instances.** Run `divixi-server` on a Linux box and switch to it from the top
  left of the window, as in VS Code Remote. Its agents, files and terminal run there.

## Install

Divixi is early software. Windows is the main platform, `divixi-server` runs on Linux,
and macOS is untested.

**You need:**

- **Node.js 22.18 or later.** The Claude Code and Codex adapters are Node programs.
  Divixi installs them into its data folder on first use.
- **At least one agent, installed and signed in:**

| Agent | Install | Sign in |
|---|---|---|
| Claude Code | [claude.com/claude-code](https://claude.com/claude-code) | run `claude` once |
| Codex | [openai.com/codex](https://openai.com/codex) | `codex login` |
| GitHub Copilot | [github.com/github/copilot-cli](https://github.com/github/copilot-cli) | `copilot login` |
| Antigravity | [antigravity.google/cli](https://antigravity.google/cli) | `agy login` (Divixi fetches its ACP server) |

On first launch, Divixi finds the agents you have and checks that each one can open a
session.

**From source** (Rust stable, Node.js 22.18+):

```bash
git clone https://github.com/jaemin93/divixi
cd divixi
npm install
npm run app                                     # development build, with a window
npx tauri build                                 # installer in target/release/bundle
```

## Remote instances

`divixi-server` is Divixi without a window, for a server. The desktop app reaches it over
an SSH tunnel, or at an address with your GitHub account. It starts the server over SSH
when it is not running.

See **[docs/divixi-server.md](docs/divixi-server.md)** to build, install, connect, keep it
running, and update it.

## Data and privacy

- **No telemetry.** Divixi sends nothing about you or your use anywhere — nothing
  reported, counted or phoned home. The only request Divixi makes of a server is the
  update check below, and only when you ask for it.
- **Checking for updates.** Pressing *Check for updates* in **Settings → About** asks the
  public GitHub API for the newest published release of this repository. The request
  carries no sign-in, no version, no platform, and nothing that identifies you or this
  machine. There is no check at startup and none on a timer. Divixi never downloads or
  installs anything by itself: it tells you a release exists and opens its page in your
  browser.
- **Where your data lives.** Tracks, conversations, boards and the knowledge library are
  in the app's data folder: `%APPDATA%\app.divixi` on Windows, `~/.local/share/app.divixi`
  on Linux.
- **What reaches providers.** Your prompts go to the agents you run. They send them to
  their providers under your own sign-in, as they would from a terminal.
- **The log stays on your machine.** Warnings and errors go to `logs/` in the same data
  folder, a file a day, the last seven kept; a panic goes to `logs/crash.log`. Values that
  look like a key or a token are hidden before anything is written. Nothing is sent
  anywhere: a bug report is yours to paste.
- **Remote instances.**
  - A server listens on `127.0.0.1` by default, and only its owner gets in.
  - Listening on every network is plain HTTP. Use it over Tailscale or a network you trust.
  - Your GitHub token is used only to prove to your own server that it is you.

## Contributing

Start with **[CONTRIBUTING.md](CONTRIBUTING.md)**: how to report a bug, how to set up,
the checks to run, and what to know before opening a pull request.
[docs/development.md](docs/development.md) is the developer guide — how the code is laid
out, the principles it is written to, and the traps worth knowing about.

Everyone taking part is covered by the [Code of Conduct](CODE_OF_CONDUCT.md). For a
security vulnerability, do not open a public issue: [SECURITY.md](SECURITY.md) has the
private route.

### Reporting a bug

Open **Settings → About → Diagnostics** and press *Copy diagnostics*. That puts the
version, your OS, the agents Divixi found and the last warnings and errors on the
clipboard as one Markdown block; it is shown on screen first, so you can read it before
pasting. Attach it to a
[bug report](https://github.com/jaemin93/divixi/issues/new?template=bug_report.yml).

[CONTRIBUTING.md](CONTRIBUTING.md#reporting-a-bug) has the rest: where the log lives, and
how to turn the logging up before you reproduce the problem.

## Acknowledgements

### Kiro Crew

Much of Divixi's interface was designed by studying
[Kiro Crew](https://github.com/kirodotdev/KiroCrew), an open-source agent workspace by
AWS (Apache License 2.0, Copyright Amazon.com, Inc. or its affiliates).

What we took from it:

- **Layout and interaction.** The left rail and session panel, the right-hand working
  folder panel, the agent picker, the instance switcher in the title bar, the
  notification bell, the session filter and context menus, and the terminal panel were
  all modelled on Kiro Crew's. These are ideas and arrangements, reimplemented from
  scratch in Svelte and Rust — no code, stylesheet, icon, font, or image was copied.
- **The knowledge library.** Our chunking strategy, retrieval fusion, and the LLM
  extraction prompt in `crates/knowledge/` follow Kiro Crew's `knowledge` module. The
  extraction prompt is adapted from Kiro Crew's `extractor.py` under the Apache License
  2.0, with two rules of ours added; see [NOTICE](NOTICE). Our implementation is new
  Rust code and works without local embeddings.
- **Remote instances.** The idea of running a headless server on another machine and
  switching to it from the app comes from Kiro Crew's remote instances, as do the
  supervised SSH tunnel options. Our implementation differs (a native webview rather
  than an iframe).

Where Divixi goes its own way: one conductor agent directing worker agents, decision
cards instead of prompt-by-prompt approval, and a knowledge library that needs no local
embedding model.

**Divixi is an independent project. It is not affiliated with, endorsed by, or sponsored
by Kiro, AWS, or Amazon. "Kiro" and "Kiro Crew" are trademarks of Amazon.com, Inc. or
its affiliates, used here only to describe the origin of ideas Divixi borrowed.**

## License

Divixi is licensed under the [Apache License, Version 2.0](LICENSE). See
[NOTICE](NOTICE) for attribution required by that license.
