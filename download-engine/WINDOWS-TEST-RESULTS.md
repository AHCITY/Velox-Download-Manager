# WINDOWS-TEST-RESULTS

Date: 2026-11-20. Every result below is from a command actually executed in this sandbox.
Windows CI results are NOT fabricated — see the honest-status section at the end.

## Executed locally (Linux sandbox)

| Check | Command | Result |
| --- | --- | --- |
| Full workspace tests | `cargo test --workspace --no-fail-fast` | **100 passed, 0 failed** (was 94 at audit start; +4 Host/Origin validation tests, +1 pause-while-queued, +1 updated queue test) |
| Formatting | `cargo fmt --all -- --check` | clean |
| Clippy | `cargo clippy --workspace --all-targets` | pre-existing stylistic warnings only; none introduced by this milestone |
| Windows cross-check (storage) | `cargo check -p engine-storage --target x86_64-pc-windows-msvc` | **passes** |
| Full workspace Windows cross-check | `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` | **blocked locally**: `ring` build script needs clang-cl (not installed) — environment limit, documented |
| Host/Origin validation | `cargo test -p engine-bridge` | 10/10 incl. 4 new validation fns (lookalikes, malformed ports, duplicate Host, foreign/null Origin) |
| Queue correctness | `cargo test -p engine-core --test integration max_concurrent` | 1/1 — start() now returns immediately when the cap is reached; row becomes `queued` |
| Pause-while-queued | `cargo test -p engine-core --test integration pause_while_queued` | 1/1 — a queued download paused mid-queue never starts |
| Bridge tests (SSE/RPC/guards) | `cargo test -p engine-bridge` | 10/10 |

## Executed live (real stack, real downloads, headless Chromium)

| Scenario | Result |
| --- | --- |
| Add download through the UI, complete | 96 MiB completed; sha256 of file on disk **identical** to an independent curl of the source (`bbc3ab03…`) |
| Multiple simultaneous downloads | verified (two concurrent rows progressing independently) |
| Pause via UI | byte-frozen: `paused 147652608` → identical 3 s later |
| Resume via UI | progress continued (147.6 → 180.1 MiB), 8 connections |
| Cancel via UI | row removed, part file cleaned, completed row untouched |
| SIGKILL engine mid-download + restart | history persisted (`completed` row intact; interrupted row retained with the true part-file size 135.8 MB) |
| Resume after SIGKILL | **resumed from the part-file offset** (135.8 → 184 → 224 MB, 8 connections), lock claimed after crash, no restart from zero |
| Foreign Host header | refused **403** (live); duplicate-Host rejection covered by unit test (curl cannot send duplicate Host) |
| localhost (IPv6 ::1) navigation | **fixed** — bridge now binds both stacks; browser shows Connected |

## Windows CI (honest status)

**NOT EXECUTED.** No GitHub runner is reachable from this sandbox. The workflow
(`.github/workflows/windows.yml`) is upgraded and covers every required gate, but until it
runs green on `windows-latest`, Windows behavior is verified only by: local msvc
cross-compile of engine-storage, cfg(windows) code review, and the shipped CI definition.
No Windows test has been labeled passing.
