# ADR-0006 — Segmentation: few start, tail-stealing, hard floors

Status: Accepted (2026-11-19)

## Context
Blind 16/32/64-connection splitters hurt small files, impolitely hammer servers, and gain nothing when the
link is the bottleneck. IDM-style dynamic segmentation (validated by TurboDM/Mister DM behavior) is the
state of the art for open implementations.

## Decision
- Initial segments: `k0 = min(max_connections, ceil(total / (2 × min_segment_size)))`, each `total/k0`.
- A worker that finds the queue empty **steals the tail** of the largest active segment when
  `remaining ≥ 2 × min_segment_size` (clamp owner's end; owner clamps every write under the same mutex —
  no double-writes are possible by construction).
- Small files (< 2 × min_segment) and range-unsupported servers fall back to single-stream automatically.
- Per-host connection cap enforced by semaphore regardless of segment count.
- Stalls handled by read timeout + per-segment retry from checkpoint; deliberately *no* speculative
  preemption of "slow" connections in MVP (measured stealing-on-completion first; premature cancel/reopen
  churn is a real cost aria2 users know).

## Consequences
+ Small files get 1 connection; big files converge to max_connections; slow tails are reclaimed, not wasted.
+ Politeness and correctness bounds are structural (floors, caps), not behavioral heuristics.
− MVP lacks throughput-aware preemption and per-server learned pacing (ROADMAP next-steps).
