# STATE.md — Velox Download Manager mission state

Last updated: 2026-11-22 (after cycle 7 session). Keep this file current; commit before ending every turn.

## Where things are

| Path | Purpose |
|---|---|
| `download-engine/` | Rust workspace: engine-core / engine-http / engine-storage / engine-state / engine-api / engine-cli / engine-testserver + `app/src-tauri` (Tauri shell) |
| `docs/AUTONOMOUS_CYCLE_INITIAL_AUDIT.md` | 3-agent initial audit (source of the task queue) |
| `docs/AUTONOMOUS_CYCLE_TASK_QUEUE.md` | Task queue with DONE markers |
| `docs/AUTONOMOUS_CYCLE_PROGRESS.md` | Per-cycle progress log |
| `scripts/installer-online-template.nsi` | NSIS installer template (online bootstrapper variant; PRODUCT_VERSION tracks velox-app crate) |
| `scripts/build-velox-msvc.sh` | Windows cross-build (cargo-xwin, clang-19 debs in /tmp) |
| `scripts/github-release-refresh.sh <dir> <tag>` | Publish build dir to GitHub release + verify |
| `scripts/github-push.sh` | Push using token file; verifies remote == local |
| `scripts/rust-env.sh` | Toolchain env (CARGO_HOME/RUSTUP_HOME under my-project) |
| `scripts/TOOLCHAIN-SETUP.md` | Exact recipe to rebuild toolchain after a sandbox reset |
| `.secrets/github-token` | GitHub token (0600, gitignored) — recreate from chat if wiped |
| `download/windows-build-20261121-c/` | Current installer staging: Velox-Setup-x64.exe, Velox.exe, SHA256SUMS.txt, INSTALL-AND-VERIFY.md |
| `download/mission-reports/` | Copies of the three mission docs |

## Progress

- Cycles 1–5: complete (see progress log). Remote at cycle-5 pause: `c85e218`.
- Cycle 6 (complete, `9757163`): installer hygiene — truthful ONLINE-variant header, WebView2 `pv="0.0.0.0"` treated as missing, makensis-validated; WINDOWS_COMPAT.md reconciled to shipped code (LockFileEx locks, Windows errno table, FlushFileBuffers, CI green).
- Cycle 7 (complete, `9770253`): expired-URL re-resolve — 403 on a resolved URL clears `row.resolved_url` and re-probes the original page once per task run; per-iteration probe target; 2 new tests; workspace 180 passed / 0 failed.
- Installer refreshed from `9770253` and round-trip verified: SHA256 `404e13afd2dc73fc…` on release v0.1.0.
- Mission docs committed `251c131`. Housekeeping commit `163cb09` on top (remote push verified in this session's first check... push it if remote falls behind).

## Remaining queue (next sessions)

1. Long-path `\\?\` helper (WINDOWS_COMPAT plan item 5).
2. Plain-GET-206 tolerance + per-chunk HTML sniff (queue task 9).
3. Extractor cookie-jar reuse (queue task 7).
4. Bench-server 416 + fault flags (task 10); typed CLI errors (task 12); dead-state cleanup (task 13).

## Recovery after sandbox reset (checked once, then ask the user)

Things Git does NOT keep: `.secrets/github-token`, `.cargo/`, `.rustup/`, `.xwin-cache/`, `/tmp/*` (llvm deb extraction, nsisx makensis, installer staging). Rebuild per `scripts/TOOLCHAIN-SETUP.md` (rustup stable into my-project dirs + `rustup target add x86_64-pc-windows-msvc` + `cargo install cargo-xwin --version 0.23.1 --locked` + clang-19 debs + nsis 3.13 debs). Durable copies of every artifact live in GitHub release v0.1.0.
