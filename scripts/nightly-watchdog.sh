#!/bin/bash
# Velox nightly guard: deterministic 30-minute cycles (no cron service in this runtime).
# Each cycle: fmt --check, clippy -D warnings, cargo test --workspace.
# Appends one line per cycle to download-engine/PROGRESS-LOG.md; archives failures
# to /home/z/my-project/backup/watchdog-failures/ for the morning.
# Max runtime 14 h, then exits. Safe to relaunch any time.
set -u
source /home/z/my-project/scripts/rust-env.sh
LOG=/home/z/my-project/download-engine/PROGRESS-LOG.md
FAILDIR=/home/z/my-project/backup/watchdog-failures
DEADLINE=$(( $(date +%s) + 14 * 3600 ))
mkdir -p "$FAILDIR"

while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  ts=$(date "+%Y-%m-%d %H:%M")
  cd /home/z/my-project/download-engine || { echo "$ts watchdog: engine dir missing" >> "$LOG"; exit 1; }
  # disk guard: skip heavy gates when free space is critically low
  FREE_KB=$(df -k / | awk 'NR==2 {print $4}')
  if [ "$FREE_KB" -lt 2000000 ]; then
    echo "- **$ts [watchdog]** SKIPPED gate (only ${FREE_KB}KB free disk)" >> "$LOG"
    sleep 1800
    continue
  fi
  if cargo fmt --all -- --check >/dev/null 2>&1 \
     && cargo clippy --workspace --all-targets -- -D warnings >/tmp/wd-clippy.log 2>&1 \
     && cargo test --workspace >/tmp/wd-tests.log 2>&1; then
    P=$(grep -oE '[0-9]+ passed' /tmp/wd-tests.log | awk '{s+=$1} END {print s+0}')
    echo "- **$ts [watchdog]** GREEN: fmt+clippy clean, $P tests passed" >> "$LOG"
  else
    STAMP=$(date +%Y%m%d-%H%M%S)
    cp /tmp/wd-clippy.log /tmp/wd-tests.log "$FAILDIR/" 2>/dev/null
    echo "- **$ts [watchdog]** FAILED — logs archived: $FAILDIR" >> "$LOG"
  fi
  sleep 1800
done
echo "- watchdog: 14 h budget exhausted, exiting" >> "$LOG"
