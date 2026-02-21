# 로드맵 (Roadmap)

> **openpista** — QUIC 기반의 모든 메신저를 통해 OS를 제어하는 자율 AI 에이전트.

---

## v0.1.0 — 초기 자율 에이전트 릴리스

첫 번째 공개 릴리스에서는 핵심 자율 루프를 확립합니다: LLM이 메시지를 수신하고, 사용 가능한 도구를 추론하며, OS 명령을 실행하고, 응답합니다 — 이 모든 것이 수동 개입 없이 이루어집니다.

### 핵심 런타임 (Core Runtime)

- [x] 에이전트 ReAct 루프 (LLM → 도구 호출 → 결과 → LLM → 텍스트 답변)
- [x] OpenAI 호환 어댑터(`async-openai`)가 있는 `LlmProvider` 트레잇(trait)
- [x] `ToolRegistry` — 동적 도구 등록 및 디스패치
- [x] 무한 루프 방지를 위한 구성 가능한 최대 도구 라운드 (기본값: 10)
- [x] 모든 요청 시 시스템 프롬프트에 스킬 문맥(context) 주입

### OS 도구 (OS Tools)

- [x] `system.run` — 구성 가능한 타임아웃(기본값: 30초)을 가진 BashTool
- [x] 명확한 프롬프트 표시와 함께 10,000자로 출력 제한(truncation)
- [x] 결과에 종료 코드(exit code)와 함께 stdout + stderr 캡처
- [x] 작업 디렉토리 재정의(override) 지원
- [x] `screen.capture` 
- [x] `browser.*`

### 전송 및 게이트웨이 (Transport & Gateway)

- [x] 포트 4433에서 `quinn` + `rustls`를 통한 QUIC 서버
- [x] `rcgen`을 통한 자체 서명 TLS 인증서 자동 생성 (설정 불필요)
- [x] 양방향 QUIC 스트림 상의 길이 접두사(length-prefixed) JSON 프레이밍
- [x] 연결별 `AgentSession` 생명주기 관리
- [x] `ChannelRouter` — `DashMap` 기반 채널-투-세션 매핑
- [x] `CronScheduler` — `tokio-cron-scheduler`를 통한 예약된 메시지 디스패치
- [x] CLI/테스트를 위한 프로세스 내(in-process) 게이트웨이 (QUIC 불필요)

### 메모리 및 지속성 (Memory & Persistence)

- [x] `sqlx`를 통한 SQLite 대화 메모리
- [x] 시작 시 자동 마이그레이션 (`sqlx::migrate!`)
- [x] 세션 생성, 조회 및 타임스탬프 업데이트
- [x] 역할(role), 내용(content), 도구 호출(tool call) 메타데이터와 함께 메시지 저장/로드
- [x] 세션 간에 보존되는 도구 호출 JSON 직렬화
- [x] 데이터베이스 URL에 대한 `~` 경로 확장(expansion)

### 채널 어댑터 (Channel Adapters)

- [x] 플러그형 채널 구현을 위한 `ChannelAdapter` 트레잇
- [x] `CliAdapter` — `/quit` 종료 명령이 포함된 stdin/stdout
- [x] `TelegramAdapter` — 채팅별 안정적인 세션을 가진 `teloxide` 디스패처
- [x] 응답 라우팅: CLI 응답 → stdout, 텔레그램 응답 → 봇 API
- [x] 사용자에게 명확히 표시되는 오류 응답

### 스킬 시스템 (Skills System)

- [x] `SkillLoader` — 작업 공간에서 재귀적 `SKILL.md` 검색
- [x] 발견된 모든 스킬로부터 문맥 연결(concatenation)
- [x] 하위 프로세스 실행: `run.sh` → bash, `main.py` → python/python3
- [x] 도구 에러로 표출되는 0이 아닌 종료 코드(Non-zero exit codes)
- [x] `OPENPISTACRAB_WORKSPACE` 환경 변수 재정의(override)

### Docker 샌드박스 (Docker Sandbox)

- [x] `container.run` 도구 — 작업(task)당 격리된 Docker 컨테이너 생성
- [ ] 작업별 임시 토큰: 컨테이너 시작 시 주입되고 종료 시 자동 폐기되는 짧은 수명의 크레덴셜
- [ ] 오케스트레이터/워커 패턴: 메인 에이전트가 오케스트레이터로 동작하며 무겁거나 위험한 작업을 위해 워커 컨테이너 생성
- [ ] 워커 컨테이너는 QUIC 스트림을 통해 오케스트레이터 세션으로 결과를 다시 보고
- [ ] 컨테이너 수준의 리소스 제한 적용: CPU 할당량, 메모리 제한, 기본적으로 네트워크 차단(no-network)
- [ ] 워커가 호스트에 대한 쓰기 권한 없이 스킬/파일을 읽을 수 있도록 작업 공간 볼륨 마운트(읽기 전용)
- [ ] 컨테이너 생명주기: 생성 → 토큰 주입 → 작업 실행 → 결과 수집 → 파기 (재사용 없음)
- [ ] Docker API 통합을 위한 `bollard` 크레이트 (`docker` CLI 쉘 호출 아님)
- [ ] 스킬별로 구성 가능한 베이스 이미지 (`SKILL.md`의 `image:` 필드)
- [ ] Docker 데몬을 사용할 수 없는 경우 하위 프로세스(subprocess) 모드로 폴백

### WASM 스킬 샌드박스 (WASM Skill Sandbox)

- [x] 임베디드 WASM 런타임으로서의 `wasmtime` 통합
- [x] WASI 호스트 인터페이스: 제한된 파일 시스템(읽기 전용 작업 공간) + stdout/stderr
- [x] `SKILL.md`의 스킬 실행 모드 플래그 (`mode: wasm` vs `mode: subprocess`)
- [x] 호스트↔게스트 ABI: WASM 메모리를 통한 JSON 인코딩된 `ToolCall` 인자(args) 수신, `ToolResult` 반환
- [x] WASM fuel/epoch 수준에서 30초 실행 타임아웃 적용
- [x] 메모리 제한: WASM 스킬 인스턴스 당 64 MB
- [x] `skills/README.md`에 포함된 `cargo build --target wasm32-wasip1` 빌드 가이드
- [x] 저장소에 포함된 WASM 스킬 예제 (`skills/hello-wasm/`)

### CLI 및 설정 (CLI & Configuration)

- [x] `openpistacrab start` — 전체 데몬 (QUIC + 활성화된 모든 채널)
- [x] `openpistacrab run -e "..."` — 단발성(single-shot) 에이전트 명령
- [x] `openpistacrab repl` — 세션 지속성을 갖춘 대화형 REPL
- [x] `openpista auth login` — OAuth PKCE 브라우저 로그인 + 자격증명 영속 저장
- [x] 내부 TUI 슬래시 명령어 (`/help`, `/login`, `/clear`, `/quit`, `/exit`)
- [x] 중앙 집중식의 랜딩 페이지 스타일 TUI (전용 Home 및 Chat 화면 포함)
- [x] 문서화된 예제가 포함된 TOML 설정 파일 (`config.toml`)
- [x] 모든 시크릿(secrets)에 대한 환경 변수 재정의 기능
- [x] 시작 시 PID 파일 작성, 종료 시 제거
- [x] `SIGTERM` + `Ctrl-C` 우아한 종료(graceful shutdown)

### 품질 및 CI (Quality & CI)

- [x] 유닛 + 통합 테스트: `cargo test --workspace` (목표: 90+ 테스트)
- [x] 클리피(clippy) 경고 제로: `cargo clippy --workspace -- -D warnings`
- [x] 일관된 포맷팅: `cargo fmt --all`
- [ ] `main` 브랜치에 대한 `push` / `pull_request` 시 GitHub Actions CI 워크플로우
- [ ] Codecov 커버리지 리포팅

### 문서화 및 릴리스 아티팩트 (Documentation & Release Artifacts)

- [ ] 배지(CI, codecov, Rust 버전, 라이선스)를 포함한 `README.md`
- [ ] `ROADMAP.md` (이 문서)
- [ ] v0.1.0 항목이 포함된 `CHANGELOG.md`
- [ ] `LICENSE-MIT` 및 `LICENSE-APACHE`
- [ ] 모든 옵션이 문서화된 `config.example.toml`
- [ ] 미리 빌드된 바이너리가 포함된 GitHub 릴리스(Release):
  - [ ] `x86_64-apple-darwin` (macOS Intel)
  - [ ] `aarch64-apple-darwin` (macOS Apple Silicon)
  - [ ] `x86_64-unknown-linux-gnu` (Linux x86_64)
  - [ ] `aarch64-unknown-linux-gnu` (Linux ARM64)
- [ ] 라이브러리 크레이트에 대한 `crates.io` 퍼블리시 (선택사항)

---

## v0.2.0 — 화면 및 브라우저 (Screen & Browser)

OS 시각적 제어까지 도구의 표면을 확장합니다.

- `screen.capture` — `screenshots` 크레이트를 통한 base64/파일 스크린샷
- `screen.ocr` — 화면 영역에서 텍스트 추출
- `browser.navigate` — `chromiumoxide`를 거친 Chromium CDP
- `browser.click`, `browser.type`, `browser.screenshot`
- `system.notify` — `notify-rust`를 통한 데스크톱 알림
- Discord 어댑터
- Slack 어댑터
- Prometheus 메트릭 내보내기 (`metrics-exporter-prometheus`)

---

## v0.3.0 — 음성 및 다중 에이전트 (Voice & Multi-Agent)

- `voice.transcribe` — `whisper-rs`를 통한 마이크 입력
- `voice.speak` — TTS 출력
- 다중 에이전트 협업 (에이전트가 에이전트를 생성)
- 처리율 제한(Rate limiting) 및 안전 계층 (명령 허용목록/차단목록)
- 대안적 전송(transport) 수단으로서의 WebSocket 게이트웨이

---

## v1.0.0 — 프로덕션 준비 완료 (Production Ready)

- 모든 크레이트에 대한 안정적인 공개 API
- 완전한 엔드-투-엔드(end-to-end) 보안 검토
- 장기 지원(Long-term support) 보장
- 패키징: `brew`, `apt`, `winget`, Docker 이미지
