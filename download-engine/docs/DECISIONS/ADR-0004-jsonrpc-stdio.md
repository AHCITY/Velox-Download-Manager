# ADR-0004 — JSON-RPC 2.0 over stdio as API v1

Status: Accepted (2026-11-19)

## Context
Future clients (desktop, Android, extension, web) need a versioned local contract. Options: gRPC (heavy),
raw REST (ABDM-style), JSON-RPC (aria2 precedent, Motrix MDXP precedent).

## Decision
JSON-RPC 2.0, line-delimited, over **stdio** for MVP (`velox serve`); methods cover the full lifecycle
(info/create/start/pause/resume/cancel/remove/list/status/verify) — not the 3-endpoint minimalism ABDM shipped.
`engine.info` returns `api_version: "1"` for negotiation. Loopback HTTP transport + token auth is a follow-up
under the SECURITY §4 rules (opt-in, 127.0.0.1, token file 0600).

## Consequences
+ Transport-free engine: the CLI uses the core directly; `serve` proves the same API works out-of-process.
+ JSON-RPC error codes map cleanly onto our error classes.
− stdio only in MVP; no concurrent-clients story yet (single process owns the DB; enforced by SQLite locks).
