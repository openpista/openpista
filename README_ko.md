# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** [English](README.md) | 한국어

**메신저로 OS를 제어하세요 — 텔레그램, 왓츠앱, 웹, CLI.**

단일 ~10 MB Rust 바이너리. LLM 프로바이더 6종. 런타임 의존성 제로.

<!-- TODO: demo GIF here -->

---

## 왜 openpista인가?

터미널 전용 AI 에이전트와 달리, openpista는 여러분이 있는 곳에서 만납니다:

- **멀티 채널** — 텔레그램, 왓츠앱, 자체 호스팅 웹 UI, 내장 CLI TUI로 채팅
- **단일 정적 바이너리** — ~10 MB, Rust, 런타임 의존성 없음, 어디서나 실행
- **LLM 프로바이더 6종** — OpenAI, Anthropic, Together, Ollama, OpenRouter, 커스텀 간 세션 중 전환
- **안전한 실행** — 위험한 명령은 일회용 Docker 컨테이너에서; 스킬은 wasmtime 격리 실행
- **확장 가능한 스킬** — 어떤 언어로든 스킬 작성, WASM으로 컴파일, 격리 및 버전 관리

---

## 빠른 시작

```bash
git clone https://github.com/openpista/openpista.git && cd openpista
cargo build --release && sudo cp target/release/openpista /usr/local/bin/
openpista auth login   # OAuth 브라우저 로그인 — API 키 불필요
openpista              # TUI 실행
```

> **사전 요구사항:** [Rust 1.85+](https://rustup.rs) · SQLite 3 (보통 기본 설치됨)

---

## 채널

```
  텔레그램    왓츠앱    웹 브라우저    CLI TUI
      \         |           |          /
       ╰── openpista gateway ──────╯
                   │
         AI Agent (ReAct loop)
                   │
     bash · browser · screen · docker
```

| 채널 | 설정 가이드 |
|------|------------|
| 텔레그램 | [use-channels/telegram.md](./use-channels/telegram.md) |
| 왓츠앱 | [use-channels/whatsapp.md](./use-channels/whatsapp.md) |
| 웹 UI (자체 호스팅) | [use-channels/web.md](./use-channels/web.md) |
| CLI TUI | [use-channels/cli.md](./use-channels/cli.md) |

---

## 기능

| 카테고리 | 기능 | 상태 |
|---------|------|------|
| **채널** | 텔레그램, 왓츠앱, 웹 UI, CLI TUI | ✅ v0.1.0 |
| **도구** | Bash · 브라우저(CDP) · 화면 캡처 · Docker 샌드박스 | ✅ v0.1.0 |
| **스킬** | SKILL.md 로더 · 서브프로세스 · WASM 격리 | ✅ v0.1.0 |
| **인증** | OAuth PKCE (OpenAI, Anthropic, OpenRouter) · API 키 | ✅ v0.1.0 |
| **메모리** | SQLite 대화 기록 · 세션 관리 | ✅ v0.1.0 |
| **플랫폼** | 크론 스케줄러 · 모델 카탈로그 · SSE 스트리밍 | ✅ v0.1.0 |

---

## 프로바이더

| 프로바이더 | 기본 모델 | 인증 방식 |
|-----------|----------|----------|
| `openai` (기본값) | gpt-5.3-codex| OAuth PKCE, API 키 |
| `claude` / `anthropic` | claude-sonnet-4-6 | OAuth PKCE, Bearer |
| `together` | meta-llama/Llama-3.3-70B-Instruct-Turbo | API 키 |
| `ollama` | llama3.2 | 없음 (로컬) |
| `openrouter` | openai/gpt-4o | OAuth PKCE, API 키 |
| `custom` | 직접 설정 | 직접 설정 |

기존 **ChatGPT Pro** 또는 **Claude Max** 구독을 OAuth로 활용 — 별도 API 키 불필요.

---

## 아키텍처

```
[ 채널 ]      텔레그램 · 왓츠앱 · CLI TUI · 웹 (WASM)
      │  tokio::mpsc  ChannelEvent
      ▼
[ 게이트웨이 ]  프로세스 내 라우터 · 크론 스케줄러
      │
      ▼
[ 에이전트 ]   ReAct 루프 · LLM 프로바이더 6종 · SQLite 메모리
      │  tool_call
      ▼
[ 도구 ]      system.run · browser.* · screen.capture · container.run
[ 스킬 ]      SKILL.md → 서브프로세스 / WASM
```


---

## 문서 & 커뮤니티

- [ROADMAP.md](./ROADMAP.md) — 향후 계획
- [CHANGELOG.md](./CHANGELOG.md) — 릴리즈 노트
- [use-channels/](./use-channels/) — 채널별 설정 가이드
- [config.example.toml](./config.example.toml) — 전체 설정 레퍼런스
- [COMMANDS.md](./COMMANDS.md) — 모든 CLI & TUI 명령어

기여 환영 — fork, 브랜치(`feat/...`), `cargo fmt && cargo clippy`, `main` 브랜치로 PR 오픈.

---

## 라이선스

[MIT](LICENSE-MIT) 또는 [Apache 2.0](LICENSE-APACHE) 중 선택하여 사용 가능합니다.
