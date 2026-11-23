#!/usr/bin/env bash
# Run the same gates as .github/workflows/ci.yml (linux job) locally.
#
# Usage: scripts/ci-local.sh [gates...] [--no-run]
#   gates (default: all):  fmt  clippy  test  js
#   --no-run  build the test binaries without executing them (warm-up)
#
# Each gate's full output goes to download-engine/target/audit-logs/<gate>.log
# (git-ignored via /target). Exit code is non-zero if any gate fails.
set -uo pipefail
PROJ=/home/z/my-project
# shellcheck source=scripts/rust-env.sh
source "$PROJ/scripts/rust-env.sh"
LOG="$PROJ/download-engine/target/audit-logs"
mkdir -p "$LOG"
cd "$PROJ/download-engine" || exit 2
export CARGO_INCREMENTAL=0   # incremental caches bloat the small sandbox disk

GATES=()
NO_RUN=()
for arg in "$@"; do
  case "$arg" in
    --no-run) NO_RUN=(--no-run) ;;
    fmt|clippy|test|js) GATES+=("$arg") ;;
    *) echo "unknown gate '$arg' (fmt|clippy|test|js)"; exit 64 ;;
  esac
done
[ ${#GATES[@]} -gt 0 ] || GATES=(fmt clippy test js)

FAILED=()
step() {  # step <name> <command...>
  local name="$1"; shift
  local start=$SECONDS rc=0
  "$@" > "$LOG/$name.log" 2>&1 || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "PASS  $name  ($((SECONDS - start))s)"
  else
    echo "FAIL  $name  rc=$rc  ($((SECONDS - start))s)  -> $LOG/$name.log"
    tail -25 "$LOG/$name.log" | sed 's/^/      /'
    FAILED+=("$name")
  fi
  return 0
}

for g in "${GATES[@]}"; do
  case "$g" in
    fmt)    step fmt    cargo fmt --all -- --check ;;
    clippy) step clippy cargo clippy --workspace --all-targets -- -D warnings ;;
    test)   step test   cargo test --workspace --no-fail-fast "${NO_RUN[@]}" ;;
    js)     ( cd web && step js node --test ) ;;  # Node >= 22 rejects a directory argument
  esac
done

if [ ${#FAILED[@]} -gt 0 ]; then
  echo "== FAILED: ${FAILED[*]}"
  exit 1
fi
echo "== all gates green"
