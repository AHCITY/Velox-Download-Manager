# PROTOCOL COMPATIBILITY — HTTP/HTTPS behavior contract

Normative rules distilled from RFC 9110/9112/6585/7617/6265/6266/8187 (primary texts read on 2026-11-19;
section citations inline). "MUST/MAY" here = our engine's own contract.

## 1. Resource inspection procedure

1. Send `GET` with `Range: bytes=0-0` (never assume HEAD works — many servers mishandle it; a ranged GET also
   probes range support *and* works on HEAD-broken servers). Redirects followed manually (§3) with the full chain
   recorded. `If-Range` is attached on resume-probes when a strong validator exists (RECOVERY_DESIGN §5).
2. `206` → ranges supported: complete length from `Content-Range` `bytes s-e/total`; `Accept-Ranges` header is
   advisory only (RFC 9110 §14.3 — a client "MUST NOT assume that receiving it means future range requests will
   return 206"); the real probe decides.
3. `200` → ranges unsupported/ignored (RFC 9110 §14.2: a server MAY ignore Range). Mode = single-stream
   (resumable only if the server accepted a mid-file range on a later probe — we re-probe on resume).
   The unread body is dropped immediately (connection closed, never reused — RFC 9112 §9.3: a client must read
   the entire body to reuse a persistent connection).
4. `416` → unsatisfiable; if `Content-Range: bytes */N` present, adopt `N` as current length and re-evaluate
   (completion check / restart decision). Clients cannot depend on ever getting 416 (§15.5.17).
5. `401` → authentication required; `WWW-Authenticate` challenge reported; Basic per RFC 7617 if credentials are
   configured, else `Permanent(AuthRequired)` error with guidance. `407` handled for proxies.
6. `405`/`501`/`429`/`5xx` on the probe → classified (§5); inspection retried per policy.
7. `Content-Encoding` != identity → force single-stream mode and record it: ranges apply to the *encoded*
   representation (RFC 9110 §14.1.2), so mixing decoded offsets with encoded ranges would corrupt files.
   We never enable transparent auto-decompression for downloads (reqwest strips Content-Length/Content-Encoding
   when decoding — verified from ClientBuilder docs); the bytes we store are the bytes the server sent.
8. Metadata captured: final URL, redirect chain, status, complete length, range support, ETag (strong/weak
   classified per §8.8.3), Last-Modified, Content-Disposition (raw), MIME, Content-Encoding,
   WWW-Authenticate, selected headers. Filename resolution order: config `filename` → `filename*` (RFC 8187
   decode) → `filename` (quoted-string, tolerate sloppy variants per RFC 6266 §4.3/App C) → URL last path
   segment (percent-decoded) → MIME-derived extension guess → `download-<id>`.

## 2. Supported download modes (compatibility matrix)

| Mode | Trigger | Resume | Notes |
|---|---|---|---|
| M1 segmented ranges | probe `206` + known length ≥ 2×min_segment | Yes, per segment | If-Range guarded; dynamic tail-splitting |
| M2 single-stream, known length | ranges unsupported but length known | Yes via `Range: bytes=N-` + If-Range if a probe confirms ranges; else restart | |
| M3 unknown length (chunked or close-delimited) | no usable length | No (restart) | close-delimited completeness is unprovable (RFC 9112 §6.3 note) — recorded as such |
| M4 non-resumable streaming | server rejects mid-file ranges at resume time | No | explicit reason in status |
| M5 conditionally resumable | validators existed originally, absent/changed now | per policy (§5.3) | |

The engine reports, per download: `resumable: Yes / Conditional(reason) / No(reason) / Unknown`.
It never claims universal resumability.

## 3. Redirects (RFC 9110 §15.4)

- Handled **manually** (client policy `none`): full chain recorded, loop detection via seen-URL set,
  max 10 hops (config). 301/302/303/307/308 all keep GET (downloader semantics; method preservation rules
  matter only for non-GET).
- `Range` and `If-Range` are preserved across hops (§15.4.9 semantics; §15.4 requires re-evaluating
  connection/proxy-specific fields).
- `Authorization` is re-applied only while the host matches the original request's host (§15.4: strip
  origin-specific/security-sensitive fields on redirect); cookies re-evaluated per RFC 6265 §5.4 rules by the
  cookie store.
- 304 on a conditional probe = resource unchanged → refresh validators, keep partial data (§15.4.5).

## 4. Framing (RFC 9112 §6)

- Body length precedence implemented as specified: HEAD/1xx/204/304 no body; TE-with-chunked framing; valid CL
  framing; else close-delimited. CL+TE together = fatal framing fault for that connection.
- Numeric parsing (Content-Length, chunk sizes, Content-Range numbers) is u64-checked — "recipients MUST
  anticipate potentially large decimal numerals".
- Chunked and close-delimited responses are tracked with explicit `incomplete` semantics; a close-delimited
  transfer is only "complete" if the connection closed cleanly after the request completed — and it is never
  hash-verifiable-complete unless a checksum was supplied out-of-band.

## 5. Error classification & retry policy

| Class | Examples | Action |
|---|---|---|
| Permanent | 404, 410, 403 after auth, 402, TLS certificate failure, malformed framing, ENOSPC, EACCES, filename invalid | fail (pause w/ reason for disk/permission); never retried |
| AuthRequired | 401/407 without credentials | fail with guidance; retried once with credentials when configured |
| Temporary | connect/timeout/reset errors, 408, 425?, 429, 500, 502, 503, 504, stalls, mid-body disconnects, DNS failure | retry segment from its checkpoint: exponential backoff + jitter (base 1 s, ×2, cap 60 s, default 5 attempts); `Retry-After` delta-seconds honored and takes precedence (RFC 9110 §10.2.3; 429 defined by RFC 6585). HTTP-date form (IMF-fixdate) parsed since session 2; obsolete date forms → backoff (documented deviation); server-provided delays clamped to RetryPolicy.max_delay (audit A5) |
| RangeMismatch | 200-on-ranged-request, wrong Content-Range start, 416 anomalies, validator change | restart-from-zero decision tree (RECOVERY_DESIGN §5.4) — never splice bytes across versions (RFC 9110 §15.3.7.3 requires the same strong validator for combining partials) |

Whole-download retry loops are bounded and observable; infinite retry is impossible (policy caps + permanent class).

## 6. Connection etiquette (RFC 9112 §9.4)

Per-host connection cap (default 8, configurable) enforced by semaphore; segment count adapts to file size
(ADR-0006); aborted bodies close their connection (never reused mid-body — §9.3). One range per request:
we never send multi-range requests, so multipart/byteranges (§14.6) is never required of us.

## 7. Cookies (RFC 6265)

Managed by the client's cookie store (session-scoped in MVP): stored per host/path, `Secure` respected,
Max-Age precedence implemented upstream. Explicit cookie headers from user config override. Persistent cookie
jar file + RFC 6265 §6.1 limits are planned (ROADMAP). Downloader-specific rule: host-only cookies never travel
to mirror hosts (§5.1.3) — mirrors must re-authenticate.

## 8. Known compatibility limitations (declared, not hidden)

- HTTP/2 via ALPN when the server offers it; HTTP/3 not implemented (upstream reqwest support is unstable —
  research report A2).
- No FTP/SFTP/BitTorrent/Metalink yet (ROADMAP P6/P8); Metalink design notes captured in RESEARCH §2/§5.
- No SOCKS4; SOCKS5 supported via reqwest `socks` feature. HTTP(S) proxies configured explicitly
  (system-proxy auto-detection disabled to keep sandbox/test behavior deterministic).
- NTFS/Android behaviors: engine rules are filesystem-agnostic; platform adapters later (RECOVERY_DESIGN §7).
