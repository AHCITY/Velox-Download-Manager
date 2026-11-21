# ADR-0003 — SQLite (rusqlite bundled) as the resume manifest; WAL + synchronous=FULL

Status: Accepted (2026-11-19)

## Context
The manifest must answer "which bytes are durably present?" atomically across downloads, segments, and error
history, and survive process kill and power loss. Alternatives: JSON sidecar (aria2-style control file), sled/
redb, sqlx.

## Decision
rusqlite 0.43 with `bundled` (SQLite amalgamation compiled in; public domain — zero license exposure).
`journal_mode=WAL` (persistent) + `synchronous=FULL` on every connection (per-connection pragma; the only
combination the pragma docs call durable in WAL mode) + `busy_timeout=5000` + `BEGIN IMMEDIATE` write
transactions. Single `Mutex<Connection>` (ops are µs-ms; WAL keeps readers unblocked). Schema-versioned via a
`meta` table; migrations run on open.

## Consequences
+ Transactional multi-table updates (segments+aggregate+error in one commit); queryable status; no bespoke
  binary format to version.
+ Power-loss semantics precisely match our fsync-before-commit checkpoint design (RECOVERY_DESIGN §3).
− Bundled C build needs cc at build time (present: gcc 14.2).
− DB is not network-fs safe (WAL) — acceptable: manifests live on local disk by definition.
