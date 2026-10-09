<p align="center">
  <img src="docs/images/divixi.png" alt="" width="120" height="120">
</p>

<h1 align="center">DIVIXI</h1>

<p align="center"><strong>지휘자 하나, 여러 에이전트.</strong></p>

<p align="center"><a href="README.md">English</a> | <strong>한국어</strong></p>

코딩 에이전트를 지휘하는 데스크톱 앱입니다. 당신은 지휘자 하나와 이야기하고, 지휘자가
나란히 돌아가는 작업자 세션들에게 일을 나눕니다.

DIVIXI는 당신이 이미 쓰는 에이전트들(Claude Code, Codex, GitHub Copilot, Antigravity)을
[Agent Client Protocol](https://agentclientprotocol.com) 위에서, 당신 자신의 로그인으로
구동합니다. 일의 단위 하나하나가 **Track**입니다. 폴더 하나, 오래 사는 지휘자 세션 하나,
그리고 그것이 여는 작업자들. 당신에게 닿는 것은 당신이 필요한 것뿐입니다. 내려야 할 결정,
읽어야 할 보고, 병합해야 할 변경. 전체 기록과 도구 호출은 클릭 한 번 거리에 있습니다.

이름은 *divisi*에서 왔습니다. 한 성부를 여러 독립 성부로 쪼개라는 악보 지시어입니다.
x는 여러 제공자를 가로지른다는 뜻입니다.

## 기능

- **지휘자와 작업자.** 지휘자가 계획하고 위임합니다. 작업자들은 각자의 폴더나 git
  worktree에서 병렬로 돌고, 정해진 형태로 보고하며(무엇이 바뀌었는지, 무엇을 검증했는지,
  무엇이 열려 있는지), 당신은 그들의 변경을 파일 단위로 병합합니다.
- **프롬프트가 아니라 결정.** 에이전트는 가장 자율적인 모드로 돕니다. 당신이 필요한 것은
  대화 속 결정 카드로 도착하고, 나머지는 오른쪽 위의 벨이 담아 둡니다.
- **대화 옆의 맥락.** 검색·미리보기·편집이 되는 파일 패널. 터미널. 메모·스케치·프레임·
  참고자료를 위한 디자인 보드. `@kb`로 메시지에 끌어오는, PDF·오피스 파일·메모의 지식
  라이브러리.
- **원격 인스턴스.** 리눅스 머신에서 `divixi-server`를 돌리고 창 왼쪽 위에서 그쪽으로
  전환합니다. VS Code Remote처럼요. 에이전트도, 파일도, 터미널도 거기서 돕니다.

## 설치

DIVIXI는 초기 단계 소프트웨어입니다. 주 플랫폼은 Windows이고, `divixi-server`는
Linux에서 돌며, macOS는 검증되지 않았습니다.

**필요한 것:**

- **Node.js 22.18 이상.** Claude Code와 Codex 어댑터가 Node 프로그램입니다.
  DIVIXI가 첫 사용 시 자기 데이터 폴더에 설치합니다.
- **에이전트 최소 하나, 설치되고 로그인된 상태:**

| 에이전트 | 설치 | 로그인 |
|---|---|---|
| Claude Code | [claude.com/claude-code](https://claude.com/claude-code) | `claude`를 한 번 실행 |
| Codex | [openai.com/codex](https://openai.com/codex) | `codex login` |
| GitHub Copilot | [github.com/github/copilot-cli](https://github.com/github/copilot-cli) | `copilot login` |
| Antigravity | [antigravity.google/cli](https://antigravity.google/cli) | `agy login` (DIVIXI가 그 ACP 서버를 받아 옵니다) |

첫 실행 때 DIVIXI는 당신이 가진 에이전트를 찾아내고, 각각이 세션을 열 수 있는지
확인합니다.

**소스에서 빌드** (Rust stable, Node.js 22.18+):

```bash
git clone https://github.com/jaemin93/divixi
cd divixi
npm install
npm run app                                     # 창이 뜨는 개발 빌드
npx tauri build                                 # 설치 파일이 target/release/bundle 에
```

## 원격 인스턴스

`divixi-server`는 창이 없는 DIVIXI로, 서버용입니다. 데스크톱 앱은 SSH 터널을 통해, 또는
당신의 GitHub 계정으로 특정 주소에서 여기에 닿습니다. 서버가 떠 있지 않으면 앱이 SSH로
서버를 시작시킵니다. 휴대폰에서 Tailscale로 서버를 열려면 서버에서
`divixi-server phone on`으로 켭니다.

빌드·설치·연결·상시 구동·업데이트는 **[docs/divixi-server.ko.md](docs/divixi-server.ko.md)**
를 보세요.

## 데이터와 프라이버시

- **텔레메트리 없음.** DIVIXI는 당신이나 당신의 사용에 관한 어떤 것도 어디로도 보내지
  않습니다. 보고도, 집계도, 몰래 알리는 것도 없습니다. DIVIXI가 서버에 보내는 요청은
  아래의 업데이트 확인 하나뿐입니다.
- **업데이트 확인.** DIVIXI는 두 경우에 이 저장소의 가장 최근 릴리스를 GitHub 공개
  API에 물어봅니다. **설정 → 정보**(Settings → About)에서 *업데이트 확인*(Check
  for updates)을 누를 때, 그리고 앱을 켤 때마다 한 번입니다. 요청에는 로그인
  정보도, 버전도, 플랫폼도, 당신이나 이 컴퓨터를 알아볼 수 있는 어떤 것도 담기지
  않습니다. GitHub이 보는 것은 공개된 페이지를 읽는 주소 하나입니다. 시작할 때의
  확인은 그 버튼 옆의 스위치로 끌 수 있고, 꺼도 버튼은 그대로 동작합니다. 새 릴리스가
  있을 때만 알려 주며, 최신이거나 확인에 실패했으면 아무것도 띄우지 않고 실패는 그
  카드에만 남습니다.
- **업데이트는 당신의 몫.** 내려받기와 설치는 누를 때만 일어납니다. 새 릴리스가
  있으면 DIVIXI가 설치 파일을 받아(지금은 Windows) GitHub이 공개한 그 파일의
  SHA-256과 대조한 다음, 열어 주는 버튼을 내줍니다. 설치 마법사는 당신이 직접
  진행합니다. DIVIXI가 저절로 설치하는 일은 없습니다. 코드 서명 인증서가 없으므로,
  받아 온 서명 없는 코드를 당신이 요청하지 않았는데 실행하지는 않습니다.
- **데이터가 있는 곳.** Track, 대화, 보드, 지식 라이브러리는 앱의 데이터 폴더에 있습니다.
  Windows는 `%APPDATA%\app.divixi`, Linux는 `~/.local/share/app.divixi`.
- **제공자에게 닿는 것.** 당신의 프롬프트는 당신이 돌리는 에이전트로 갑니다. 에이전트는
  그것을 당신 자신의 로그인으로 자기 제공자에게 보냅니다. 터미널에서 돌릴 때와 똑같이.
- **로그는 당신 기계에 남습니다.** 경고와 오류는 같은 데이터 폴더의 `logs/`로 가고, 하루에
  한 파일씩, 최근 일곱 개를 보관합니다. 패닉은 `logs/crash.log`로 갑니다. 키나 토큰처럼
  보이는 값은 무엇이든 기록되기 전에 가려집니다. 아무것도 어디로도 전송되지 않습니다.
  버그 리포트는 당신이 붙여넣는 당신의 것입니다.
- **원격 인스턴스.**
  - 서버는 기본적으로 `127.0.0.1`에서 듣고, 그 주인만 들어올 수 있습니다.
  - 모든 네트워크에서 듣게 하면 평문 HTTP입니다. Tailscale이나 신뢰하는 네트워크 위에서
    쓰세요.
  - 당신의 GitHub 토큰은 오직 당신 자신의 서버에게 당신임을 증명하는 데만 쓰입니다.

## 기여

**[CONTRIBUTING.md](CONTRIBUTING.md)** 부터 보세요. 버그를 신고하는 법, 개발 환경을
갖추는 법, 돌려야 할 검사, 풀 리퀘스트를 열기 전에 알아야 할 것이 있습니다.
[docs/development.md](docs/development.md) 는 개발자 안내서입니다. 코드가 어떻게
배치되어 있는지, 어떤 원칙으로 쓰였는지, 알아 둘 만한 함정은 무엇인지 다룹니다.

참여하는 모든 사람은 [행동 강령](CODE_OF_CONDUCT.md)의 적용을 받습니다. 보안 취약점은
공개 이슈로 열지 마세요. [SECURITY.md](SECURITY.md) 에 비공개 경로가 있습니다.

### 버그 신고

**설정 → 정보 → 진단**(Settings → About → Diagnostics)을 열고 *진단 정보 복사*(Copy
diagnostics)를 누르세요. 버전, 당신의 OS, DIVIXI가 찾아낸 에이전트들, 그리고 마지막
경고와 오류가 하나의 마크다운 블록으로 클립보드에 담깁니다. 붙여넣기 전에 읽을 수
있도록 화면에 먼저 보여 줍니다. 그것을
[버그 리포트](https://github.com/jaemin93/divixi/issues/new?template=bug_report.yml)
에 붙여 주세요.

나머지는 [CONTRIBUTING.md](CONTRIBUTING.md#reporting-a-bug) 에 있습니다. 로그가 어디에
있는지, 문제를 재현하기 전에 기록 수준을 어떻게 올리는지 다룹니다.

## 감사의 말

### Kiro Crew

DIVIXI 인터페이스의 상당 부분은 [Kiro Crew](https://github.com/kirodotdev/KiroCrew)를
연구하며 설계했습니다. AWS가 만든 오픈소스 에이전트 워크스페이스입니다 (Apache License
2.0, Copyright Amazon.com, Inc. or its affiliates).

거기서 가져온 것:

- **레이아웃과 상호작용.** 왼쪽 레일과 세션 패널, 오른쪽 작업 폴더 패널, 에이전트 선택기,
  제목 표시줄의 인스턴스 스위처, 알림 벨, 세션 필터와 컨텍스트 메뉴, 터미널 패널,
  되풀이 목록과 그 실행 이력. 전부 Kiro Crew의 것을 본떴습니다. 이들은 아이디어와 배치이며, Svelte와 Rust로 처음부터 다시
  구현했습니다. 코드도, 스타일시트도, 아이콘도, 폰트도, 이미지도 복사하지 않았습니다.
- **지식 라이브러리.** `crates/knowledge/`의 청킹 전략, 검색 결과 융합, LLM 추출
  프롬프트는 Kiro Crew의 `knowledge` 모듈을 따릅니다. 추출 프롬프트는 Apache License
  2.0 아래 Kiro Crew의 `extractor.py`에서 가져와 우리 규칙 두 개를 더한 것입니다.
  [NOTICE](NOTICE)를 보세요. 우리 구현은 새로 쓴 Rust 코드이고 로컬 임베딩 없이
  동작합니다.
- **원격 인스턴스.** 다른 기계에서 창 없는 서버를 돌리고 앱에서 그쪽으로 전환한다는
  발상은 Kiro Crew의 원격 인스턴스에서 왔고, 감시되는 SSH 터널 옵션들도 마찬가지입니다.
  우리 구현은 다릅니다 (iframe이 아니라 네이티브 webview).

DIVIXI가 제 길을 가는 지점: 작업자 에이전트들을 지휘하는 지휘자 에이전트 하나,
프롬프트마다 승인받는 대신 쓰는 결정 카드, 그리고 로컬 임베딩 모델이 필요 없는 지식
라이브러리.

**DIVIXI는 독립적인 프로젝트입니다. Kiro, AWS, Amazon과 제휴 관계가 없으며, 이들의
보증이나 후원을 받지 않습니다. "Kiro"와 "Kiro Crew"는 Amazon.com, Inc. 또는 그 계열사의
상표이며, 여기서는 오직 DIVIXI가 빌려 온 아이디어의 출처를 밝히기 위해서만
사용되었습니다.**

## 라이선스

DIVIXI는 [Apache License, Version 2.0](LICENSE) 아래 배포됩니다. 그 라이선스가 요구하는
귀속 표기는 [NOTICE](NOTICE)를 보세요.
