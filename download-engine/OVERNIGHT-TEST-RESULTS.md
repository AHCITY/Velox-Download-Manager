# Overnight test results

Final gate: `cargo test --workspace --no-fail-fast` — **17 test binaries, 74 tests, 0 failures**
(after rustfmt + clippy --fix; raw log: benchmarks/full-test-run.log).

| Suite | Count | Notes |
|---|---|---|
| integration (engine-core) | 18 | + finalize-fault ×2, adoption ×1, restart-zero unification |
| http_compat | 13 | + A7 changed-ETag, H3 length-change restarts |
| crash_resume (SIGKILL, out-of-process) | 4 | **hash assertions now genuinely execute** |
| segments_property | 2 | no overlap/gap, exact partition |
| engine-core lib | 23 | + retry clamp, limiter slicing, lock steal, HTTP-date tests |
| engine-state / storage / http / testserver / api libs | 3/4/3/2/2 | storage lock tests included |

Commands: cargo test --workspace --no-fail-fast; cargo clippy --workspace --all-targets (clean after
mechanical --fix); cargo fmt --all -- --check (clean). Benchmarks: run_local.sh (64 MiB loopback:
velox 0.04 s vs curl 0.08 s vs wget 0.06 s) and run_scenarios.sh (moderate 32 MiB @4 MB/s/conn:
velox 4.2 s vs curl/wget 8.5 s = 2.0×; raw logs in benchmarks/).

## Follow-up session (2026-11-20): 80 tests, 0 failures
+ plan override (min_segment), mid-download 503 retry, admission-control queueing,
  API inspect/notification/id/line-cap tests. Gates: fmt clean, clippy 11 accepted stylistic.
