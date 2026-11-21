# SECURITY — threat model and rules

## 1. Threat model

Assets: local files (integrity + location), stored credentials/cookies, the local API surface, the process.
Adversaries: malicious/forged servers (headers, filenames, redirects, checksums), hostile URLs fed to the
engine (through API or CLI), local unprivileged processes reaching the API, tampering with state files.

## 2. Untrusted inputs — all validated

- **Filenames** from Content-Disposition/URL: sanitized (path separators (both `/` and `\`), `..`, control
  chars, Windows device names, trailing dots/spaces, leading `-`, colon; length capped ~240 UTF-8 bytes);
  final path is always inside the destination directory (traversal impossible by construction: name component
  only, joined onto a fixed dir).
- **Headers**: numeric fields parsed with checked u64; Content-Range validated (start must equal requested
  start, complete-length sane per RFC 9110 §14.4 — invalid values mean *do not recombine*).
- **Redirects**: bounded (10), loop-detected; Authorization stripped when host changes (RFC 9110 §15.4).
- **URLs**: parsed with the WHATWG `url` crate; scheme allow-list enforced at create (`http`, `https`; more via
  future protocol modules).

## 3. TLS

rustls with ring; certificate verification is **always on** and is never a compatibility fallback. A TLS
verification failure is classified Permanent; the only override is an explicit, per-download opt-in flag
(persisted in config_json and surfaced in status) — never silent, never global.

## 4. Local resource / SSRF discipline

The engine accepts arbitrary URLs by design (it is a downloader). Rules that keep this sane:
- No URL schemes beyond the allow-list (no `file:`, no Unix socket tricks via URL).
- Local API (future loopback transport) binds **127.0.0.1 only**, is **opt-in**, and requires a locally
  generated token (config file mode 0600) — the stdio transport inherits process privileges and needs none.
- A remote/web API is a separate future phase gated on the token mechanism + explicit CORS/origin policy;
  an unauthenticated download-control service will never be exposed.
- Loopback/link-local targets from *remote* API callers are refused unless explicitly allowed (future phase;
  recorded here as a design commitment).

## 5. Credentials & cookies

- Credentials (Basic/Bearer/proxy) live in `config_json` on disk with file mode 0600 on Unix; status output
  masks them. OS keyring integration planned (ROADMAP P7) before any UI ships credentials.
- Cookies: session store in MVP; persistent jar (planned) stored 0600, never logged.
- Nothing credential-bearing is written into error messages, events, or logs.

## 6. State files

`state.db` (SQLite) and part files: created with user-only modes on Unix (part file 0600 until finalization,
final file honors umask); DB integrity is SQLite's WAL domain (local disk only, per wal.html).

## 7. Supply chain

- Dependency set is deliberately small; licenses verified (RESEARCH §3): tokio/reqwest/rustls (MIT/Apache/ISC),
  rusqlite+SQLite (MIT + public domain), blake3 (CC0/Apache), sha2 (MIT/Apache), others MIT/Apache.
- `cargo audit`/dependabot-style review planned as part of CI (ROADMAP P5); no git deps; crates.io only.
- No code copied from copyleft projects (RESEARCH §3) — clean-room rules.

## 8. Robustness rules (binding)

Never disable TLS verification as a workaround; never downgrade a security decision to "make it work" without
an explicit user-visible flag; never expose an unauthenticated control service; never write outside the
destination directory; never execute downloaded content; treat every header/URL/manifest as hostile input.
