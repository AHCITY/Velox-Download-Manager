# API v2 — event delivery and transport design

Status: **implemented** (transport + SSE), design rationale preserved below.
v1 (stdio JSON-RPC) is unchanged and remains the reference wire format.

## 1. Options considered

| Option | Complexity | Reconnect | Ordering | Notes |
| --- | --- | --- | --- | --- |
| A. JSON-RPC notifications on stdio (extend v1) | very low | n/a | good | couples UI to child-process lifetime; no multi-client |
| B. HTTP POST `/rpc` + **SSE** `/events` | low | **Last-Event-ID replay** | per-connection, sequence-tagged | browser-native (EventSource); desktop app = a tiny HTTP server; no schema change |
| C. WebSocket, length-delimited JSON | medium | manual resume protocol | good | needs a framing protocol + reconnect state machine by hand |
| D. gRPC / tonic | high | built-in | per-stream | heavy dependency, proto toolchain, overkill for a local single-user app |

**Decision: option B.** Rationale:

- A desktop UI is a browser context (webview) — `EventSource` gives auto-reconnect,
  `Last-Event-ID` replay and backpressure handling from the platform instead of hand-rolled
  framing (option C duplicates that work for no gain at this scale).
- The request/response side stays **plain JSON-RPC 2.0 over one `POST /rpc`** — the exact same
  `handle_line` implementation as v1, so method semantics can never drift between v1 and v2.
- No new dependencies: the transport is ~200 lines of tokio TCP (the engine already ships a
  minimal HTTP server for its test harness; the same HTTP/1.1 subset suffices).

## 2. Wire protocol

### 2.1 Requests — `POST /rpc`

Request body = one JSON-RPC 2.0 request (same shape as the v1 stdio line):

```bash
curl -s http://127.0.0.1:3000/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"download.create","params":{"url":"http://127.0.0.1:8099/file"}}'
```

Response = the v1 response JSON (`id` echoed, `result` or `error`).

Versioning: the transport adds a `X-Velox-Api: 2` response header; `engine.info` keeps
`"api_version": "1"` because the METHOD surface is identical to v1 (same `handle_line`) —
clients detect the transport by the header, not by divergent method semantics. Method names, params and results are
byte-identical to v1 — a UI written for v2 works against future stdio/IPC transports.

### 2.2 Events — `GET /events` (Server-Sent Events)

```
GET /events
Last-Event-ID: 41        (optional; browser sends it automatically on reconnect)

HTTP/1.1 200 OK
Content-Type: text/event-stream
Cache-Control: no-cache

id: 42
event: progress
data: {"id":7,"received":1048576,"total":4194304,"bps":524288}

id: 43
event: completed
data: {"id":7,"path":"/out/file.bin","size":4194304,"sha256":"…"}
```

- **Event types** mirror the engine's `Event` enum: `queued`, `started`, `progress`, `paused`,
  `resumed`, `retrying`, `completed`, `failed`, `cancelled`, plus `hello` (sent immediately with
  the current event id — a fresh client uses it as the starting point).
- **Ordering**: events carry a monotonically increasing `id`; the stream is generated from the
  engine's broadcast bus in arrival order (the engine's own sequencing).
- **Reconnect**: the server keeps a bounded replay ring (512 events). On connect with
  `Last-Event-ID`, it replays everything after that id; if the client is older than the ring
  start, it sends a `resync` event telling the UI to re-poll `download.list` (cheap, and the
  only correct answer when history is gone).
- **Slow clients**: a lagging subscriber that cannot drain the ring in time is **dropped, never
  blocking** — the engine's broadcast bus is already lossy-by-lag; the bridge adds a bounded
  channel (64) per connection and disconnects on overflow (the browser reconnects and either
  replays or resyncs).
- **Progress flooding**: `progress` events are coalesced per download to ≤4 Hz (≥250 ms spacing)
  with the *latest* values; terminal events (`completed`/`failed`/`cancelled`) are never
  coalesced or dropped.

### 2.3 Static UI

`GET /` (and any relative path) serves files from `--ui-dir`, content types for the common
set. Path traversal is rejected (`..` and absolute escapes never escape the ui root).

## 3. Security posture

- Binds `127.0.0.1` by default (`--bind` to override). A local desktop app needs no auth for
  loopback-only sockets; this is documented and explicit.
- No cookies, no CORS headers: the UI is served from the same origin it calls — cross-origin
  browser clients are refused by the default (no `Access-Control-Allow-Origin` header).
- Event payloads carry download metadata only; cookie/auth configuration never appears.

## 4. What was deliberately NOT done

- No write-ahead event log: the 512-event ring + `resync` re-poll covers a desktop UI's needs;
  a durable event log would add a second source of truth next to the manifest DB.
- No per-client filtering/subscription: a desktop UI wants everything; filters would be dead
  complexity today.
