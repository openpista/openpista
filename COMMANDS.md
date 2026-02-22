# 명렁어 가이드 (COMMANDS)

`openpista`에서 자주 사용되는 명령어들을 `CLI`와 `TUI` 카테고리별로 정리했습니다.

## CLI 명령어 (CLI Commands)

### 기본 실행

```bash
# TUI 실행 (기본값)
openpista
openpista tui

# 데몬 실행
openpista start

# 단일 요청 실행 (스크립트용)
openpista run -e "프롬프트를 입력하세요"
```

### 모델 카탈로그 (Model Catalog)

```bash
# 기본: 추천 코딩 모델 목록 확인
openpista models list
```

### 인증 (Authentication)

```bash
# 대화형 인증 제공자(Provider) 선택창 열기
openpista auth login

# 비대화형 로그인 (CI 또는 스크립트용)
openpista auth login --non-interactive --provider opencode --api-key "$OPENCODE_API_KEY"

# 엔드포인트와 키를 같이 사용하는 제공자 예시
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"

# 로그아웃
openpista auth logout --provider openai

# 저장된 인증 상태 확인
openpista auth status
```

## TUI 명령어 (TUI Commands)

### 슬래시 명령어 (Slash Commands)

```txt
/help                    - 사용 가능한 TUI 명령어 목록 보기
/login                   - 인증을 위한 제공자 선택창 열기
/connection              - /login 과 동일 (별칭)
/models                  - 모델 브라우저 열기 (상단에 추천 모델, 하단에 전체 모델 표시)
/clear                   - 대화 기록 지우기
/quit                    - TUI 종료
/exit                    - TUI 종료 (/quit 과 동일)
```

### 로그인 브라우저 단축키 (Login Browser Keybinds)

```txt
↑/↓, j/k: 이동
Enter: 선택 / 다음 단계로 진행
텍스트 입력: 검색 또는 정보 입력
Backspace: 입력 지우기
Esc: 이전 단계로 돌아가거나 종료
```

### 모델 브라우저 단축키 (Model Browser Keybinds)

```txt
s 또는 /: 검색 모드 진입
입력/Backspace: 모델 ID 부분 일치 검색
j/k, ↑/↓: 이동
PgUp/PgDn: 페이지 단위 이동
Enter: 선택한 모델을 현재 세션 모델로 적용
r: 강제 새로고침 (Force Refresh)
Esc: (검색 모드에서) 검색 종료, (일반 모드에서) 브라우저 종료
```
