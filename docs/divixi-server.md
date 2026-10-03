# Installing divixi-server (Linux server)

**English** | [한국어](divixi-server.ko.md)

`divixi-server` is **DIVIXI without a screen**. It runs Tracks, conductors, workers, knowledge and
terminals from the same code as the app, and a DIVIXI app on another PC attaches to it as a
**remote instance**. Pick it from the menu at the top left of the window and that server fills the
same window.

The steps below were checked on Ubuntu 24.04 (x86_64) with Rust 1.96.

---

## 0. The app can do this for you (x86_64 Linux)

Everything below is the manual path, and it still works. But for an **x86_64
Linux** machine reached over SSH, the app installs and updates divixi-server
itself, the way an editor's remote extension does: no apt packages, no Rust,
no Node, and nothing built on that machine. It fetches the release binary.

In the app, the instance's card in **Settings › Remote instances** says what it
found on that machine, and so does the page shown when an instance could not be
reached. It asks before it writes anything — the binary is about 207 MB — and an
update can be told "don't ask again" per instance.

What it does there:

```bash
~/.divixi/server/v2026-09-30/divixi-server   # the release binary
~/.divixi/server/current -> v2026-09-30      # the pointer the app moves
```

- The machine fetches it from GitHub itself with `curl` or `wget` where it can;
  where it cannot (no fetcher, or no route out) the app streams the bytes down
  and pushes them over the SSH connection. Which of the two is happening is on
  screen, because one is much slower than the other.
- The download lands on `divixi-server.part` and is only renamed into place once
  its **SHA-256** matches the digest GitHub keeps for that asset. A download that
  drops halfway is deleted, never installed.
- The old server is stopped, the new binary is renamed in, the pointer moves in
  one step, and the server is started again — in that order.
- The instance's **divixi-server path** setting is then pointed at
  `~/.divixi/server/current/divixi-server`.
- `systemd`: if a user service for divixi-server is enabled there, the app uses
  it to stop and start. If that unit's `ExecStart` names some other binary, the
  app **stops** and tells you to point it at the pointer above — installing
  behind it would leave systemd launching the old one.

The manual steps are the way for everything else: arm64, macOS, a machine with
no SSH server, or a server you want built from your own checkout (which is the
only way to make its build match an app you built yourself).

## 1. What you need

**System packages (for building).** Building the server compiles Tauri too, so Tauri's Linux build
packages are needed. The executable itself only uses gtk3 at run time; no display (X, Wayland) is
required.

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

**Rust** (rustup):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

**Node.js 22.18 or later, and npm.** It builds the UI, and it fetches and runs the ACP adapters for
Claude Code and Codex, which are JS programs. nvm or a distribution package, either is fine.

**Agent CLIs.** Install the ones you want to use on the server, as that server's user, and **sign in
once**. DIVIXI uses that sign-in as it is.

| Agent | Install | Sign in |
|---|---|---|
| Claude Code | https://claude.com/claude-code | run `claude`, then sign in |
| Codex | https://openai.com/codex | `codex login` |
| GitHub Copilot | https://github.com/github/copilot-cli | `copilot login` |
| Antigravity | https://antigravity.google/cli | `agy login` (fetch the ACP server from the app) |

---

## 2. Getting the source

**Build from the same commit as the app.** If the server comes from different code, the app keeps
showing "this build differs from the app" in the host list and in the chip at the top left
(section 7). The comparison is not a version string but a **hash of the Rust source**
(`src-tauri/src`, `crates`), so two builds that both say `0.1.0` still disagree when the commits
differ. A difference in the UI alone (`ui/`) does not matter.

`git clone` gives you the **default branch (main)**. If the app came from a release, the app is a
**tag** build, and a single Rust commit that landed on main after the tag is enough to disagree.
This does happen: four minutes after the tag `v2026-09-28`, one dependency commit landed on main,
so the app built from the tag reported `71d291569a81` while a server cloned from main that same day
reported `353c7bc7a8f4`.

**When the app is a release build** — ask for that release's tag.

```bash
# on the server (the tag of the release the app came from)
git clone --branch v2026-09-28 --depth 1 https://github.com/jaemin93/divixi ~/divixi-src
```

**When you built the app yourself** — send the checkout on your PC as it is. You need not find the
commit, and the server need not reach GitHub. Changes you have not pushed go too.

```bash
# on the PC (in the repository folder, at the commit the app was built from)
git archive HEAD | ssh user@server 'mkdir -p ~/divixi-src && tar -x -C ~/divixi-src'
```

**When both are main** — if the app was built from main as well, just clone it.

```bash
# on the server
git clone https://github.com/jaemin93/divixi ~/divixi-src
```

Updating one side only puts them out of step again. See section 7.

---

## 3. Building and installing

```bash
cd ~/divixi-src
npm ci                 # UI dependencies
npm run build          # the UI (it goes inside the executable)
cargo build --release -p orchestra-app --features server --bin divixi-server
install -Dm755 target/release/divixi-server ~/.local/bin/divixi-server
```

- The first build takes a few minutes; later ones are around thirty seconds.
- `~/.local/bin` is the default install path. Put it elsewhere if you write that path into the
  instance settings in the app.
- There are five commands.

```bash
divixi-server                  # = serve. Run the server (127.0.0.1:7488 by default)
divixi-server token            # print a pairing token once (5 minutes, single use; the app calls this over SSH)
divixi-server owner <login>    # the GitHub account allowed in over an address (--none: nobody)
divixi-server listen all|local # accept from every network / only from this server (SSH tunnel) — restart to apply
divixi-server help
```

---

## 4. Choosing how to connect

### A. SSH tunnel (recommended, nothing to configure)

The server accepts only on `127.0.0.1`, and the app opens a tunnel over SSH. Whoever can get in over
SSH is treated as the owner.

1. `ssh user@server` must work from your PC **without asking for a password** (a key plus
   ssh-agent, or an alias in `~/.ssh/config`). The app runs SSH in BatchMode, so it cannot ask you
   for a password or to confirm a host key. For a server you are reaching for the first time, run
   `ssh user@server` once in a terminal on your PC to take the host key.
2. In the DIVIXI app, press **Settings › Remote instances › Add remote instance**, fill it in as
   below, and save.
   - Connection: SSH tunnel
   - SSH host: `user@server`
   - Remote port: `7488`
   - divixi-server path: `~/.local/bin/divixi-server`
   - Remote PATH: only when the agents or node cannot be found. For example:
     `~/.local/bin:~/.nvm/versions/node/v20.20.2/bin`
3. Pick that instance from the menu at the top left. The app opens the tunnel, takes a pairing
   token over SSH, and signs in. It does **not** start a server that is down while it connects: it
   reports that nothing answered through the tunnel and shows a **Start it** button on that
   instance's card, which starts the server over SSH. To have one up without pressing anything,
   keep it running (section 5).

### B. A direct address (when there is no SSH server, as on a Windows PC; GitHub account)

The server accepts from the network directly, and only an app signed in with **the same GitHub
account as the server's owner** gets in. The address is unencrypted http, so use it **only over a
private network such as Tailscale**.

```bash
divixi-server owner <my-github-login>
divixi-server listen all
pkill -x divixi-server; setsid -f ~/.local/bin/divixi-server serve >/dev/null 2>&1 </dev/null
```

- If you run a firewall, open 7488/tcp towards the private network.
- In the app, go to Settings › Remote instances and sign in to **GitHub** first. It needs an OAuth
  app Client ID; follow what the screen says.
- Then, under **Add remote instance**, set the connection to **Direct address** and the address to
  `http://<Tailscale IP>:7488`.
- The desktop DIVIXI becomes a server the same way, through "Open this PC as a remote instance" in
  Settings.

---

## 5. Keeping it running

**Starting and stopping it yourself.** Connecting does not start a stopped server (section 4A);
the app shows a **Start it** button for that. These are the same two commands from a shell.

```bash
setsid -f ~/.local/bin/divixi-server serve >/dev/null 2>&1 </dev/null   # start (survives the SSH session)
pkill -x divixi-server                                                  # stop
```

**Always on (optional): a systemd user service.**

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/divixi-server.service <<'EOF'
[Unit]
Description=DIVIXI server (remote instance)
After=network-online.target

[Service]
ExecStart=%h/.local/bin/divixi-server serve
# where the agent CLIs and node are (with nvm, add that bin directory too)
Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin
Restart=on-failure

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now divixi-server
sudo loginctl enable-linger "$USER"     # start at boot without signing in
```

- Nothing the app does starts a second server: the line it runs over SSH is guarded by `pgrep`, and
  the **Start it** button checks first too. So a server kept up by systemd does not end up with two
  of them.
- After changing `listen` or `owner`, restart it with `systemctl --user restart divixi-server`.

---

## 6. What happens on the first start

- It creates the data folder `~/.local/share/app.divixi/` (under `$XDG_DATA_HOME` when that is set).
  - `divixi.db`: Tracks, conversations, settings
  - `remote.key`: the token signing key
  - `adapters/`: the ACP adapters
- It detects the agents by itself.
- It **fetches the ACP adapters** for Claude Code and Codex into `adapters/` on its own. That needs
  npm, and until they arrive it falls back to npx.
- Antigravity's ACP server is fetched from the agent settings in the app, with "download".

---

## 7. Updating

Do sections 2 and 3 again (get the source, build, install) and restart the server.

```bash
pkill -x divixi-server            # with systemd: systemctl --user restart divixi-server
```

When the server's build differs from its own, the app puts a **"!"** on the chip at the top left and
says "update the remote DIVIXI". The comparison is made against the Rust code, so it does not appear
for an app update that only changed the UI. Put the app and the server on the **same commit** and it
goes away — section 2 has the ways to do that. If you updated only the app, build the server again
from that commit.

---

## 8. Troubleshooting

| What you see | What to check |
|---|---|
| `ssh ended … Host key verification failed` | Run `ssh user@server` once in a terminal on your PC to take the host key |
| `ssh ended … Permission denied` | Whether you get in without a password (key, ssh-agent) |
| `divixi-server token gave no link` | That the path is right (`~/.local/bin/divixi-server`) and that it is executable |
| `did not answer through the tunnel` | Usually the server is not running: press **Start it** on that instance's card. If it will not come up, run `~/.local/bin/divixi-server serve` yourself (you will see the log) |
| An agent shows as "not installed" | PATH on the server. Add the CLI and node directories to "Remote PATH" in the instance settings, or to `Environment=PATH` in the systemd unit |
| A direct address says "no owner yet" | Run `divixi-server owner <github login>` on the server |
| A direct address says "… does not own this DIVIXI" | Whether the app's GitHub account is the same as the server's owner |
| A remote shell is left behind | When the app dies, the server closes that machine's shells two minutes later |
| "this build differs from the app" | Whether the server and the app came from the same commit (section 2). If you updated one side only, see section 7 |

**To see the log**, run the server in the foreground.

```bash
pkill -x divixi-server
RUST_LOG=info,orchestra_app=debug ~/.local/bin/divixi-server serve
```
