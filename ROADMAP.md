# DCC — Interactive Desktop Roadmap

목표: 모니터링 벽지를 넘어, 아이콘·위젯·런처를 대체하는 **종합 인터랙티브 데스크톱 레이어**.

범례: `[x]` 완료 · `[~]` 부분 완료/설정 필요 · `[ ]` 미착수
(2026-09-27 기준. 설정 파일은 dcc.toml 대신 `dcc.conf` 라인 포맷으로 구현됨)

## Phase 0 — 기반 공사 ✅

- [x] 입력 파이프라인: 마우스 메시지 → 셀 히트테스트 (즉시 리페인트, MsgWait 루프)
- [x] 히트영역 레지스트리 + UiState (호버/휠 스크롤/클릭 라우팅)
- [x] `dcc.conf` 로더 + 저장 즉시 반영 (launcher/rss/calendar/git/display 섹션)
- [x] 토스트 + 저널 코어 (모든 서브시스템의 단일 이벤트 채널)
- [x] 네트워크 워커 (WinHTTP, 주기 폴링, 실패 시 이전 값 유지)
- [x] 단일 인스턴스 가드 + `--restore-icons` 색 복원 버그 수정
- [x] (추가) 전각(한글) 렌더링 — 2셀 글리프 + 셀 고정 페인팅
- [x] (추가) 셀 밀도 상향 200→273열, `--shot` 프리뷰에 피드 프리페치

## Phase 1 — 로컬 도구 ✅

- [x] 런처 (dcc.conf → 팔레트에서 실행; 벽면 패널 자리는 day · anomalies로 교체)
- [x] 클립보드 히스토리 (재시도 큐, 클릭 재복사)
- [x] 다운로드 감시 (완료 토스트, 저널 클릭 → 열기)
- [x] GPU 패널 (nvml.dll 직접 로드 — 사용률/VRAM/온도/전력/그래프)
- [x] 시간 진행률 (Y/M/D)
- [x] 뽀모도로 — 브라유 고해상도 파이, 시간 숨김, 호버 시 표시, 클릭/우클릭

## Phase 2 — 피드 ✅

- [x] 날씨 (Open-Meteo, 위치는 dcc.conf [weather] — 기본 서울)
- [x] 환율 USD/xxx + 업비트 코인 하나 (dcc.conf [ticker], 등락률)
- [x] 뉴스 — RSS/Atom 겸용 파서, 기본 Geek News, 클릭 → 브라우저
- [~] 캘린더 ICS — 구현 완료, **비밀 iCal URL을 dcc.conf [calendar]에 붙여넣어야 표시됨**

## Phase 3 — 추적/개발자 패널 ✅

- [x] git 수집기 — repo 스캔(2단계), 브랜치/dirty/ahead, 통합 커밋 피드, 5분 주기
- [x] git 잔디 — 실제 26주 커밋 히트맵
- [x] claude 수집기 — 트랜스크립트 증분 파싱, 오늘/7d 토큰·비용, 14d 그래프, 세션 보드(Herd 스타일)
- [x] Windows 이벤트 로그 구독 — 앱 크래시·서비스 사망·디스크·재부팅·Defender (큐레이션, Sysmon 제외 확정)
- [ ] 신규 커밋/claude 세션 시작 → 토스트 (수집은 되나 diff 알림 배선 안 됨)
- [ ] repo 행 클릭 → VS Code/터미널/탐색기 (프로젝트 허브화)

## Phase 4 — 팔레트 & 클라우드

- [x] 전역 단축키 Ctrl+` → 팔레트 (포커스 예외 지점, Esc/포커스 이탈 시 닫힘)
- [x] Everything IPC 파일 검색 (미실행 시 안내)
- [~] 팔레트 명령 — 런처 필터 [x] · `+todo` 추가 [x] · 일정 추가 [ ](OAuth 필요) · 타이머 시작 [ ]
- [x] Todo 패널 — TODO.md 실파일, 클릭 토글, 외부 편집 감시
- [x] 드롭존 (IDropTarget) — Drive 미러 복사 / 확장자 정리 / 휴지통, 드래그 중 존 하이라이트
- [ ] Google OAuth (Calendar 쓰기 + Drive API) — 팔레트에서 일정 추가용. GCP OAuth 클라이언트 생성 필요(사용자 작업)

## 추적/관측 확장 (로드맵 외 추가 완료)

- [x] 4존 레이아웃 (좌 리소스 / 중 추적 / 우상 일상 / 우하 툴)
- [x] hot 5열 — CPU / MEM / IO/s / NET/s / CONN
- [x] 프로세스별 IO 전송률 (커널 카운터) + 개별 CPU 스파크라인
- [x] 프로세스별 네트워크 바이트 — ETW Kernel-Network (관리자, 비승격 시 conn 폴백)
- [x] 멀티메트릭 스파이크 로그 — cpu/io/net/gpu/lat/mem, 히스테리시스 + 지속조건(mem 4s, gpu 2s)
- [x] USE 패널 — run queue, hard faults/s, commit%, net err/s, sys io r/w, ICMP 레이턴시 그래프
- [x] us/sy/dpc 분리 + 인터럽트/s (vmstat 스타일)
- [x] load average 합성 (1/5/15분 EMA)
- [x] 타임라인 2h (30s 해상도 cpu/gpu)
- [x] 임계 경보 → 토스트 (run queue, hard faults, 핑 3연속 타임아웃)

## 운영/품질 (추가 완료)

- [x] 관리자 권한 스케줄러 자동시작 — `dcc --install / --uninstall / --stop` (실행 시간 제한 없음, 실패 시 재시작)
- [x] OLED 보호 — 픽셀 시프트(5분 궤도) + 야간 감광(23-08, dcc.conf)
- [x] soft 저채도 테마 옵션 (`theme = soft`, 라이브 전환)
- [x] git 저장소 초기화 + 커밋 관리 시작
- [x] 하루 로그(`logs\dcc-day-*.log`) + 오늘의 특이점 요약 패널 (재시작 시 리플레이)
- [x] 설정·로그를 `%LOCALAPPDATA%\dcc`로 분리 (구버전 파일 자동 이전)
- [x] 팔레트 내장 명령: config / todo / log / data
- [x] `--shot --demo` 예시 데이터 렌더 (README 스크린샷)
- [x] GitHub 공개 (README 영/한, MIT)

## 남은 것 / 다음 후보

- [ ] 리디자인 방향 선택 후 렌더러 이식 (시안 A~E)
- [ ] Google OAuth (일정 추가·Drive API 직접 업로드)
- [ ] 커밋/세션 diff 토스트 + repo 행 클릭 액션
- [ ] 디스크별 큐/busy (PDH PhysicalDisk) — sar -d
- [ ] 메모리 세분화 (standby/cache, si/so 분리)
- [ ] 팔레트 한글 IME 조합 개선
- 보류 후보: 치지직 라이브 알림, Now Playing 미디어 컨트롤, 프로세스 kill(2단계 확인)
- 제외 확정: 오디오 스펙트럼, CRT/노이즈/글리치, 부팅 시퀀스, 타이핑 터미널, Sysmon
