# Security Policy

Divixi launches coding-agent CLIs on your machine, holds the tokens those agents and the
remote bridge need, and opens SSH tunnels to other hosts. So a private way to report a
flaw is not decoration here.

## Reporting a vulnerability

**Please do not open a public issue.**

Report it at
**[Security → Advisories → Report a vulnerability](https://github.com/jaemin93/divixi/security/advisories/new)**.
That is GitHub's private vulnerability reporting: the report is visible only to the
maintainers, the discussion happens in the advisory, and it becomes public only when a
fix is published — with credit to you, unless you ask otherwise.

If that page tells you reporting is not available, the feature has not been switched on
yet. Open an issue saying only *"I would like to report a security issue privately, please
enable private vulnerability reporting"* — no details — and wait for the channel before
sending anything.

What helps, in rough order:

- What an attacker gets out of it, and what they need first (local access? a network
  path? a malicious repository? a hostile agent?).
- Steps to reproduce, and the version — the version block from **Settings → About →
  Diagnostics** is ideal.
- Your platform. Divixi's exposure is not the same on Windows, Linux and macOS.
- Whether the flaw is in Divixi itself or in something it launches or downloads (an ACP
  adapter, an agent CLI). Both are worth reporting; the fix is in a different place.

**Please redact your own secrets** before attaching a log. `*Copy diagnostics*` masks
what looks like a key or a token, and so does the log — but a hand-pasted terminal
transcript does not.

This is a small project with one maintainer. Expect an acknowledgement within about a
week. There is no bug-bounty programme, no embargo period we can promise to hold to, and
no security release train: a fix lands on `main` and goes out in the next release.

## Supported versions

Divixi is pre-1.0. Only the **latest release** gets fixes, and `main` is where they land
first. Older releases are not patched.

## In scope

- The desktop app and `divixi-server`: the Tauri command surface, session handling, the
  event store, the in-process MCP server the conductor's tools are bound to.
- **The remote path.** The supervised SSH tunnel, the token-auth scheme a server uses to
  decide that a caller is its owner, the `127.0.0.1` default, the remote webview bridge.
  Anything that lets someone who is not the owner reach a `divixi-server`, or lets one
  Track reach another's tools, is in scope.
- **Secrets that escape.** Divixi masks values that look like a key or a token before
  anything is written to the log; the diagnostics report deliberately reads no settings
  and no environment; nothing that holds a secret derives `Debug`. A token that still
  ends up in `divixi.db`, in the log, in `crash.log`, or in the diagnostics report is a
  vulnerability — report it here, not in an issue.
- Anything that lets a repository you merely *open* in Divixi run code without your
  going along with it.

## Not vulnerabilities

Two of Divixi's design decisions look alarming from the outside and are deliberate:

- **Agents run in their most autonomous mode.** Divixi's whole purpose is to let a
  conductor agent delegate work to worker agents that edit files and run commands in the
  folder you pointed them at. An agent doing something destructive because it was asked
  to, or because it misunderstood, is the risk you take by running coding agents at all —
  per-Track settings can tighten it. Divixi is not a sandbox and does not claim to be.
  Point it at a folder you would let an agent loose in.
- **A server listening on every network is plain HTTP.** This is documented in the
  README and in [docs/divixi-server.md](docs/divixi-server.md): the default is
  `127.0.0.1`, and binding wider is for use over Tailscale or a network you already
  trust. That it is unencrypted when you ask for it is not a flaw; a way past the owner
  check *is*.

Also not for this channel: bugs in the agent CLIs themselves (Claude Code, Codex,
Antigravity), in their npm ACP adapters, or in what their providers do with your prompts.
Report those upstream. If Divixi's use of one of them makes an upstream flaw worse, that
part is ours.

Divixi sends no telemetry. Nothing about you or your use goes anywhere, so there is no
collection pipeline to attack.

## Fixes

A security fix is an ordinary commit with an ordinary message; it does not announce what
it closes until the advisory is published. When one is, the advisory says which versions
are affected, what to do if you cannot upgrade, and who reported it.
