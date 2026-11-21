# ADR-0002 — reqwest 0.13 + rustls with the ring provider; manual redirects

Status: Accepted (2026-11-19)

## Context
Choice among hyper-direct, reqwest, and raw sockets. reqwest gives pooled connections, cookie store,
proxies (socks), and HTTP/2 — hyper-direct would re-implement all of that. Constraint: no cmake in this
environment; reqwest 0.13's default `rustls` feature pulls aws-lc-rs (build risk), and its 0.13 feature set
renamed the 0.12 TLS flags (verified from Cargo.toml source). Downloads are opaque binary: transparent
decompression would strip Content-Length/Content-Encoding and break range math (RFC 9110 §14.1.2).

## Decision
- `reqwest` with `default-features = false`, features `rustls-no-provider`, `cookies`, `socks`;
  direct dep `rustls = { default-features = false, features = ["ring", "std", "tls12"] }` — with only the
  `ring` feature enabled, rustls auto-selects ring (no runtime `install_default`, no aws-lc-rs).
- Decompression features **not** enabled; we store exactly the bytes the server sends.
- Redirects handled **manually** (client policy `none`): full chain capture, loop detection, host-scoped
  Authorization re-application, Range/If-Range preservation — impossible with the client's built-in policy
  (which cannot report per-request chains).
- Timeouts: `timeout(None)`, `connect_timeout` + `read_timeout` for stall detection; per-chunk watchdog too.
- Trait boundary: `engine-core::ports::HttpPort` (`async_trait`), so tests can mock the transport.

## Consequences
+ No cmake/NASM build dependency; smaller binary; deterministic behavior in tests.
+ Full control of redirect semantics (RFC 9110 §15.4).
− Manual redirect loop is our code to maintain (loop detection, per-hop rules).
− No HTTP/3 (upstream unstable anyway); HTTP/2 free via ALPN.
