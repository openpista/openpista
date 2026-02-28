# 로드맵 (Roadmap)

> **openpista** — 모든 메신저를 통해 OS를 제어하는 자율 AI 에이전트.

---

## v0.1.0 — 초기 자율 에이전트 릴리스

첫 번째 공개 릴리스에서는 핵심 자율 루프를 확립합니다: LLM이 메시지를 수신하고, 사용 가능한 도구를 추론하며, OS 명령을 실행하고, 응답합니다 — 이 모든 것이 수동 개입 없이 이루어집니다.

### 핵심 런타임 (Core Runtime)

- [x] 에이전트 ReAct 루프 (LLM → 도구 호출 → 결과 → LLM → 텍스트 답변)
- [x] OpenAI 호환 어댑터(`async-openai`)가 있는 `LlmProvider` 트레잇(trait)
- [x] `ToolRegistry` — 동적 도구 등록 및 디스패치
- [x] 무한 루프 방지를 위한 구성 가능한 최대 도구 라운드 (기본값: 30)
- [x] 모든 요청 시 시스템 프롬프트에 스킬 문맥(context) 주입

### 에이전트 프로바이더 (Agent Providers)

- [x] `OpenAiProvider` — `async-openai`를 통한 표준 OpenAI 채팅 완성(chat completions) API
- [x] `ResponsesApiProvider` — SSE 스트리밍을 지원하는 OpenAI Responses API (`/v1/responses`); JWT에서 추출한 `chatgpt-account-id`를 통한 ChatGPT Pro 구독자 지원; 도구 이름 충돌 감지
- [x] `AnthropicProvider` — Anthropic Messages API; 시스템 메시지 추출; 연속 tool-result 병합; 도구 이름 정규화 (점을 밑줄로 변환); `anthropic-beta: oauth-2025-04-20`을 사용한 OAuth Bearer 인증
- [x] 6가지 프로바이더 프리셋: `openai`, `claude` / `anthropic`, `together`, `ollama`, `openrouter`, `custom`
- [x] OpenAI, Anthropic, OpenRouter에 대한 OAuth PKCE 지원
- [x] 확장 프로바이더 자격증명 슬롯: GitHub Copilot, Google Gemini, Vercel AI Gateway, Azure OpenAI, AWS Bedrock

### OS 도구 (OS Tools)

- [x] `system.run` — 구성 가능한 타임아웃(기본값: 30초)을 가진 BashTool
- [x] 명확한 프롬프트 표시와 함께 10,000자로 출력 제한(truncation)
- [x] 결과에 종료 코드(exit code)와 함께 stdout + stderr 캡처
- [x] 작업 디렉토리 재정의(override) 지원
- [x] `screen.capture` — `screenshots` 크레이트를 통한 디스플레이 스크린샷, base64 출력
- [x] `browser.navigate`, `browser.click`, `browser.type`, `browser.screenshot` — `chromiumoxide`를 통한 Chromium CDP

### 게이트웨이 (Gateway)

- [x] `ChannelRouter` — `DashMap` 기반 채널-투-세션 매핑을 갖춘 프로세스 내(in-process) 게이트웨이
- [x] `CronScheduler` — `tokio-cron-scheduler`를 통한 예약된 메시지 디스패치


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
- [x] `MobileAdapter` — QUIC 양방향 스트림, 토큰 기반 인증, `rcgen`을 통한 자체 서명 TLS
- [x] 응답 라우팅: CLI 응답 → stdout, 텔레그램 응답 → 봇 API
- [x] 사용자에게 명확히 표시되는 오류 응답
 - [x] `WebAdapter` — axum WebSocket 서버 + 정적 H5 채팅 UI (`static/`) 서빙; Rust→WASM 클라이언트 진행 중 (웹 채널 어댑터 섹션 참조)


### WhatsApp 채널 어댑터 (WhatsApp Channel Adapter)

> WhatsApp은 서브프로세스 브릿지 패턴을 사용합니다: openpista가 Node.js 프로세스(Baileys)를 생성하여 WhatsApp Web에 연결합니다. stdin/stdout 위의 JSON 라인으로 통신하며, QR 코드 스캔으로 페어링합니다 — API 키 불필요.

 [x] `WhatsAppAdapter` — Node.js Baileys 브릿지 서브프로세스를 통한 WhatsApp Web 멀티 디바이스 프로토콜
 [x] TUI에서 QR 코드 페어링 플로우 (`/whatsapp` 명령) — API 키 불필요
 [x] 서브프로세스 브릿지 프로토콜 (stdin/stdout 위의 JSON 라인) Rust ↔ Node.js 통신
 [x] 세션 지속성 (`session_dir/auth/`) — 재시작 후 자동 재연결
 [x] 대화별 안정적인 세션: `whatsapp:{sender_phone}` 채널 ID 및 세션 매핑
 [x] `WhatsAppConfig` — `[channels.whatsapp]` 설정 섹션: `enabled`, `session_dir`, `bridge_path`
 [x] 환경 변수 재정의: `WHATSAPP_SESSION_DIR`, `WHATSAPP_BRIDGE_PATH`
 [x] 수신 메시지 파싱: 텍스트 (이미지, 오디오, 비디오, 문서 — 향후 예정)
 [x] `/whatsapp status` TUI 명령 — 페어링 상태 및 설정 표시
 - [ ] 미디어 메시지 다운로드 및 전달 (수신 미디어 → base64 또는 로컬 경로로 에이전트 컨텍스트에 전달)
 - [ ] 인터랙티브 메시지 지원: 응답 버튼, 목록 메시지, 빠른 답장
 - [ ] 일시적 연결 장애에 대한 지수 백오프(exponential backoff) 재시도 로직
- [x] 사용자에게 명확히 표시되는 오류 응답 (❌ 접두사, 다른 어댑터와 일관됨)
- [x] 응답 라우팅 통합: WhatsApp 응답 → Graph API `send_message`
 - [ ] 다중 번호 지원: 여러 번호를 가진 비즈니스 계정을 위한 구성 가능한 전화번호 ID
- [x] 유닛 테스트: 웹훅 검증, 메시지 파싱, 세션 ID 생성, 응답 포맷팅, 서명 검증
 - [ ] 통합 테스트: 엔드-투-엔드 웹훅 → `ChannelEvent` → `AgentResponse` → WhatsApp 전송 흐름

#### 참고 오픈소스 프로젝트 (Reference Open-Source Projects)

> **Rust 크레이트**
>
> | 크레이트 | 설명 |
> |----------|------|
> | [`whatsapp-business-rs`](https://github.com/veecore/whatsapp-business-rs) | WhatsApp Business Cloud API 풀 SDK — axum 웹훅 서버 내장, HMAC-SHA256 검증, 메시지 송수신. 최우선 후보. |
> | [`whatsapp-cloud-api`](https://github.com/sajuthankappan/whatsapp-cloud-api-rs) | Meta Graph API 경량 클라이언트 (30k+ 다운로드). 웹훅 서버 미포함 — 별도 axum 핸들러와 조합 필요. |
> | [`whatsapp_handler`](https://github.com/bambby-plus/whatsapp_handler) | 웹훅 메시지 처리 + 미디어/인터랙티브 메시지 전송 지원. |
>
> **유사 아키텍처 Rust AI 에이전트**
>
> | 프로젝트 | 설명 |
> |----------|------|
> | [`zeroclaw`](https://github.com/zeroclaw-labs/zeroclaw) | trait 기반 `Channel` 패턴이 openpista의 `ChannelAdapter`와 거의 동일. WhatsApp 포함 다채널 지원. |
> | [`opencrust`](https://github.com/opencrust-org/opencrust) | 동일한 `crates/` 워크스페이스 구조. `whatsapp/webhook.rs` + `api.rs` 분리 모듈 패턴 참고. |
> | [`localgpt`](https://github.com/localgpt-app/localgpt) | `bridges/whatsapp/` 브릿지 패턴으로 WhatsApp 통합. |
> | [`loom`](https://github.com/ghuntley/loom) | Rust 워크스페이스 내 axum 기반 `routes/whatsapp.rs` 라우트 핸들러. |
>
> **API 스펙 레퍼런스 (TypeScript)**
>
> | 프로젝트 | 설명 |
> |----------|------|
> | [`WhatsApp-Nodejs-SDK`](https://github.com/WhatsApp/WhatsApp-Nodejs-SDK) | Meta 공식 SDK — 웹훅 페이로드 스키마 및 API 엔드포인트 스펙의 권위 있는 출처. |
> | [`whatsapp-business-sdk`](https://github.com/MarcosNicolau/whatsapp-business-sdk) | 깔끔한 TypeScript 타입 정의와 Business Cloud API에 대한 좋은 테스트 커버리지. |
>
> **Axum 웹훅 HMAC-SHA256 패턴**
>
> | 리소스 | 설명 |
> |--------|------|
> | [pg3.dev — GitHub Webhooks in Rust with Axum](https://pg3.dev/post/github_webhooks_rust) | HMAC-SHA256 + axum 완성형 튜토리얼. `X-Hub-Signature-256` 형식이 WhatsApp과 동일. |
> | [`axum-github-hooks`](https://github.com/rustunit/axum-github-hooks) | 웹훅 서명 검증을 위한 axum extractor 패턴 — `WhatsAppWebhookPayload` extractor로 응용 가능. |


### 웹 채널 어댑터 (Web Channel Adapter — Rust→WASM + WebSocket)

> 웹 어댑터는 네이티브 앱 없이 openpista를 모든 폰 또는 데스크톱 브라우저로 가져옵니다. 클라이언트는 Rust로 작성되어 WASM으로 컴파일되며, H5 채팅 UI와 함께 서빙됩니다. 통신은 모든 브라우저에서 보편적으로 지원되는 표준 WebSocket을 사용합니다.

#### 서버 (axum)

 - [x] `WebAdapter` — axum HTTP 서버: WebSocket 업그레이드 + WASM 번들용 정적 파일 서빙
 - [x] WebSocket 메시지 프레이밍: WS 텍스트 프레임 위의 JSON `WsMessage` 봉투 (`UserMessage`, `AgentReply`, `Ping`, `Pong`, `Auth`, `AuthResult`, `ModelChange`, `ModelChanged`, `ModelList`, `ProviderAuthRequest`, `ProviderAuthStatus`, `ProviderLogin`)
 - [x] 2단계 인증: `POST /auth` 세션 토큰 발급 + `?session_token=`을 통한 WebSocket 업그레이드; 레거시 `?token=` 쿼리 파라미터 폴백
 - [x] `WebConfig` — `[channels.web]` 설정 섹션: `port`, `token`, `cors_origins`, `static_dir`
 - [x] 환경 변수 재정의: `openpista_WEB_TOKEN`, `openpista_WEB_PORT`
 - [x] 세션 매핑: 인증된 클라이언트별 안정적인 세션을 가진 `web:{client_id}` 채널 ID
 - [x] 자동 재연결 지원: `Ping`/`Pong` 킵얼라이브 + 클라이언트 측 지수 백오프 재연결 루프; 만료 토큰 복구
 - [x] URL 기반 세션 라우팅: `GET /s/{session_id}`로 직접 세션 접근을 위한 `index.html` 서빙
 - [x] 크로스 오리진 브라우저 접근을 위한 CORS 설정
 - [x] 웹 OAuth PKCE 프로바이더 인증: 브라우저에서 시작하는 OpenAI, Anthropic, OpenRouter, GitHub Copilot OAuth; Together, Ollama, custom 프로바이더의 API 키 입력
 - [x] WebSocket을 통한 런타임 모델 전환: 세션 연결 해제 없이 `model_change` → `model_changed`
 - [x] 에이전트 처리 타임아웃: `runtime.process()`에 120초 `tokio::time::timeout` 래퍼 적용으로 무응답 방지
 - [x] `DashMap` 기반 클라이언트별 응답 라우팅 — 클론된 센더 패턴 (lock-across-await 방지)
 - [x] WASM 번들 및 H5 에셋을 위한 구성 가능한 정적 파일 디렉토리
 - [x] 도구 호출 승인 시스템: 모든 채널에서 채팅 내 인라인 승인 UI; `ToolApprovalRequest` / `ToolApprovalResponse` WebSocket 메시지 쌍; 세션별 "모두 허용" 토글
 - [x] 대화 히스토리 검증: 프로바이더 전환 시 고아 `tool_use` 블록 제거; 도구 이력 존재 시 빈 출력 ≠ 인증 오류 처리
 - [x] 화면 캡처 데이터 정제: LLM 전송 전 수 MB 크기 `data_b64` 필드 제거; 프론트엔드에서 인라인 이미지 렌더링을 위해 원본 데이터 보존
 - [ ] 리버스 프록시 또는 `axum-server` + `rustls`를 통한 WSS (TLS) 지원

#### 클라이언트 (Rust→WASM)

 - [x] `wasm-pack`을 통해 `wasm32-unknown-unknown`으로 컴파일되는 Rust 클라이언트 크레이트 (`crates/web/`)
 - [x] `wasm-bindgen` JS 인터롭: WebSocket API, DOM 조작, localStorage
 - [x] WebSocket 연결 관리자: 연결, 지수 백오프 자동 재연결 (1초~30초, 최대 10회), `Ping`/`Pong` 하트비트
 - [x] 메시지 직렬화: `ChannelEvent` / `AgentResponse`를 위한 WASM 내 `serde_json`
 - [x] 세션 지속성: 페이지 새로고침 시 클라이언트 ID, 인증 토큰, 세션 토큰 유지를 위한 `localStorage`
 - [x] 토큰 영속 저장: 디바이스당 한 번 인증; 저장된 토큰으로 페이지 로드 시 자동 연결
 - [x] H5 채팅 UI: 모바일 대응 채팅 인터페이스 (`static/index.html` + `style.css` + `app.js`; 바닐라 JS, Rust→WASM 미전환)
 - [x] 세션 관리: 사이드바 세션 목록 — 새 대화 생성(New Chat), Claude 스타일 ⋯ 컨텍스트 메뉴 (이름 변경 / 확인 대화상자 포함 삭제)
 - [x] 세션 이름 커스터마이징: localStorage 영속 저장을 통한 인라인 편집; 기본값은 세션 ID 축약형
 - [x] URL 기반 세션 접근: `/s/{session_id}` 경로 라우팅으로 자동 세션 로드
 - [x] 모델 선택기: 네비게이션 바의 드롭다운으로 프로바이더별 그룹화된 모델 표시; 인증된 프로바이더의 모델만 필터링
 - [x] 동적 모델 전환: 활성 세션 내에서 재연결 없이 모델 변경; 서버 측 모델 교체가 대화를 유지
 - [x] 마크다운 렌더링: 에이전트 응답을 HTML로 렌더링 (제목, 코드 블록, 목록, 테이블, 인용문, 링크, 강조)
 - [x] 반응형 스크롤 채팅 레이아웃: 새 메시지 및 에이전트 생각 중 자동 하단 스크롤
 - [x] 생각 중 표시기: 에이전트 처리 중 표시되는 바운싱 점 애니메이션
 - [x] 토스트 알림: 비침습적 시스템 이벤트 메시지 (연결 상태, 모델 변경, 인증 결과)
 - [x] 프로바이더 인증 모달: 11개 프로바이더의 인증 상태 표시; OAuth 로그인, API 키 입력, 엔드포인트+키 설정, 인증 코드 입력 지원
 - [x] 인증 기반 모델 필터링: 인증된 프로바이더의 모델만 표시하는 서버 측 필터링; 인증 상태 변경 시 모델 목록 자동 갱신
 - [x] 생성 중지 기능: Claude 스타일 원형 중지 버튼 + ESC 키 취소; 백엔드에서 클라이언트 응답 채널을 드롭하여 에이전트 중단; `CancelGeneration` / `GenerationCancelled` WebSocket 메시지 쌍
 - [x] 보안 강화: Unix에서 자격증명 파일 `chmod 600`; OAuth 팝업에 `noopener,noreferrer` 적용; 대기 중인 PKCE 흐름에 10분 TTL 적용
 - [x] 인라인 도구 승인: 도구 호출 승인이 채팅 흐름 내에서 렌더링 (모달 오버레이 아님); 허용 / 거부 / 모두 허용 버튼과 부드러운 스타일링; 승인 대기 중 타임아웃 일시 정지
 - [x] 인라인 이미지 렌더링: `screen.capture` base64 데이터가 채팅에서 `<img>` 태그로 렌더링; 정제된 도구 출력으로 base64 범람 방지
 - [x] 자동화된 Trunk 빌드 파이프라인: `scripts/build-web.sh` — 정적 파일 동기화 → JS 유효성 검증 (`node -c`) → `trunk build --release` → `~/.openpista/web/`에 배포; 선택적 `--restart` 플래그
 - [x] 레거시 모델 정리: 카탈로그에서 더 이상 사용되지 않는 모델 항목 제거; 모든 모델에 최대 추론 노력 및 사고 모드 활성화 구성
 - [x] 크로스 프로바이더 대화 수정: 프로바이더 전환 시 `tool_use` ID 불일치 오류 방지를 위한 대화 히스토리 검증; 도구 이력 존재 시 빈 LLM 출력 올바르게 처리
 - [ ] 스트리밍 응답 표시: 에이전트 출력 생성 시 점진적 텍스트 렌더링
 - [ ] 슬래시 명령어 지원: 웹 UI 입력에서 `/model`, `/session`, `/clear`, `/help`
 - [ ] 미디어 첨부 지원: 이미지 업로드 → base64 인코딩 → 에이전트 컨텍스트
 - [ ] PWA 매니페스트: 홈 화면 앱으로 설치 가능 (오프라인 셸 + 온라인 WebSocket)
 - [ ] CI에서 `wasm-pack build --target web` 빌드 파이프라인

#### 품질 (Quality)

 - [x] 유닛 테스트: WebSocket 핸드쉐이크, 세션 토큰 인증, 메시지 프레이밍, ping/pong, CORS, 세션 매핑, 응답 라우팅, 브로드캐스트 폴백 — 25개 이상 테스트 (`channels/src/web.rs`)
 - [ ] 통합 테스트: 브라우저 → WebSocket → `ChannelEvent` → `AgentResponse` → 브라우저 렌더
 - [ ] WASM 번들 크기 최적화: `wasm-opt`, 트리 셰이킹, gzip/brotli 서빙

#### 참고 오픈소스 프로젝트 (Reference Open-Source Projects)

> **Axum WebSocket 서버 패턴**
>
> | 프로젝트 | 설명 |
> |----------|------|
> | [axum — chat 예제](https://github.com/tokio-rs/axum/blob/main/examples/chat/src/main.rs) | 공식 Axum 브로드캐스트 기반 WebSocket 채팅 예제. `WebSocketUpgrade` + `tokio::sync::broadcast` 패턴의 최적 출발점. |
> | [axum — websockets 예제](https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs) | 공식 Axum WebSocket 예제 — 일반적인 WS 처리, ping/pong, 연결 생명주기 시연. |
> | [0xLaurens/chatr](https://github.com/0xLaurens/chatr) | WebSocket과 Axum을 사용한 채팅 룸. 룸 기반 세션 아키텍처 시연. |
> | [kumanote/axum-chat-example-rs](https://github.com/kumanote/axum-chat-example-rs) | Dragonfly(Redis 호환) pub/sub을 사용한 Axum WebSocket 채팅. 다중 인스턴스 스케일링 참고. |
> | [`danielclough/fireside-chat`](https://github.com/danielclough/fireside-chat) | LLM 채팅 봇: Axum WebSocket 백엔드 + Leptos WASM 프론트엔드 — openpista의 목표 Web 어댑터 아키텍처와 가장 근접한 레퍼런스. |
> | [`dawnchan030920/axum-ycrdt-websocket`](https://github.com/dawnchan030920/axum-ycrdt-websocket) | 컨넥션별 상태와 멀티클라이언트 브로드캐스트를 갖춘 Axum WebSocket 미들웨어 — 룸을 인식 WebSocket 핸들러 패턴 참고. |
>
> **Rust→WASM WebSocket 클라이언트 라이브러리**
>
> | 크레이트 | 설명 |
> |----------|------|
> | [wasm-bindgen — WebSocket 예제](https://github.com/rustwasm/wasm-bindgen/tree/main/examples/websockets) | wasm-bindgen의 정규 `web-sys` WebSocket 예제. WASM WS 클라이언트의 기초. |
> | [`ewebsock`](https://github.com/rerun-io/ewebsock) | 크로스 플랫폼(네이티브 + WASM) WebSocket 클라이언트. 단일 통합 API 제공. Rerun 팀 제작. |
> | [`tokio-tungstenite-wasm`](https://github.com/TannerRogalsky/tokio-tungstenite-wasm) | `web-sys`(WASM)와 `tokio-tungstenite`(네이티브)를 하나의 크로스 플랫폼 API로 래핑. crates.io 다운로드 ~343k. |
> | [`ws_stream_wasm`](https://github.com/najamelan/ws_stream_wasm) | WASM 전용 WebSocket 라이브러리. `WsMeta`/`WsStream`을 통한 `AsyncRead`/`AsyncWrite` 제공. 스트림 기반 패턴에 최적. |
> | [`cunarist/tokio-with-wasm`](https://github.com/cunarist/tokio-with-wasm) | 브라우저에서 tokio 런타임 에뮬레이션 — WASM 컨텍스트에서 `tokio::spawn`, `spawn_blocking` 등 사용 가능. |
> | [`gloo-net`](https://github.com/rustwasm/gloo) | 공식 `rustwasm` 조직의 ergonomic `web-sys` 래퍼; `gloo_net::websocket`이 `Stream`/`Sink` WebSocket API를 제공 — 로우레벨 `web-sys`보다 간결. |
>
> **Rust WASM 프론트엔드 프레임워크 (H5 채팅 UI용)**
>
> | 프레임워크 | 설명 |
> |------------|------|
> | [`yew`](https://github.com/yewstack/yew) (~30.5k 스타) | 가장 성숙한 Rust/WASM 프론트엔드 프레임워크. React 유사 컴포넌트 모델과 JSX 스타일 `html!` 매크로. |
> | [`leptos`](https://github.com/leptos-rs/leptos) (~20k 스타) | 풀스택 동형(isomorphic) Rust 프레임워크. 세밀한 반응성(fine-grained reactivity). `Stream`을 통한 네이티브 WebSocket 지원. |
> | [`dioxus`](https://github.com/DioxusLabs/dioxus) (~24.5k 스타) | 크로스 플랫폼(웹 + 데스크톱 + 모바일) 앱 프레임워크. Axum과의 깊은 통합으로 풀스택 Rust 구현 가능. |
> | [`leptos-use` — `use_websocket`](https://github.com/Synphonyte/leptos-use) | Leptos용 반응형 WebSocket 훅(hook). 코덱 지원 포함. VueUse에서 영감. |
> | [`leptos_server_signal`](https://github.com/tqwewe/leptos_server_signal) | WebSocket을 통해 서버와 동기화되는 Leptos 시그널. Axum 및 Actix 백엔드 지원. |
> | [`security-union/yew-websocket`](https://github.com/security-union/yew-websocket) | Yew 코어에서 추출된 독립형 Yew WebSocket 서비스 크레이트. |
>
> **풀스택 Rust WebSocket 채팅 레퍼런스**
>
> | 프로젝트 | 설명 |
> |----------|------|
> | [`YewChat`](https://github.com/jtordgeman/YewChat) | Yew로 구축된 완전한 WebSocket 채팅 앱 — 라우팅, 에이전트, GIF 지원. 인기 튜토리얼 시리즈의 동반 프로젝트. |
> | [`ztm-project-uchat`](https://github.com/jayson-lennon/ztm-project-uchat) | 전체가 Rust로 작성된 풀스택 채팅 클론: WASM 프론트엔드(Trunk), Diesel ORM, PostgreSQL, Tailwind CSS. |
> | [`fullstack-rust-axum-dioxus-rwa`](https://github.com/dxps/fullstack-rust-axum-dioxus-rwa) | Axum 백엔드 + Dioxus WASM 프론트엔드의 RealWorld 앱 — 인증, 라우팅, CRUD 구현. |
> | [`rust-axum-leptos-wasm`](https://github.com/hvalfangst/rust-axum-leptos-wasm) | 풀스택 Axum + Leptos WASM. JWT 보호 엔드포인트 포함. |
> | [`veklov/rust-chat`](https://github.com/veklov/rust-chat) | Warp 백엔드 + Yew/WASM 프론트엔드 웹 채팅. 엔드-투-엔드 WebDriver 테스트 포함. |
> | [`ProstoyVadila/ws_chat`](https://github.com/ProstoyVadila/ws_chat) | 백엔드와 프론트엔드 모두 Rust로 작성된 WebSocket 채팅 — 서버/클라이언트 분리 크레이트 패턴 참고. |
> | [`bestia-dev/mem6_game`](https://github.com/bestia-dev/mem6_game) | Rust WASM + 실시간 WebSocket + PWA 서비스 워커를 동시에 커버하는 유일한 레퍼런스 — WASM+WS+PWA 세 가지를 함께 다루는 유일한 사례. |
>
> **WASM 빌드 도구 및 PWA**
>
> | 도구 | 설명 |
> |------|------|
> | [`trunk`](https://github.com/trunk-rs/trunk) (~4.2k 스타) | Rust용 최고의 WASM 웹 애플리케이션 번들러. 핫 리로드 내장 개발 서버. Yew, Leptos, Dioxus 모두 지원. |
> | [`wasm-pack`](https://github.com/rustwasm/wasm-pack) (~6.5k 스타) | 전통적인 Rust→WASM 워크플로우 도구. 참고: `rustwasm` 조직 2025년 중반 보관(archived); 커뮤니티 포크 [drager/wasm-pack](https://github.com/drager/wasm-pack). |
> | [`yew-wasm-pack-template`](https://github.com/nickkos/nickkos/yew-wasm-pack-template) | 풀스택 PWA 템플릿: Yew 프론트엔드 + Actix 백엔드, Workbox 서비스 워커 구성 포함. |
> | [`woz`](https://github.com/alexkehayias/woz) | Rust용 Progressive WebAssembly App(PWAA) 생성기 — PWA + WASM 도구를 단일 CLI로 결합. |
> | [`wasm-bindgen-service-worker`](https://github.com/justinrubek/wasm-bindgen-service-worker) | wasm-bindgen을 통해 전체가 Rust로 작성된 서비스 워커. 최소한의 JS 글루 코드. |

### 스킬 시스템 (Skills System)

- [x] `SkillLoader` — 작업 공간에서 재귀적 `SKILL.md` 검색
- [x] 발견된 모든 스킬로부터 문맥 연결(concatenation)
- [x] 하위 프로세스 실행: `run.sh` → bash, `main.py` → python/python3
- [x] 도구 에러로 표출되는 0이 아닌 종료 코드(Non-zero exit codes)
- [x] `openpista_WORKSPACE` 환경 변수 재정의(override)

### Docker 샌드박스 (Docker Sandbox)

- [x] `container.run` 도구 — 작업(task)당 격리된 Docker 컨테이너 생성
- [x] 작업별 임시 토큰: 컨테이너 시작 시 주입되고 종료 시 자동 폐기되는 짧은 수명의 크레덴셜
- [x] 오케스트레이터/워커 패턴: 메인 에이전트가 오케스트레이터로 동작하며 무겁거나 위험한 작업을 위해 워커 컨테이너 생성
- [x] 워커 컨테이너는 오케스트레이터 세션으로 결과를 다시 보고
- [x] 컨테이너 수준의 리소스 제한 적용: CPU 할당량, 메모리 제한, 기본적으로 네트워크 차단(no-network)
- [x] 워커가 호스트에 대한 쓰기 권한 없이 스킬/파일을 읽을 수 있도록 작업 공간 볼륨 마운트(읽기 전용)
- [x] 컨테이너 생명주기: 생성 → 토큰 주입 → 작업 실행 → 결과 수집 → 파기 (재사용 없음)
- [x] Docker API 통합을 위한 `bollard` 크레이트 (`docker` CLI 쉘 호출 아님)
- [x] 스킬별로 구성 가능한 베이스 이미지 (`SKILL.md`의 `image:` 필드)
- [x] Docker 데몬을 사용할 수 없는 경우 하위 프로세스(subprocess) 모드로 폴백

### WASM 스킬 샌드박스 (WASM Skill Sandbox)

- [x] 임베디드 WASM 런타임으로서의 `wasmtime` 통합
- [x] WASI 호스트 인터페이스: 제한된 파일 시스템(읽기 전용 작업 공간) + stdout/stderr
- [x] `SKILL.md`의 스킬 실행 모드 플래그 (`mode: wasm` vs `mode: subprocess`)
- [x] 호스트↔게스트 ABI: WASM 메모리를 통한 JSON 인코딩된 `ToolCall` 인자(args) 수신, `ToolResult` 반환
- [x] WASM fuel/epoch 수준에서 30초 실행 타임아웃 적용
- [x] 메모리 제한: WASM 스킬 인스턴스 당 64 MB
- [x] `skills/README.md`에 포함된 `cargo build --target wasm32-wasip1` 빌드 가이드
- [x] 저장소에 포함된 WASM 스킬 예제 (`skills/hello-wasm/`)

### CLI, 설정 및 TUI (CLI, Configuration & TUI)

- [x] `openpista start` — 전체 데몬 (활성화된 모든 채널)
- [x] `openpista run -e "..."` — 단발성(single-shot) 에이전트 명령
- [x] `openpista tui [-s SESSION_ID]` — 선택적 세션 재개를 포함한 TUI 실행
- [x] `openpista model [MODEL_OR_COMMAND]` — 모델 카탈로그 (목록 / 테스트)
- [x] `openpista -s SESSION_ID` — 세션 재개 단축 명령
- [x] `openpista auth login` — OAuth PKCE 브라우저 로그인 + 자격증명 영속 저장
- [x] 멀티 프로바이더 OAuth PKCE: OpenAI, Anthropic, OpenRouter, GitHub Copilot
- [x] GitHub Copilot PKCE OAuth — 구독 기반 인증 (GitHub OAuth → Copilot 세션 토큰 교환)
- [x] 프로바이더 로그인 선택기 (검색, OAuth/API 키 방식 선택, 자격증명 상태 표시)
- [x] 내부 TUI 슬래시 명령어 (`/help`, `/login`, `/clear`, `/quit`, `/exit`)
- [x] 중앙 집중식의 랜딩 페이지 스타일 TUI (전용 Home 및 Chat 화면 포함)
- [x] 문서화된 예제가 포함된 TOML 설정 파일 (`config.toml`)
- [x] 모든 시크릿(secrets)에 대한 환경 변수 재정의 기능
- [x] 시작 시 PID 파일 작성, 종료 시 제거
- [x] `SIGTERM` + `Ctrl-C` 우아한 종료(graceful shutdown)
- [x] Elm 아키텍처(TEA) 반응형 TUI — 단방향 데이터 흐름 (`Action → update() → State → view()`)

### 멀티 프로바이더 인증 (Multi-Provider Authentication)

- [x] OpenAI OAuth 2.0 PKCE 브라우저 로그인 (ChatGPT Plus/Pro 구독)
- [x] Anthropic OAuth 2.0 PKCE 코드 표시 흐름 (Claude Max 구독)
- [x] GitHub Copilot PKCE OAuth: GitHub OAuth → `copilot_internal/v2/token` 세션 토큰 교환
- [x] OpenAI `id_token` → API 키 교환 (구독 과금 Responses API)
- [x] 만료 5분 전 자동 토큰 갱신
- [x] `~/.openpista/credentials.toml`에 프로바이더별 토큰 영속 저장
- [x] 확장 프로바이더 슬롯: GitHub Copilot, Google, Vercel AI Gateway, Azure OpenAI, AWS Bedrock
- [x] `openpista auth status` — 모든 저장된 프로바이더 자격증명 및 만료 표시
- [x] `openpista auth logout` — 프로바이더별 자격증명 제거

### 품질 및 CI (Quality & CI)

- [x] 모든 크레이트에 걸친 755개의 유닛 + 통합 테스트 (`cargo test --workspace`)
- [x] 클리피(clippy) 경고 제로: `cargo clippy --workspace -- -D warnings`
- [x] 일관된 포맷팅: `cargo fmt --all`
- [x] `main` 브랜치에 대한 `push` / `pull_request` 시 GitHub Actions CI 워크플로우
- [x] Linux 크로스 빌드 매트릭스 (`x86_64/aarch64` × `gnu/musl`)
- [x] Codecov 커버리지 리포팅

### 문서화 및 릴리스 아티팩트 (Documentation & Release Artifacts)

- [x] 배지(CI, codecov, Rust 버전, 라이선스)를 포함한 `README.md`
- [x] `ROADMAP.md` (이 문서)
- [x] v0.1.0 항목이 포함된 `CHANGELOG.md`
- [ ] `LICENSE-MIT` 및 `LICENSE-APACHE`
- [ ] 모든 옵션이 문서화된 `config.example.toml`
- [ ] 미리 빌드된 바이너리가 포함된 GitHub 릴리스(Release):
  - [ ] `aarch64-apple-darwin` (macOS Apple Silicon)
  - [ ] `x86_64-unknown-linux-gnu` (Linux x86_64)
  - [ ] `aarch64-unknown-linux-gnu` (Linux ARM64)
  - [ ] `x86_64-unknown-linux-musl` (Linux x86_64 정적 링크)
  - [ ] `aarch64-unknown-linux-musl` (Linux ARM64 정적 링크)
- [ ] 라이브러리 크레이트에 대한 `crates.io` 퍼블리시 (선택사항)

---

## v0.2.0 — 플랫폼 통합 및 관측성 (Platform Integrations & Observability)

채널 표면을 확장하고 프로덕션 관측성(observability)을 추가합니다.

### 신규 OS 도구 (New OS Tools)

- [ ] `screen.ocr` — 화면 캡처 영역에서 OCR 텍스트 추출 (Tesseract 또는 `leptonica` 바인딩)
- [ ] `system.notify` — `notify-rust`를 통한 데스크톱 알림 (macOS, Linux)
- [ ] `system.clipboard` — 시스템 클립보드 읽기/쓰기

### MCP 통합 (MCP Integration)

- [ ] MCP (Model Context Protocol) 클라이언트 — MCP 호환 도구 서버와 openpista 연결
- [ ] MCP 도구 검색 및 `ToolRegistry`에 동적 등록
- [ ] MCP 리소스 및 프롬프트 지원
- [ ] 설정: `config.toml`의 `[mcp]` 섹션에 서버 URL 구성

### 플러그인 시스템 (Plugin System)

- [ ] 서드파티 도구 확장을 위한 Plugin 트레잇
- [ ] 공유 라이브러리(`.dylib` / `.so`) 또는 WASM을 통한 동적 로딩
- [ ] `~/.openpista/plugins/`에서 플러그인 매니페스트 형식 및 검색
- [ ] 플러그인 생명주기: 로드 → 도구 등록 → 언로드

### 추가 채널 어댑터 (Additional Channel Adapters)

- [ ] `DiscordAdapter` — `serenity` 크레이트를 통한 Discord 봇, 슬래시 명령, 스레드 기반 세션
- [ ] `SlackAdapter` — Bolt 스타일 HTTP 이벤트 API를 통한 Slack 봇, 채널/스레드 세션

### 관측성 (Observability)

- [ ] `metrics-exporter-prometheus`를 통한 Prometheus 메트릭 내보내기
- [ ] 핵심 메트릭: 요청 지연 시간, 도구 호출 횟수, 오류율, 활성 세션 수, 메모리 사용량
- [ ] 구성 가능한 포트에서 `/metrics` HTTP 엔드포인트
- [ ] OpenTelemetry 트레이싱 통합 (선택사항)
- [ ] `tracing-subscriber` JSON 출력을 통한 구조화된 로깅

### 워커 보고 시스템 (Worker Report System)

> 워커 컨테이너는 현재 프로세스 내에서 결과를 수집합니다. 이 섹션은 워커 보고, 모니터링 및 이력에 대한 향후 개선 사항을 추적합니다.

 [ ] 워커 보고 수신 엔드포인트 — `axum`을 통한 HTTP POST 라우트(`/api/worker-report`)로 워커 실행 결과 수신
 [ ] 보고 인증 — 보고 제출 시 워커 작업 토큰 검증 (ContainerTool의 기존 `TaskCredential` 재사용)
 [ ] 보고 확인 응답 프로토콜 — 구조화된 ACK/NACK 응답을 통한 신뢰성 있는 전달
 [ ] 지수 백오프 재시도 — ContainerTool HTTP 클라이언트의 일시적 장애 처리
 [ ] 오프라인 보고 버퍼 — 실패한 보고를 로컬 디스크(`~/.openpista/report-queue/`)에 큐잉, 연결 복구 시 재전송
 [ ] 워커 상태 WebSocket 피드 — 활성 컨테이너 실행에 대한 실시간 진행 상황을 TUI/Web UI에 푸시
 [ ] 워커 실행 이력 API — REST 엔드포인트 또는 TUI `/worker` 명령어를 통한 과거 워커 보고 조회
 [ ] TUI 워커 대시보드 — 활성/완료/실패 워커 실행 현황과 로그를 표시하는 전용 화면

---

## v1.0.0 — 프로덕션 준비 완료 (Production Ready)

- [ ] 모든 크레이트에 대한 안정적인 공개 API (semver 1.0 보장)
- [ ] 완전한 엔드-투-엔드 보안 감사 (제3자 수행)
- [ ] 장기 지원(LTS) 릴리스 약속
- [ ] 패키징: `brew` 포뮬러, `apt` 저장소, 공식 Docker 이미지
- [ ] docs.rs에 포괄적인 API 문서화
- [ ] 성능 벤치마크 및 최적화 패스
