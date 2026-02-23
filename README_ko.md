# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** [English](README.md) | 한국어

Docs: [ROADMAP](./ROADMAP.md) · [CHANGELOG (v0.1.0+)](./CHANGELOG.md) · [WhatsApp 설정 가이드](./WHATSAPP.md)

**Rust→WASM 브라우저 접속을 지원하는 OS 게이트웨이 AI 에이전트.** LLM이 메신저를 통해 여러분의 머신을 제어할 수 있게 합니다.
> WebSocket 기반 에이전트 아키텍처인 [OpenClaw](https://github.com/openclaw/openclaw)에서 영감을 받아,
> Rust와 Rust→WASM 브라우저 클라이언트로 완전히 새롭게 작성되었습니다 —
> 런타임 의존성 없는 단일 정적 바이너리.

---

## openpista란?

openpista는 Rust로 작성된 경량 데몬으로, **메시징 채널**(텔레그램, 왓츠앱, CLI, 웹 브라우저)과 **운영체제**를 AI 에이전트 루프로 연결합니다.

- 텔레그램에서 메시지를 보내면: LLM이 무엇을 할지 결정하고, bash가 실행하며, 결과가 돌아옵니다
 단일 정적 바이너리, ~10 MB, 최소 메모리 사용
 SQLite 기반의 지속적 대화 메모리
- Chromium CDP를 통한 완전한 브라우저 자동화와 데스크톱 화면 캡처
- 확장 가능한 **Skills** 시스템: 워크스페이스에 `SKILL.md`를 넣어 새로운 에이전트 기능 추가

```
[ 채널 어댑터 ]        텔레그램 · 왓츠앱 · CLI (TUI) · 웹 (WASM)
        │  tokio::mpsc  ChannelEvent
        ▼
[ OS 게이트웨이 ]      프로세스 내 라우터 · 크론 스케줄러
        │
[ 에이전트 런타임 ]     ReAct 루프 · OpenAI / Anthropic / Responses API · SQLite 메모리
        │  tool_call
        ▼
[ OS 도구 ]            system.run · browser.* · screen.capture · container.run
[ Skills ]             SKILL.md → 시스템 프롬프트 + 서브프로세스 / WASM
```

---

## 기능

| 기능 | 상태 |
|---|---|
| Bash 도구 (`system.run`) | ✅ v0.1.0 |
| 브라우저 도구 (`browser.*`) | ✅ v0.1.0 |
| 화면 캡처 (`screen.capture`) | ✅ v0.1.0 |
| Docker 샌드박스 (`container.run`) | ✅ v0.1.0 |
| WASM 스킬 샌드박스 | ✅ v0.1.0 |
| 텔레그램 채널 | ✅ v0.1.0 |
| 크론 스케줄러 | ✅ v0.1.0 |
| SQLite 대화 메모리 | ✅ v0.1.0 |
| 세션 관리 (사이드바 + 브라우저) | ✅ v0.1.0 |
| Skills (SKILL.md 로더) | ✅ v0.1.0 |
| 멀티 프로바이더 OAuth (PKCE) | ✅ v0.1.0 |
| 모델 카탈로그 브라우저 | ✅ v0.1.0 |
| OpenAI Responses API (SSE) | ✅ v0.1.0 |
| Anthropic Claude 프로바이더 | ✅ v0.1.0 |
| 웹 어댑터 (Rust→WASM + WebSocket) | ✅ v0.1.0 |
| 왓츠앱 채널 (WhatsApp Web / QR 페어링) | ✅ v0.1.0 |
| Discord / Slack 어댑터 | 🔜 v0.2.0 |

---

## 프로바이더

기본 제공 프로바이더 프리셋 6가지:

| 프로바이더 | 기본 모델 | 인증 방식 |
|---|---|---|
| `openai` (기본값) | gpt-4o | OAuth PKCE, API 키 |
| `claude` / `anthropic` | claude-sonnet-4-6 | OAuth PKCE, Bearer |
| `together` | meta-llama/Llama-3.3-70B-Instruct-Turbo | API 키 |
| `ollama` | llama3.2 | 없음 (로컬) |
| `openrouter` | openai/gpt-4o | OAuth PKCE, API 키 |
| `custom` | 직접 설정 | 직접 설정 |

OpenAI 프리셋은 표준 ChatCompletions API와 ChatGPT Pro 구독자용 Responses API(`/v1/responses`) 모두를 지원합니다. Anthropic 프리셋은 `anthropic-beta: oauth-2025-04-20` 헤더를 사용한 OAuth Bearer 인증을 사용하며, 도구 이름 정규화를 자동으로 처리합니다.

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

## 빠른 시작 (Quick Start)

openpista를 빌드한 후, LLM 프로바이더에 인증하고 TUI를 실행하세요:

```bash
# 1. 로그인 (브라우저 OAuth PKCE — 권장)
openpista auth login
# 2. TUI 실행
openpista
```

이것으로 끝입니다. OAuth 토큰은 `~/.openpista/credentials.json`에 저장되며, 만료 시 자동으로 갱신됩니다.

---

## 인증 (Authentication)

**OAuth PKCE 브라우저 로그인**이 권장되는 인증 방법입니다. OpenAI, Anthropic, OpenRouter에서 바로 사용 가능하며 — API 키가 필요 없습니다.

```bash
# 인터랙티브 프로바이더 선택창 (검색 + 화살표 선택)
openpista auth login
```

TUI 명령:

```txt
/login
/connection
```

OAuth를 지원하지 않는 프로바이더(Together, Ollama, Custom)는 API 키를 제공하세요:

```bash
# API 키 로그인 (자격증명 저장소에 저장)
openpista auth login --non-interactive --provider together --api-key "$TOGETHER_API_KEY"
# 커스텀 엔드포인트를 사용하는 프로바이더
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"
```

### 자격증명 해석 우선순위 (Credential Resolution Priority)

openpista는 다음 순서로 자격증명을 해석합니다 (높은 우선순위 순):

| 우선순위 | 출처 | 설명 |
|---|---|---|
| 1 | 설정 파일 / `openpista_API_KEY` | `config.toml`의 `api_key` 또는 환경 변수 오버라이드 |
| 2 | 자격증명 저장소 | `openpista auth login`으로 저장된 토큰 (`~/.openpista/credentials.json`) |
| 3 | 프로바이더 환경 변수 | 예: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `TOGETHER_API_KEY` |
| 4 | 레거시 폴백 | `OPENAI_API_KEY` (다른 출처가 없을 때 사용) |

대부분의 사용자는 **`openpista auth login` (우선순위 2)만으로 충분합니다.** 환경 변수와 설정 파일 키는 CI 파이프라인, Docker 컨테이너, 비대화형 스크립트용으로 제공됩니다.

---

## 설정 (Configuration)

설정 파일 로딩 순서: `--config` 경로 → `./config.toml` → `~/.openpista/config.toml` → 기본값.

```bash
cp config.example.toml config.toml
```
```toml
[agent]
provider = "openai"
model = "gpt-4o"
max_tool_rounds = 10
# api_key = ""       # 선택적 — `openpista auth login` 사용을 권장
[channels.telegram]
enabled = false
token = ""
[channels.cli]
enabled = true
url = "~/.openpista/memory.db"
workspace = "~/.openpista/workspace"

[channels.whatsapp]
enabled = false
phone_number = ""
access_token = ""
webhook_port = 8443

[channels.web]
enabled = false
token = ""
port = 3210
static_dir = "~/.openpista/web"
```

### 환경 변수 오버라이드 (CI / 스크립트용)

환경 변수는 설정 파일 값을 덮어씁니다. CI 파이프라인, Docker, 비대화형 환경용이며 — 기본 설정 방법이 아닙니다.
| 변수 | 설명 |
|---|---|
| `openpista_API_KEY` | API 키 오버라이드 (최상위 우선순위) |
| `OPENAI_API_KEY` | OpenAI API 키 |
| `ANTHROPIC_API_KEY` | Anthropic API 키 |
| `openpista_MODEL` | 모델 재정의 |
| `openpista_OAUTH_CLIENT_ID` | 커스텀 OAuth PKCE 클라이언트 ID |
| `openpista_WEB_TOKEN` | 웹 어댑터 인증 토큰 |
| `openpista_WEB_PORT` | 웹 어댑터 HTTP/WS 포트 (기본값: 3210) |
| `openpista_WORKSPACE` | 커스텀 Skills 워크스페이스 경로 |
| `WHATSAPP_ACCESS_TOKEN` | 왓츠앱 액세스 토큰 |
| `WHATSAPP_PHONE_NUMBER` | 왓츠앱 전화번호 |
| `TELEGRAM_BOT_TOKEN` | 텔레그램 봇 토큰 (자동 활성화) |
| `OPENCODE_API_KEY` | OpenCode Zen API 키 |
---

## 사용법 (Usage)

### TUI (기본값)

```bash
# TUI 실행
openpista
openpista -s SESSION_ID
openpista tui -s SESSION_ID
```

### 단일 명령 실행

```bash
openpista run -e "홈 디렉토리의 파일을 나열해줘"
```

### 모델 카탈로그

```bash
openpista model list
```

TUI 명령:

```txt
/model
/model list
```

모델 브라우저 내부 키:

```txt
s 또는 /: 모델 ID 검색
r:        원격 카탈로그 강제 동기화
Esc:      (검색 모드) 검색 종료, (일반 모드) 브라우저 종료
```

### 세션 관리

TUI 명령:

```txt
/session              - 세션 브라우저 열기
/session new          - 새 세션 시작
/session load ID      - ID로 세션 로드
/session delete ID    - ID로 세션 삭제
```

`Tab`을 눌러 최근 세션 목록을 보여주는 사이드바를 토글합니다. `j`/`k` 또는 화살표 키로 이동하고, `Enter`로 열고, `d`/`Delete`로 삭제를 요청하고, `Esc`로 포커스를 해제합니다.

### 데몬 모드 (텔레그램 + 왓츠앱 + CLI + 웹 UI)

```bash
openpista start
```

텔레그램을 `config.toml` 또는 환경 변수로 활성화하세요:

```bash
# config.toml 방식 (권장)
# [channels.telegram]
# enabled = true
# token = "123456:ABC..."

# 또는 CI/Docker용 환경 변수
TELEGRAM_BOT_TOKEN=123456:ABC... openpista start
```

왓츠앱을 `config.toml`에서 활성화하세요 (자세한 설정 방법은 [WHATSAPP.md](./WHATSAPP.md) 참조):
```bash
# [channels.whatsapp]
# enabled = true
# phone_number = "15551234567"
# access_token = "EAA..."
WHATSAPP_ACCESS_TOKEN=EAA... WHATSAPP_PHONE_NUMBER=15551234567 openpista start
```

웹 UI 어댑터를 활성화하세요:

```bash
# [channels.web]
# enabled = true
# token = "my-secret-token"
# port = 3210

# 또는 환경 변수로
openpista_WEB_TOKEN=my-secret-token openpista_WEB_PORT=3210 openpista start
# 그러면 브라우저에서 http://localhost:3210 으로 접속하세요
```

데몬은:
 활성화된 모든 채널 어댑터 시작
 `~/.openpista/openpista.pid`에 PID 파일 저장
 정상 종료를 위한 `SIGTERM` / `Ctrl-C` 처리

### 웹 서버 전용 명령어

웹 어댑터만 다루고 싶다면 전용 라이프사이클 명령어를 사용하세요:

```bash
# 1) [channels.web] 설정 저장 + 정적 파일(static) 설치
#    최초 setup 시 안전한 web token을 자동 발급합니다.
openpista web setup --enable --port 3210

# 토큰 관련 옵션
openpista web setup --regenerate-token          # 새 토큰 강제 재발급
openpista web setup --yes                       # 재발급 확인 프롬프트 자동 승인
openpista web setup --token "manual-token"      # 토큰 직접 지정

# 2) 웹 전용 데몬 시작
openpista web start

# 3) 설정 + 런타임 상태(pid/health) 확인
openpista web status
```

`openpista web setup`은 `crates/channels/static` 파일을 `channels.web.static_dir`
(기본값 `~/.openpista/web`)로 복사하고 web 섹션 설정을 저장합니다.
기존 토큰이 있으면(대화형 터미널 기준) 재발급 여부를 물어봅니다.
비대화형 환경에서는 `--regenerate-token`을 주지 않으면 기존 토큰을 유지합니다.
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
│   ├── gateway/    # 프로세스 내 게이트웨이, 크론 스케줄러
│   ├── agent/      # ReAct 루프, OpenAI / Anthropic / Responses API, SQLite 메모리
│   ├── tools/      # Tool 트레이트 — BashTool, BrowserTool, ScreenTool, ContainerTool
│   ├── channels/   # CliAdapter, TelegramAdapter, WhatsAppAdapter, WebAdapter
│   ├── skills/     # SKILL.md 로더, 서브프로세스 + WASM 실행기
│   ├── web/        # Rust→WASM 브라우저 클라이언트 (wasm-bindgen, H5 채팅 UI)
│   └── cli/        # 바이너리 진입점, clap, config, TUI (ratatui + crossterm)
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
   docs: update installation guide
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
