# Orchestra

에이전트 오케스트레이션 데스크톱 앱. Rust + Tauri 2 + Svelte 5.

## 설계 원칙

**막(membrane).** Track 타임라인에는 Report / Decision / Running / 대화 네 가지만 올라옵니다.
트랜스크립트, diff, 툴 호출은 레인에 남고 인스펙터로만 내려갑니다.
`LaneEvent::above_membrane()`이 그 경계를 코드로 강제합니다.

**메커니즘과 정책의 분리.** Rust 코어는 레인을 띄우고 이벤트를 나르기만 합니다.
무엇을 어떻게 분해할지(지휘자 로직)는 에이전트가 정합니다 — 코어에 넣지 않습니다.
지휘자는 Track마다 하나 있는 오래 사는 ACP 세션이고, 앱 API(`spawn_lane`, `ask_lane`,
`read_report`, `record_decision` …)를 프로세스 안 HTTP MCP 서버로 받습니다. 레인도 각각
오래 사는 세션이라 Claude Code 세션 하나를 열어 두고 계속 시키는 것과 같습니다.

**프로토콜은 한 곳에만.** `crates/acp`만 ACP를 압니다. 그 위는 `LaneEvent`만 봅니다.
ACP를 지원하지 않는 에이전트는 나중에 같은 채널 뒤에 PTY 백엔드로 붙습니다.

## 구조

```
crates/orchestra/   도메인 — LaneEvent, 막의 정의
crates/acp/         ACP 클라이언트 — 에이전트 기동, 세션, 스트리밍
crates/agents/      에이전트 카탈로그 — CLI 탐색, ACP 프로브·로그인, Antigravity 서버 다운로드
crates/mcp/         앱 내장 HTTP MCP 서버 — 지휘자에게 주는 도구
crates/store/       이벤트 스토어 — SQLite, append-only 로그 + runs 투영 + FTS5
src-tauri/          앱 셸 — run 식별자, 이벤트 영속화, 코얼레싱, IPC
ui/                 Svelte 5 — 타임라인, 인스펙터, 두 테마
refs/               디자인 레퍼런스
```

## 실행

```bash
npm install
npm run app          # tauri dev — 창이 뜹니다
```

ACP 연결만 따로 확인하려면:

```bash
cargo run -p orchestra-acp --example smoke -- "Reply with exactly: ORCHESTRA OK"
```

## 알아둘 것

**Claude Code 세션 안에서 실행하면 레인이 죽습니다.** `CLAUDECODE`,
`CLAUDE_CODE_*` 환경변수가 자식 프로세스로 상속되면 Claude Code가 중첩 세션으로
판단하고 즉시 종료합니다. 증상은 `session/new`에서 나는
`Query closed before response received` — 원인을 전혀 알려주지 않는 에러입니다.
`scrub_inherited_session_env()`가 `main` 맨 앞에서 이 변수들을 지웁니다.
스레드가 생기기 전에 호출해야 합니다.

**에이전트.** 네 가지를 ACP로 붙입니다. 첫 실행 setup과 설정 화면의 "다시 감지"가
`crates/agents`의 감지를 돌립니다.

| 에이전트 | ACP 진입 | 자율 모드 id |
|---|---|---|
| Claude Code | `@agentclientprotocol/claude-agent-acp` (devDependency, `node`로 직접) | `bypassPermissions` |
| Codex | `@agentclientprotocol/codex-acp` (devDependency, `@openai/codex` 번들) | `agent-full-access` |
| GitHub Copilot | 설치된 `copilot.exe --acp` | `…/session-modes#autopilot` |
| Antigravity | Google의 `agy_acp_server` zip을 앱 데이터 폴더에 다운로드 | `yolo` |

감지는 세 단계입니다. CLI 실행 파일 탐색(프로세스 PATH + Windows 레지스트리 PATH +
알려진 설치 경로), ACP `initialize`(이름·버전·로그인 방법), `session/new`(성공이면
준비됨, `auth_required`면 로그인 필요). 로그인은 ACP `authenticate`로 에이전트가 직접
합니다. Antigravity는 `agy` CLI 로그인과 별개로 ACP 서버 인증이 필요하며
`oauth-personal`이 Google 계정 자격을 재사용합니다.

레인은 에이전트가 광고하는 모드 중 가장 자율적인 것으로 돌아갑니다 — 툴 승인
프롬프트가 아니라 레인이 스스로 올리는 에스컬레이션만 사람에게 보이는 게 설계
의도입니다. `ORCHESTRA_ACP_ADAPTER`로 Claude 어댑터 경로를 강제할 수 있습니다.

에이전트 프로세스는 `crates/acp`가 직접 띄우고 직접 죽입니다. Windows에서는 Job
Object로 묶어 자손까지 함께 종료됩니다. Antigravity 서버는 stdin EOF를 무시하므로
프로토콜 크레이트의 기본 정리에 맡기면 레인마다 하나씩 남습니다.

```bash
cargo run -p orchestra-agents --example detect                  # 감지 결과 출력
cargo run -p orchestra-agents --example detect -- --download    # Antigravity 서버까지
cargo run -p orchestra-agents --example lane -- codex "prompt"  # 특정 에이전트로 레인 1회
```

**코얼레싱.** 에이전트 텍스트 청크는 40ms 단위로 묶어서 webview에 보냅니다.
토큰마다 IPC를 태우면 창이 버벅입니다. 스토어에도 같은 프레임 단위로 기록됩니다.

**이벤트 스토어.** 레인 이벤트는 전부 `events` 테이블에 append-only로 쌓이고,
`runs` 테이블은 그 로그를 런 종료 시점에 접은(fold) 투영입니다. 타임라인은 `runs`만
읽어서 복원하고, 인스펙터를 열 때만 그 런의 `events`를 재생합니다 — 막이 저장소
레벨에서도 지켜집니다. 종료된 런은 프롬프트·출력·툴 제목이 FTS5로 색인됩니다.
앱이 실행 중이던 런을 남기고 죽으면 다음 기동 때 `failed`로 닫습니다.
파일은 앱 데이터 폴더의 `orchestra.db`이고 `ORCHESTRA_DB`로 바꿀 수 있습니다
(`:memory:`도 됩니다). 저장 경로만 따로 확인하려면:

```bash
cargo run -p orchestra-store --example persist -- "Reply with exactly: ORCHESTRA OK"
```

## 현재 상태 (Phase 1 진행 중)

되는 것: 첫 실행 setup의 에이전트 감지·로그인·다운로드, 설정의 재감지, 런마다
에이전트 선택(Claude Code / Codex / Copilot / Antigravity), 레인 1개 기동, 프롬프트 1회
실행, 스트리밍, Report 카드, 인스펙터(트랜스크립트 / 출력 / 툴), 다크·라이트 테마,
SQLite 이벤트 스토어(재시작 후 타임라인 복원, 인스펙터 지연 로드, 전문 검색 API).

아직 없는 것: Track 생성 흐름(첫 화면), 지휘자 세션, 다중 레인, 워크트리 격리,
Report 스키마 강제, Decision 에스컬레이션, Draft 화면, 승격, wrap-up 폴드, 검색 UI,
대화창 폰트 크기 설정.
