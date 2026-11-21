# ADR-0007 — Durability ladder: fsync-before-commit is law

Status: Accepted (2026-11-19)

## Context
"A successful DB transaction does not guarantee file data reached disk" (directive §2D). Research verified:
fsync(2) alone may not persist directory entries; WAL+FULL commits are power-durable; write() may short-write;
dropping a File ignores close errors.

## Decision
- Checkpoint = `File::sync_data()` **then** one `BEGIN IMMEDIATE` transaction (segments pos + aggregate).
- Segment completion = same, with pos == end.
- Download completion = `sync_all` → `rename(temp→final)` → directory fsync (Unix) → `state='completed'` commit.
  Invariant: `state=completed` ⇒ final file durable.
- Writes use `write_all_at` (loop-safe); part file opened with user-only perms; `set_len` sparse preallocation
  (fallocate KEEP_SIZE is a future platform optimization; SetFileValidData rejected for its privilege/exposure
  caveats).

## Consequences
+ Every crash window maps to a defined recovery (RECOVERY_DESIGN §6); no "DB said done but disk lied" state.
+ Cost is bounded: one fsync per checkpoint_interval (4 MiB default) per worker, measured later (P5 target <2%).
− fsync frequency is configurable downward at documented integrity cost; default keeps the guarantee.
