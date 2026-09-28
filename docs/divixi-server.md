# divixi-server 설치 (Linux 서버)

`divixi-server`는 **화면 없는 Divixi**입니다. 앱과 같은 코드로 Track, 지휘자, 작업자, 지식, 터미널을 돌리고,
다른 PC의 Divixi 앱이 **원격 인스턴스**로 붙어서 씁니다. 창 왼쪽 위 메뉴에서 고르면 같은 창에 그 서버가 뜹니다.
설계는 [design/remote-access.md](design/remote-access.md)에 있습니다.

아래 순서는 Ubuntu 24.04(x86_64), Rust 1.96, Node.js 20에서 확인했습니다.

---

## 1. 준비물

**시스템 패키지(빌드용).** 서버 빌드도 Tauri를 컴파일하므로 Tauri의 Linux 빌드 패키지가 필요합니다. 실행
파일이 실제로 쓰는 것은 gtk3뿐이고, 화면(X, Wayland)은 필요 없습니다.

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

**Rust**(rustup):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

**Node.js 20 이상과 npm.** UI를 빌드할 때 쓰고, Claude Code·Codex의 ACP 어댑터(JS 프로그램)를 받고 실행할 때도 씁니다.
nvm이든 배포판 패키지든 상관없습니다.

**에이전트 CLI.** 서버에서 쓸 것을 그 서버 사용자로 설치하고 **한 번 로그인**해 둡니다. Divixi는 그 로그인을 그대로 씁니다.

| 에이전트 | 설치 | 로그인 |
|---|---|---|
| Claude Code | https://claude.com/claude-code | `claude` 실행 후 로그인 |
| Codex | https://openai.com/codex | `codex login` |
| GitHub Copilot | https://github.com/github/copilot-cli | `copilot login` |
| Antigravity | https://antigravity.google/cli | `agy login` (ACP 서버는 앱에서 받기) |

---

## 2. 소스 받기

**앱과 같은 커밋에서 빌드하세요.** 서버가 다른 코드에서 나오면 앱이 호스트 목록과 왼쪽 위 칩에
"빌드가 이 앱과 다릅니다"를 계속 띄웁니다(7절). 비교는 버전 문자열이 아니라 **Rust 소스
(`src-tauri/src`, `crates`)의 해시**라서, 둘 다 `0.1.0`이어도 커밋이 다르면 어긋납니다.
화면(`ui/`)만 다른 것은 상관없습니다.

`git clone`은 **기본 브랜치(main)**를 받습니다. 앱이 릴리스에서 받은 것이면 앱은 **태그** 빌드이므로,
태그 뒤에 main에 들어온 Rust 커밋 하나만으로도 어긋납니다. 실제로 그렇게 됩니다 — 태그
`v2026-09-28` 4분 뒤 의존성 커밋 하나가 main에 들어가서, 태그로 빌드한 앱은 `71d291569a81`,
그날 main을 clone해 빌드한 서버는 `353c7bc7a8f4`가 됐습니다.

**앱이 릴리스 빌드일 때** — 그 릴리스의 태그를 지정해 받습니다.

```bash
# 서버에서 (앱이 받아 온 릴리스의 태그)
git clone --branch v2026-09-28 --depth 1 https://github.com/jaemin93/divixi ~/divixi-src
```

**앱을 직접 빌드했을 때** — PC의 그 체크아웃을 그대로 보냅니다. 커밋을 찾을 필요도, 서버가
GitHub에 닿을 필요도 없습니다. 아직 밀지 않은 변경도 같이 갑니다.

```bash
# PC에서 (앱을 빌드한 그 커밋에 있는 상태로, 저장소 폴더 안에서)
git archive HEAD | ssh user@server 'mkdir -p ~/divixi-src && tar -x -C ~/divixi-src'
```

**둘 다 main일 때** — 앱도 main에서 빌드했다면 그냥 받습니다.

```bash
# 서버에서
git clone https://github.com/jaemin93/divixi ~/divixi-src
```

한쪽만 업데이트하면 다시 어긋납니다. 7절을 보세요.

---

## 3. 빌드와 설치

```bash
cd ~/divixi-src
npm ci                 # UI 의존성
npm run build          # UI (실행 파일 안에 들어감)
cargo build --release -p orchestra-app --features server --bin divixi-server
install -Dm755 target/release/divixi-server ~/.local/bin/divixi-server
```

- 처음 빌드는 몇 분 걸리고, 그다음부터는 30초 안팎입니다.
- `~/.local/bin`은 기본 설치 경로입니다. 앱의 인스턴스 설정에 경로를 따로 적으면 다른 곳에 두어도 됩니다.
- 명령은 다섯 가지입니다.

```bash
divixi-server                  # = serve. 서버 실행 (기본 127.0.0.1:7488)
divixi-server token            # 페어링 토큰 한 번 출력 (5분, 1회용; 앱이 SSH로 부름)
divixi-server owner <login>    # 주소로 들어올 수 있는 GitHub 계정 (--none: 없음)
divixi-server listen all|local # 모든 네트워크 / 이 서버에서만(SSH 터널) 받기 — 다시 켜야 적용
divixi-server help
```

---

## 4. 연결 방법 고르기

### A. SSH 터널 (권장, 설정할 것 없음)

서버는 `127.0.0.1`에서만 받고, 앱이 SSH로 터널을 엽니다. SSH로 들어올 수 있으면 주인으로 봅니다.

1. PC에서 **암호를 묻지 않고** `ssh user@server`가 되어야 합니다(키 + ssh-agent, 또는 `~/.ssh/config` 별칭).
   앱은 SSH를 BatchMode로 돌려서 암호나 호스트 키 확인을 물을 수 없습니다. 처음 붙는 서버라면 PC의
   터미널에서 한 번 `ssh user@server`를 해서 호스트 키를 받아 두세요.
2. Divixi 앱에서 **설정 › 원격 인스턴스 › 원격 인스턴스 추가**를 누르고 아래처럼 채운 뒤 저장합니다.
   - 연결 방식: SSH 터널
   - SSH 호스트: `user@server`
   - 원격 포트: `7488`
   - divixi-server 경로: `~/.local/bin/divixi-server`
   - 원격 PATH: 에이전트나 node를 못 찾을 때만 채웁니다. 예: `~/.local/bin:~/.nvm/versions/node/v20.20.2/bin`
3. 왼쪽 위 메뉴에서 그 인스턴스를 고릅니다. 서버가 꺼져 있으면 앱이 SSH로 켜고, 토큰을 받고, 터널을 엽니다.

### B. 직접 주소 (Windows PC처럼 SSH 서버가 없을 때, GitHub 계정)

서버가 네트워크에서 직접 받고, **서버 주인과 같은 GitHub 계정**으로 로그인한 앱만 들어옵니다.
주소는 암호화되지 않은 http라서 **Tailscale 같은 사설망 위에서만** 쓰세요.

```bash
divixi-server owner <내-GitHub-아이디>
divixi-server listen all
pkill -x divixi-server; setsid -f ~/.local/bin/divixi-server serve >/dev/null 2>&1 </dev/null
```

- 방화벽을 쓰면 7488/tcp를 사설망 쪽으로 엽니다.
- 앱에서는 설정 › 원격 인스턴스로 가서 먼저 **GitHub 계정**에 로그인합니다. OAuth 앱 Client ID가 필요하고,
  화면의 안내를 따르면 됩니다.
- 그다음 **원격 인스턴스 추가**에서 연결 방식은 **직접 주소**, 주소는 `http://<Tailscale IP>:7488`로 넣습니다.
- 데스크톱 Divixi도 설정의 "이 PC를 원격 인스턴스로 열기"로 같은 방식의 서버가 됩니다.

---

## 5. 켜 두기

**직접 켜고 끄기.** SSH 터널 방식은 앱이 알아서 켜므로 보통은 필요 없습니다.

```bash
setsid -f ~/.local/bin/divixi-server serve >/dev/null 2>&1 </dev/null   # 켜기 (SSH가 끊겨도 계속)
pkill -x divixi-server                                                  # 끄기
```

**항상 켜 두기(선택): systemd 사용자 서비스.**

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/divixi-server.service <<'EOF'
[Unit]
Description=Divixi server (remote instance)
After=network-online.target

[Service]
ExecStart=%h/.local/bin/divixi-server serve
# 에이전트 CLI와 node가 있는 곳 (nvm이면 그 bin도 넣으세요)
Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin
Restart=on-failure

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now divixi-server
sudo loginctl enable-linger "$USER"     # 로그인하지 않아도 부팅 때 켜지게
```

- 앱은 이미 떠 있는 서버를 찾으면(`pgrep`) 새로 켜지 않습니다. 그래서 systemd로 켜 두어도 두 개가 뜨지 않습니다.
- `listen`이나 `owner`를 바꾼 뒤에는 `systemctl --user restart divixi-server`로 다시 켭니다.

---

## 6. 처음 켜질 때 일어나는 일

- 데이터 폴더 `~/.local/share/app.divixi/`(`$XDG_DATA_HOME`이 있으면 그 아래)를 만듭니다.
  - `divixi.db`: Track, 대화, 설정
  - `remote.key`: 토큰 서명 키
  - `adapters/`: ACP 어댑터
- 에이전트를 스스로 감지합니다.
- Claude Code와 Codex의 ACP 어댑터를 `adapters/`에 **자동으로 받습니다**. npm이 필요하고, 받는 동안에는 npx로 대신 돌아갑니다.
- Antigravity의 ACP 서버는 앱의 에이전트 설정에서 "받기"로 받습니다.

---

## 7. 업데이트

2~3단계를 다시 하고(소스 받기, 빌드, 설치) 서버를 다시 켭니다.

```bash
pkill -x divixi-server            # systemd면: systemctl --user restart divixi-server
```

앱은 서버의 빌드가 자기와 다르면 왼쪽 위 칩에 **"!"**를 붙이고 "원격 Divixi를 업데이트하세요"라고 알립니다. 이 비교는
Rust 코드 기준이라, 화면만 바뀐 앱 업데이트에서는 뜨지 않습니다. 앱과 서버를 **같은 커밋**으로 맞추면 사라집니다 —
받는 방법은 2절에 있습니다. 앱만 업데이트했다면 서버도 그 커밋에서 다시 빌드하세요.

---

## 8. 문제 해결

| 증상 | 확인할 것 |
|---|---|
| `ssh ended … Host key verification failed` | PC 터미널에서 `ssh user@server`를 한 번 해서 호스트 키를 받아 두기 |
| `ssh ended … Permission denied` | 암호 없이 들어가지는지(키, ssh-agent) |
| `divixi-server token gave no link` | 경로가 맞는지(`~/.local/bin/divixi-server`), 실행 권한이 있는지 |
| `did not answer through the tunnel` | 서버가 뜨는지 직접 돌려 보기: `~/.local/bin/divixi-server serve` (로그가 보임) |
| 에이전트가 "설치 안 됨" | 서버 쪽 PATH. 인스턴스 설정의 "원격 PATH"나 systemd의 `Environment=PATH`에 CLI·node 경로 추가 |
| 직접 주소가 "no owner yet" | 서버에서 `divixi-server owner <GitHub 아이디>` |
| 직접 주소가 "… does not own this Divixi" | 앱의 GitHub 로그인 계정과 서버 주인이 같은지 |
| 원격 셸이 남음 | 앱이 비정상 종료되면 2분 뒤 서버가 그 기기의 셸을 닫음 |
| "빌드가 이 앱과 다릅니다" | 서버와 앱이 같은 커밋에서 나왔는지(2절). 한쪽만 업데이트한 것이면 7절 |

**로그를 보려면** 서버를 앞에서 실행합니다.

```bash
pkill -x divixi-server
RUST_LOG=info,orchestra_app=debug ~/.local/bin/divixi-server serve
```
