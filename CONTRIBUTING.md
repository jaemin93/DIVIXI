# Contributing to Divixi

Thanks for being here. Divixi is a young project, and the fastest way to help is still
the simplest: report a bug you hit, or fix one that annoys you.

- [Reporting a bug](#reporting-a-bug)
- [Asking for a feature](#asking-for-a-feature)
- [Before you start something large](#before-you-start-something-large)
- [Setting up](#setting-up)
- [The checks](#the-checks)
- [Every string in both languages](#every-string-in-both-languages)
- [Adding a top-level directory](#adding-a-top-level-directory)
- [Code style, and the formatters we do not gate on](#code-style-and-the-formatters-we-do-not-gate-on)
- [Commit messages](#commit-messages)
- [Working with AI tools](#working-with-ai-tools)
- [Opening a pull request](#opening-a-pull-request)
- [What CI tells you, and what it does not](#what-ci-tells-you-and-what-it-does-not)
- [Licensing and attribution](#licensing-and-attribution)
- [Conduct and security](#conduct-and-security)

## Reporting a bug

Search [the open issues](https://github.com/jaemin93/divixi/issues) first; the quickest
answer is usually a thread that already exists. Then open a
[bug report](https://github.com/jaemin93/divixi/issues/new?template=bug_report.yml).

**Attach the diagnostics.** Open **Settings → About → Diagnostics** and press *Copy
diagnostics*. That puts the version, your OS and webview, the agent CLIs it found and
their versions, where the log and the store live, and the last warnings and errors on
your clipboard as one Markdown block. It is shown on screen before it is copied, so you
can read it first — no API key or token goes in it, but the paths on your machine do.

Two more things in the same pane:

- *Open log folder* opens the log itself, when the tail in the report is not enough. It
  is `logs/` in the app's data folder (`%APPDATA%\app.divixi` on Windows,
  `~/.local/share/app.divixi` on Linux, `~/Library/Application Support/app.divixi` on
  macOS), one file a day, the last seven kept, with a panic in `crash.log` beside them.
- **How much is logged** turns the level up without a restart, so whatever you were
  reproducing is still on screen. `DIVIXI_LOG` sets the level the app *starts* at and
  takes the same filters as `RUST_LOG`; the setting comes after it. Debug and above also
  record what the agents write to stderr, so turn it back down when you are done.

The diagnostics pane belongs to the machine the app runs on, so it is not shown while
you are switched to a remote instance. Switch back to the local one, or collect the log
on the server.

## Asking for a feature

Open a
[feature request](https://github.com/jaemin93/divixi/issues/new?template=feature_request.yml)
and lead with the problem, not the design. What you were trying to do and what got in
the way tells a maintainer more than a proposed solution, and it leaves room for an
answer nobody had thought of.

## Before you start something large

For a typo, a crash, or a one-file fix, just send the pull request.

For anything bigger — a new surface, a changed default, something other parts of the app
would have to build around — open an issue first and get a reaction to the approach.
Nobody enjoys declining a finished pull request that went the wrong direction, and the
answer usually fits in a paragraph. `git log` is the fastest way to see why something is
the shape it is; the commit bodies carry the reasoning.

## Setting up

[docs/development.md](docs/development.md) is the developer guide: the crate layout, the
principles the code is written to, the examples worth running, and the traps worth
knowing about. Read it once. The short version:

```bash
npm install
npm run build          # once, before any cargo command
npm run app            # the app, in development mode
```

- **Node 22.18+**, as `engines` in `package.json` says — and not because of Vite.
  `npm test` runs `node --test` straight over `.ts` files, which needs Node to expand the
  glob (21+) and to strip types without a flag (22.18+). On anything older the other
  three checks pass and that one fails with `Unknown file extension ".ts"`.
- **Rust stable, 1.85 or newer** (`rust-version` in the workspace `Cargo.toml`).
- **Tauri 2's own prerequisites** for your platform — see
  [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites). On Debian or
  Ubuntu, CI installs `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
  libayatana-appindicator3-dev librsvg2-dev pkg-config libssl-dev patchelf`, which is a
  working list to copy.
- **`npm run build` before your first cargo command.** `tauri-build` reads the built UI
  from `dist/`, and `dist/` is not in git, so on a fresh clone a bare `cargo test` fails
  without it. `npm run app` does it for you; cargo does not.

One trap that costs people an hour: **running Divixi from inside a Claude Code session
breaks Claude Code workers**, because Claude Code sees the inherited `CLAUDECODE` /
`CLAUDE_CODE_*` variables as a nested session and exits at once. The app clears them at
the top of `main`; if you are calling into the crates yourself, do the same.

## The checks

The five CI runs, all runnable locally:

```bash
npm run check          # UI types (svelte-check)
npm test               # UI unit tests (node --test)
npm run check:i18n     # every string exists in both ko and en
npm run check:readme   # the two READMEs still have the same shape
cargo test --workspace # Rust (a couple of tests are Windows-only)
```

`check:readme` compares `README.md` and `README.ko.md`: the same headings at the same
levels in the same order, relative links that resolve, the language switcher present in
both, and matching, balanced code fences. It does not compare prose — the two languages
are not expected to split a thought the same way. So **a pull request that edits only one
of the READMEs fails CI** if it adds, removes or moves a section, or leaves a link
pointing at something that is gone.

CI also builds, so it is worth knowing these pass too:

```bash
cargo clippy --workspace --all-targets
npm run build
cargo build -p orchestra-app --bin divixi
cargo build -p orchestra-app --features server --bin divixi-server
```

`npx tauri build` produces an installer, if you want to check the bundle.

## Every string in both languages

The interface is Korean and English, and **`ko` and `en` must hold exactly the same
keys**. Both dictionaries are in `ui/src/lib/i18n.svelte.ts`: `ko` first, then `en`. Add
a UI string to one and you must add it to the other.

`npm run check:i18n` is a gate on every pull request. It catches a key only one side
has, a key written twice in the same dictionary (where the second silently wins), and it
prints the counts, which is what you want to see after adding a dozen strings. `npm run
check` catches some of this through the types, but not the case where `en` has a key
`ko` does not — so run both.

If you do not write Korean, add the key to `ko` with your best attempt, or with the
English text, and say so in the pull request. A missing key ships an untranslated label
to whoever runs the other locale; a rough translation you flagged is easy for someone to
fix.

## Adding a top-level directory

Read [.gitignore](.gitignore) before you add a directory at the repository root.

Divixi's "a folder each" worker mode gives a worker its own folder directly under the
track folder — which, when you develop Divixi with Divixi, is this repository root —
named after the worker. There is no prefix or parent folder to match on, and a new
worker adds a new root-level directory at any time. So `.gitignore` ignores **every**
directory at the root and names the ones that belong to the project:

```
/*/
!/.github/
!/crates/
!/docs/
!/scripts/
!/src-tauri/
!/ui/
```

**If your change adds a real top-level directory, add a `!` line for it in the same
commit, or git will not see your files.** `git status` will show nothing, `git add` will
refuse, and the pull request will arrive with the directory missing. Root-level *files*
are deliberately left alone, so a new file next to this one shows up normally.

## Code style, and the formatters we do not gate on

Match the file you are editing. That is most of it.

Two things surprise people, so they are written down here rather than left to be
discovered in review:

- **`cargo fmt --check` is not a gate, and `rustfmt.toml` is a description rather than
  an instruction.** The code was not developed under rustfmt. `max_width = 200` is the
  width the code actually uses, but even at that width about 300 hunks remain — nearly
  all of them rustfmt wanting to join lines the author left expanded. The measurements
  are in the comments in [rustfmt.toml](rustfmt.toml).
- **Prettier is a devDependency with no gate.** Same reason, for the UI.

So: **please do not send a pull request that reformats files you are not otherwise
changing.** Reformatting the tree is a deliberate, separate step, and it has to happen
in a commit with nothing else in it — otherwise every later `git blame` on those files
lands on the reformat instead of on the change that matters. The gate gets added in the
same commit that reformats. Running your formatter over a file you are already
rewriting is fine; the reviewer will tell you if the diff got hard to read.

One more house rule, because a test enforces it rather than a linter: **nothing that
holds a secret derives `Debug`**. Tokens, keys and credentials get a hand-written
`Debug` that prints a placeholder, and what goes to the log is masked first
(`src-tauri/src/mask.rs`).

## Commit messages

Divixi does not use Conventional Commits. A subject line is a sentence saying what the
commit does, in the present tense, with no type prefix:

```
Instances keep the order you put them in, and pins go
An agent's whole process tree goes down on Unix, not just its shell
Notifications: one rule for what is worth interrupting for
```

`git log` is the reference. The body is prose, wrapped at about 72 characters, and it
explains **why** — what was wrong, what was considered, what was deliberately not done.
The commits in this repository run long by most standards; you do not have to match
that, but a body that only restates the subject is a wasted one.

One logical change per commit. If your pull request is five commits and each is a
separate idea, leave it that way; if it is five commits of "fix review comment", squash
them.

## Working with AI tools

Use them. Divixi exists to run coding agents, and this repository is developed with them
— the `Co-Authored-By` lines in `git log` are there on purpose. Nobody here will ask you
to pretend otherwise, and no pull request is judged by which tools helped write it.

What does not change is who answers for it:

- **Understand the diff.** All of it, not only the part you asked for. If you cannot say
  why a line is there, it is not ready to send.
- **Answer review yourself.** A reviewer's question is a request for your reasoning.
  Pasting a fresh round of generated text back moves the work onto them, and the thread
  gets longer instead of shorter.
- **Run [the checks](#the-checks) on your own machine.** An agent reporting a green run
  is not a green run. This is the one we act on: a pull request whose checks were never
  actually run comes back, and a tool having written it does not change that.

Watch the size, too. A large mechanically generated diff is cheap to produce and
expensive to read, and the cost lands on whoever reviews it — so the rule for
[reformatting](#code-style-and-the-formatters-we-do-not-gate-on) holds for any sweeping
change: ask in an issue first, and keep one idea per pull request.

One thing worth naming: a generated patch can reproduce code from somewhere else without
saying so. If you recognise something as coming from another project, it belongs in
[NOTICE](NOTICE) like anything else — see
[Licensing and attribution](#licensing-and-attribution).

## Opening a pull request

1. Fork, then branch from `main`:
   ```bash
   git fetch origin
   git checkout -b my-change origin/main
   ```
2. Make the change, and add or update tests. The UI tests are plain functions under
   `ui/src/lib/**/*.test.ts`; Rust tests live beside the code they cover.
3. Update the docs in the same commit if you changed documented behaviour. A doc nobody
   updated is worse than no doc, because readers still trust it. [README.md](README.md)
   and [README.ko.md](README.ko.md) are one document in two languages — change one and
   change the other in the same commit. `npm run check:readme` gates their shape, so a
   one-sided edit that touches a section or a link is caught; whether the two say the
   same thing is still on you and the reviewer.
4. Run [the checks](#the-checks).
5. Push, and open a pull request against `main`. The
   [template](.github/PULL_REQUEST_TEMPLATE.md) asks which checks you ran; answering
   honestly, including "did not run" with a reason, is more useful than ticking
   everything.
6. Address review by pushing more commits to the branch.

If the change is user-visible, a screenshot or a short recording saves a round trip.
Attach it to the pull request rather than committing it.

## What CI tells you, and what it does not

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on every pull request: one
fast Node job (types, unit tests, i18n, the READMEs) that fails in about a minute, then a
build-and-test matrix on **Windows, macOS and Linux**, then an unsigned bundle on each.
Some of it is deliberately not a gate, and reading a green run without knowing that is
misleading:

- **macOS has never been verified on a real machine.** It compiles, tests and bundles in
  CI, and the bundle is unsigned — so it is refused by Gatekeeper on any machine that
  did not build it. Green on macOS means "it builds", not "it runs". If you are on a
  Mac, your report of what actually happens is genuinely valuable.
- **`cargo clippy` does not use `-D warnings`.** The existing warnings have not been
  triaged, and making them fatal would block every pull request for reasons unrelated to
  it. Errors still fail the step. Do not add new warnings; you do not have to clear old
  ones.
- **A "Remote-launch prerequisites" step records rather than asserts.** Divixi starts a
  remote instance with `setsid`, which Linux has and macOS does not. The step logs which
  tools are present, per platform.
- **Nothing CI builds is signed or notarized**, so the bundles it uploads are there to
  show that packaging works, not to install.

A red run on something your change cannot have touched does happen. Say so in the pull
request rather than chasing it.

## Licensing and attribution

Divixi is [Apache License 2.0](LICENSE). By opening a pull request you contribute your
work under that licence; there is no CLA.

[NOTICE](NOTICE) is the attribution Divixi owes under section 4 of the licence —
including what was adapted from [Kiro Crew](https://github.com/kirodotdev/KiroCrew).
**If your change brings in code, a prompt, or a data file from another project, say
where it came from in the pull request and add it to NOTICE.** Do not paste code whose
licence you have not checked. Third-party dependency licences are summarised in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md); a new dependency belongs there too.

## Conduct and security

Everyone taking part is covered by the [Code of Conduct](CODE_OF_CONDUCT.md).

**Do not open a public issue for a security vulnerability.** Divixi runs agent CLIs on
your machine, holds API tokens, and opens SSH tunnels, so there is real surface here.
[SECURITY.md](SECURITY.md) has the private route.
