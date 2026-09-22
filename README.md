# Orchestra

에이전트 오케스트레이션 데스크톱 앱. Rust + Tauri 2 + Svelte 5.

## 설계 원칙

**막(membrane).** Track 타임라인에는 Report / Decision / Running / 대화 네 가지만 올라옵니다.
트랜스크립트, diff, 툴 호출은 레인에 남고 인스펙터로만 내려갑니다.
`LaneEvent::above_membrane()`이 그 경계를 코드로 강제합니다.

**메커니즘과 정책의 분리.** Rust 코어는 레인을 띄우고 이벤트를 나르기만 합니다.
무엇을 어떻게 분해할지(지휘자 로직)는 에이전트가 정합니다 — 코어에 넣지 않습니다.

**프로토콜은 한 곳에만.** `crates/acp`만 ACP를 압니다. 그 위는 `LaneEvent`만 봅니다.
ACP를 지원하지 않는 에이전트는 나중에 같은 채널 뒤에 PTY 백엔드로 붙습니다.

## 구조

```
crates/orchestra/   도메인 — LaneEvent, 막의 정의
crates/acp/         ACP 클라이언트 — 에이전트 기동, 세션, 스트리밍
src-tauri/          앱 셸 — run 식별자, 이벤트 코얼레싱, IPC
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

**어댑터.** Claude Code CLI 2.1.x는 ACP 네이티브 지원이 없어서
`@zed-industries/claude-code-acp`를 `npx`로 띄웁니다. 첫 실행은 다운로드 때문에
느립니다. 레인은 `bypassPermissions` 모드로 돌아갑니다 — 툴 승인 프롬프트가 아니라
레인이 스스로 올리는 에스컬레이션만 사람에게 보이는 게 설계 의도입니다.

**코얼레싱.** 에이전트 텍스트 청크는 40ms 단위로 묶어서 webview에 보냅니다.
토큰마다 IPC를 태우면 창이 버벅입니다.

## 현재 상태 (Phase 0)

되는 것: 레인 1개 기동, 프롬프트 1회 실행, 스트리밍, Report 카드, 인스펙터
(트랜스크립트 / 출력 / 툴), 다크·라이트 테마.

아직 없는 것: 다중 레인, 워크트리 격리, 이벤트 스토어(SQLite), Decision 에스컬레이션,
Draft 화면, 승격, wrap-up 폴드.
