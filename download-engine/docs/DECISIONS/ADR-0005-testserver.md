# ADR-0005 — Own std-only test server with fault injection

Status: Accepted (2026-11-19)

## Context
The test suite must simulate: missing/lying Content-Length, ignored ranges, wrong Content-Range, mid-body
closes, stalls, slow-drip connections, ETag changes, 429+Retry-After, redirects, chunked framing, unicode
Content-Disposition — and count connections. Existing crates (tiny_http, hyper server) don't expose per-
connection fault choreography cleanly.

## Decision
`engine-testserver`: std::net::TcpListener + thread-per-connection, minimal HTTP/1.1 parser, behavior
object shared via `Arc<Mutex<Behavior>>` that tests mutate mid-run (e.g., flip ETag between sessions).
Zero dependencies (std only), so it can be a dev-dependency anywhere without build cost. Doubles as the
benchmark origin server.

## Consequences
+ Exact fault injection at the TCP byte level; deterministic tests; no extra dependency tree.
− Our HTTP server is not RFC-exhaustive (it is a *test oracle*, deliberately quirky server behavior is the point).
