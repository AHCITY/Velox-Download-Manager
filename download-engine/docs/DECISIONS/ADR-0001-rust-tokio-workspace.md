# ADR-0001 — Rust + tokio; crates split by responsibility

Status: Accepted (2026-11-19)

## Context
Directive names Rust/tokio as the preferred stack and warns against empty speculative crates.
2-core sandbox, MSRV target 1.89+ (rustc 1.100 installed).

## Decision
Rust 2021 edition; tokio multi-thread runtime with the explicit feature set
`rt-multi-thread, macros, sync, time, io-util, fs, net, process, signal` (no `full`).
Workspace crates: engine-core, engine-http, engine-storage, engine-state, engine-api, engine-cli,
engine-testserver — each has distinct, currently-needed responsibilities. The directive's remaining
suggested crates (mirrors, auth, protocols, platform, observability) appear only with their features.

## Consequences
+ Compile/test surface stays small; boundaries are load-bearing, not decorative.
+ Cross-platform core (no OS-specific APIs in core; cfg'd storage shims only).
− More crates than a single-binary project; accepted because UI/engine independence is a hard requirement.
