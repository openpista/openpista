# 명령어 가이드 (COMMANDS)

`openpista`에서 자주 사용되는 명령어들을 `CLI`와 `TUI` 카테고리별로 정리했습니다.

## CLI 명령어 (CLI Commands)
### 인증 (권장 — 가장 먼저 실행)

OAuth PKCE 브라우저 로그인이 권장되는 인증 방법입니다. OpenAI, Anthropic, OpenRouter에서 API 키 없이 바로 사용 가능합니다.

```bash
# 대화형 인증 제공자(Provider) 선택창 열기 (권장)
openpista auth login
openpista auth login --non-interactive --provider openai --api-key "$OPENAI_API_KEY"
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"
# 로그아웃
openpista auth logout --provider openai
openpista auth status
```

> **자격증명 해석 우선순위:** config 파일 / `openpista_API_KEY` → credential store (`auth login`) → 프로바이더 환경 변수 → `OPENAI_API_KEY` 폴백
> 대부분의 사용자는 `openpista auth login`만으로 충분합니다.

### 기본 실행

```bash
# TUI 실행 (기본값)
openpista
openpista tui
openpista -s SESSION_ID
openpista tui -s SESSION_ID
openpista start
openpista run -e "프롬프트를 입력하세요"
```

### 모델 카탈로그 (Model Catalog)

```bash
# 사용 가능한 모델 목록 출력
openpista model list
openpista model -m "안녕하세요" gpt-4o
openpista model
```

### 전역 플래그 (Global Flags)

```bash
# 커스텀 설정 파일 지정
openpista --config /path/to/config.toml

# 로그 레벨 설정 (trace, debug, info, warn, error)
openpista --log-level debug

# 디버그 로그를 파일로 출력
openpista --debug
```

---

## TUI 명령어 (TUI Commands)

### 슬래시 명령어 (Slash Commands)

```txt
/help                    - 사용 가능한 TUI 명령어 목록 보기
/login                   - 인증을 위한 제공자 선택창 열기
/connection              - /login 과 동일 (별칭)
/model                   - 모델 브라우저 열기
/model list              - 사용 가능한 모델을 채팅에 출력
/session                 - 세션 브라우저 열기
/session new             - 새 세션 시작
/session load <id>       - ID로 세션 로드
/session delete <id>     - ID로 세션 삭제
/clear                   - 대화 기록 지우기
/quit                    - TUI 종료
/exit                    - TUI 종료 (/quit 과 동일)
```

> **팁:** 입력 창에서 `Tab`을 누르면 슬래시 명령어 자동완성 팔레트가 열립니다. 화살표 키로 탐색하고 `Enter`로 선택하세요.

---

## 사이드바 단축키 (Sidebar Keybinds)

```txt
Tab:          사이드바 토글 (열기/닫기)
j 또는 ↓:    다음 세션으로 이동
k 또는 ↑:    이전 세션으로 이동
Enter:        선택한 세션 로드
d 또는 Delete: 선택한 세션 삭제 요청 (확인 대화상자 표시)
Esc:          사이드바 포커스 해제
```

---

## 세션 브라우저 단축키 (Session Browser Keybinds)

`/session` 명령어로 열리는 전체 화면 세션 브라우저입니다.

```txt
텍스트 입력:  세션 제목으로 검색 필터링
j 또는 ↓:    다음 세션으로 이동
k 또는 ↑:    이전 세션으로 이동
Enter:        선택한 세션 로드
n:            새 세션 생성
d 또는 Delete: 선택한 세션 삭제 요청
Esc:          세션 브라우저 닫기
```

### 삭제 확인 대화상자 (ConfirmDelete Dialog)

```txt
y 또는 Enter: 삭제 확인
n 또는 Esc:  취소
```

---

## 로그인 브라우저 단축키 (Login Browser Keybinds)

```txt
↑/↓, j/k:    이동
Enter:        선택 / 다음 단계로 진행
텍스트 입력:  검색 또는 정보 입력
Backspace:    입력 지우기
Esc:          이전 단계로 돌아가거나 종료
```

---

## 모델 브라우저 단축키 (Model Browser Keybinds)

`/model` 명령어로 열리는 전체 화면 모델 브라우저입니다.

```txt
s 또는 /:    검색 모드 진입
텍스트 입력:  모델 ID 부분 일치 검색
Backspace:   검색어 지우기
j/k, ↑/↓:   이동
PgUp/PgDn:  페이지 단위 이동
Enter:       선택한 모델을 현재 세션 모델로 적용
r:           강제 새로고침 (Force Refresh)
Esc:         (검색 모드에서) 검색 종료, (일반 모드에서) 브라우저 종료
```

---

## 채팅 단축키 (Chat Keybinds)

```txt
Enter:                  메시지 전송
Shift+Enter:            줄 바꿈
↑/↓ 또는 스크롤:        채팅 히스토리 스크롤
마우스 드래그:           텍스트 선택
Ctrl+C 또는 Cmd+C:      선택한 텍스트 복사
Tab:                    슬래시 명령어 자동완성 팔레트 열기
```
