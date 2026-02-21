# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** [English](README.md) | 한국어

**QUIC 기반 OS 게이트웨이 AI 에이전트** — LLM이 메신저를 통해 여러분의 머신을 제어할 수 있게 합니다.

> WebSocket 기반 에이전트 아키텍처인 [OpenClaw](https://github.com/openpista/openclaw)에서 영감을 받아,
> 더 낮은 레이턴시, HOL(Head-of-Line) 블로킹 제거, 런타임 의존성 없는 단일 정적 바이너리를 위해
> Rust와 QUIC 전송 프로토콜로 완전히 새롭게 작성되었습니다.

---

## openpista이란?

openpistacrab은 Rust로 작성된 경량 데몬으로, **메시징 채널**(텔레그램, CLI, WhatsApp)과 **운영체제**를 AI 에이전트 루프로 연결합니다.

- 텔레그램에서 메시지를 보내면 → LLM이 무엇을 할지 결정 → bash가 실행 → 결과가 돌아옴
- 단일 정적 바이너리, ~10 MB, 최소 메모리 사용
- 낮은 레이턴시를 위한 QUIC 전송 (0-RTT), WebSocket 대신 사용
- SQLite 기반의 지속적 대화 메모리
- 확장 가능한 **Skills** 시스템: 워크스페이스에 `SKILL.md`를 넣어 새로운 에이전트 기능 추가

```
[ 채널 어댑터 ]        텔레그램 · CLI
        │  tokio::mpsc
        ▼
[ QUIC OS 게이트웨이 ]  quinn · rustls · session · router · cron
        │  QUIC 스트림
        ▼
[ 에이전트 런타임 ]     LLM 루프 · ToolRegistry · SQLite 메모리
        │  tool_call
        ▼
[ OS 도구 ]            system.run (bash) · screen* · input control*
[ Skills ]             SKILL.md → 시스템 프롬프트 + 서브프로세스

* v0.2.0에서 지원 예정
```

---

## 기능

| 기능 | 상태 |
|---|---|
| Bash 도구 (`system.run`) | ✅ v0.1.0 |
| 텔레그램 채널 | ✅ v0.1.0 |
| 대화형 CLI / REPL | ✅ v0.1.0 |
| QUIC 게이트웨이 (자체 서명 TLS) | ✅ v0.1.0 |
| 크론 스케줄러 | ✅ v0.1.0 |
| SQLite 대화 메모리 | ✅ v0.1.0 |
| Skills (SKILL.md 로더) | ✅ v0.1.0 |
| 화면 캡처 | 🔜 v0.2.0 |
| 화면 & 입력 제어 (OpenClaw 방식) | 🔜 v0.2.0 |
| Discord / Slack 어댑터 | 🔜 v0.2.0 |

---

## 설치

### 사전 요구사항

- **Rust 1.85+** — [rustup.rs](https://rustup.rs)
- **SQLite 3** — macOS/Linux에 보통 기본 설치되어 있음

### macOS

```bash
# Rust 툴체인 설치
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# (선택사항) Homebrew로 SQLite 설치
brew install sqlite

# 클론 및 빌드
git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release

# 바이너리를 PATH에 복사
sudo cp target/release/openpista /usr/local/bin/
```

### Ubuntu / Debian

```bash
# Rust 툴체인 설치
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 시스템 의존성
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev libsqlite3-dev

# 클론 및 빌드
git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release

sudo cp target/release/openpista /usr/local/bin/
```

### Fedora / RHEL

```bash
sudo dnf install -y gcc pkg-config openssl-devel sqlite-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release
sudo cp target/release/openpista /usr/local/bin/
```

---

## 설정

예제 설정 파일을 복사하여 수정하세요:

```bash
cp config.example.toml config.toml
```

```toml
[gateway]
port = 4433          # QUIC 수신 포트
tls_cert = ""        # 비워두면 자체 서명 인증서 자동 생성

[agent]
provider = "openai"
model = "gpt-4o"
api_key = ""         # 또는 OPENPISTACRAB_API_KEY 환경변수 사용
max_tool_rounds = 10

[channels.telegram]
enabled = false
token = ""           # 또는 TELEGRAM_BOT_TOKEN 환경변수 사용

[channels.cli]
enabled = true

[database]
url = "~/.openpista/memory.db"

[skills]
workspace = "~/.openpista/workspace"
```

### 환경 변수

| 변수 | 설명 |
|---|---|
| `OPENPISTACRAB_API_KEY` | OpenAI 호환 API 키 (설정 파일 덮어씀) |
| `OPENAI_API_KEY` | 대체 API 키 |
| `OPENCODE_API_KEY` | OpenCode Zen API 키 |
| `TELEGRAM_BOT_TOKEN` | 텔레그램 봇 토큰 (텔레그램 채널 활성화) |
| `OPENPISTACRAB_WORKSPACE` | 커스텀 Skills 워크스페이스 경로 |

---

## 사용법

### 단일 명령 실행

```bash
OPENPISTACRAB_API_KEY=sk-... openpista run -e "홈 디렉토리의 파일을 나열해줘"
```

### 인증 로그인 Picker

```bash
# 검색 + 화살표 선택 기반 인터랙티브 로그인
openpista auth login

# 스크립트/CI용 비대화형 모드
openpista auth login --non-interactive --provider opencode --api-key "$OPENCODE_API_KEY"
```

TUI 명령:

```txt
/login
/connection
```

### 모델 카탈로그 (OpenCode)

```bash
# 코딩 추천 모델 목록
openpista models list
```

TUI 명령:

```txt
/models
```

`/models` 브라우저 내부 키:

```txt
s 또는 /: model id 검색
r: 원격 카탈로그 강제 동기화
Esc: (검색 모드) 검색 종료, (일반 모드) 브라우저 종료
```

### 데몬 모드 (텔레그램 + CLI + QUIC 게이트웨이)

```bash
OPENPISTACRAB_API_KEY=sk-... \
TELEGRAM_BOT_TOKEN=123456:ABC... \
openpista start
```

데몬은:
- 원격 에이전트 연결을 위해 QUIC 포트 `4433`에서 수신 대기
- 활성화된 모든 채널 어댑터 시작
- `~/.openpista/openpista.pid`에 PID 파일 저장
- 정상 종료를 위한 `SIGTERM` / `Ctrl-C` 처리

### Skills

워크스페이스에 `SKILL.md`를 배치하여 에이전트 기능을 확장하세요:

```
~/.openpista/workspace/skills/
├── deploy/
│   ├── SKILL.md      ← 이 skill이 무엇을 하는지 설명
│   └── run.sh        ← 에이전트가 이 skill을 호출할 때 실행됨
└── monitor/
    ├── SKILL.md
    └── main.py
```

---

## 워크스페이스 구조

```
openpista/
├── crates/
│   ├── proto/      # 공유 타입, 에러 (AgentMessage, ToolCall, …)
│   ├── gateway/    # QUIC 서버, 세션 라우터, 크론 스케줄러
│   ├── agent/      # ReAct 루프, LLM 프로바이더, SQLite 메모리
│   ├── tools/      # Tool 트레이트 + BashTool (system.run)
│   ├── channels/   # ChannelAdapter, CliAdapter, TelegramAdapter
│   ├── skills/     # SKILL.md 로더, 서브프로세스 실행기
│   └── cli/        # 바이너리 진입점, clap, config, daemon
├── migrations/     # SQLite 스키마 마이그레이션
├── config.example.toml
└── README.md
```

---

## 기여하기

기여는 언제나 환영합니다! 다음 절차를 따라주세요:

1. **Fork** 후 피처 브랜치 생성:
   ```bash
   git checkout -b feat/my-feature
   ```

2. **코드 스타일** — 커밋 전에 포맷 및 린트 실행:
   ```bash
   cargo fmt --all
   cargo clippy --workspace -- -D warnings
   ```

3. **테스트** — 기존 테스트가 모두 통과해야 하며, 새 코드에는 테스트 포함:
   ```bash
   cargo test --workspace
   ```

4. **커밋 메시지 규칙**:
   ```
   feat(tools): add screen capture tool
   fix(agent): handle empty LLM response gracefully
   docs: update installation guide for Windows
   ```
   [Conventional Commits](https://www.conventionalcommits.org/) 스타일을 따릅니다.

5. `main` 브랜치를 대상으로 **Pull Request**를 엽니다. PR 설명에는 다음을 작성하세요:
   - 해결하는 문제
   - 변경 사항 테스트 방법

6. 중요한 변경인 경우, **먼저 이슈를 열어** 코드 작성 전에 접근 방식을 논의하세요.

### 개발 환경 설정

```bash
git clone https://github.com/openpista/openpista.git
cd openpista

# 테스트 실행
cargo test --workspace

# 문제 확인
cargo clippy --workspace -- -D warnings

# 릴리즈 바이너리 빌드
cargo build --release
```

## 에이전트 오케스트레이션

멀티 에이전트 역할 분리 및 모델 라우팅에 대한 운영 가이드는 다음에서 확인하세요:

- `docs/agent-orchestration/README.md`
- `docs/agent-orchestration/routing-rules.md`
- `docs/agent-orchestration/policies.md`

---

## 라이선스

다음 라이선스 중 하나를 선택하여 사용할 수 있습니다:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)
