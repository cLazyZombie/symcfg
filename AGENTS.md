# AGENTS.md

- 커밋 메시지는 한국어로 작성한다.
- 코드 주석은 한국어로 작성한다.
- 사용자에게 보이는 CLI 출력은 영어로 작성한다.
- 완료/커밋 전 `cargo clippy --all-targets --all-features -- -D warnings`가 경고 없이 통과해야 한다.
- 완료/커밋 전 `./scripts/coverage_100.sh`가 통과해야 한다.
- 커버리지는 `cargo llvm-cov --workspace --all-targets --all-features --lcov --quiet | lcov_filter --text`로 검사한다.
- `lcov_filter --text`는 `LCOV_EXCL_LINE`, `LCOV_EXCL_START`, `LCOV_EXCL_STOP` 마커를 반영한 뒤 누락 라인이 있으면 실패한다.
- OS 오류, 복구 불가능한 race, 테스트 가치가 낮은 방어 분기만 `LCOV_EXCL_*` 마커로 제외한다. 정상 동작과 의미 있는 오류 동작은 테스트로 덮는다.
