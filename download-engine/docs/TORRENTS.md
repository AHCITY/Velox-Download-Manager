# BitTorrent support (magnet links + .torrent files)

Velox downloads torrents through a BitTorrent engine (librqbit 9.1, Apache-2.0)
that runs inside the app next to the HTTP engine. Torrent rows live in the SAME
download list as HTTP downloads — same states, same pause/cancel semantics, same
progress events. This document describes what works, what is intentional v1
scope, and where the state lives.

## What is supported

| Input | Example | Notes |
| --- | --- | --- |
| Magnet URI | `magnet:?xt=urn:btih:<40-hex>&dn=Name` | Metadata resolves via DHT + trackers |
| .torrent URL | `https://example.com/linuxmint-22.iso.torrent` | Fetched by the engine |
| .torrent file | `C:\Users\me\Downloads\big.iso.torrent` | Local path, read directly |

The Add dialog accepts all three forms. Torrent rows show the real name and
total size as soon as metadata resolves (magnets may take seconds to minutes
depending on swarm health — honest "downloading" while unknown-size).

## Behavior

- **Registration is paused.** `download.create` on a torrent source resolves
  metadata (name, size, file list) but starts NO traffic. The row appears in
  `created` state; pressing Resume (or auto-start) unpauses it.
- **Progress is mirrored into the ordinary row pipeline** (2 s poll →
  `progress` events + row updates), so the UI, activity feed and notifications
  work unchanged.
- **Completion** marks the row `completed` with the torrent's real name; files
  land under `<dest_dir>/<torrent name>/` (single-file torrents use the name as
  the file itself). Multi-file torrents create a folder — Remove with
  "delete file" removes the whole folder.
- **Pause/Cancel** reuse the HTTP engine's watch-channel machinery: pause stops
  the swarm, cancel also forgets the torrent from the session (files stay on
  disk unless "delete file" is chosen on remove).
- **Crash recovery**: the BitTorrent session persists (`fastresume` + a session
  dump under `<data_dir>/torrents-session`), so after an app restart torrents
  continue where they left off. Cancel-after-restart is idempotent (unknown
  torrent ids are silently tolerated).

## Engine layout

- New crate `crates/engine-torrent` implements `engine_core::ports::TorrentPort`
  (add/stats/pause/unpause/remove) — the same port seam as the ffmpeg merge
  stage. The transfer engine stays dependency-free of BitTorrent internals.
- Rows keep `mode = "torrent"` and carry the BitTorrent engine id in
  `config_json.torrent_id` so controls survive restarts.
- Download manager routing: `is_torrent_source()` classifies the source at
  `download.create` time; everything else stays HTTP.

## Dependency hygiene (why the build stays CI-friendly)

- librqbit is built with `default-features = false`: its HTTP stack unifies
  with the workspace's `reqwest` (rustls + **ring** provider, no aws-lc-sys —
  no cmake/NASM requirement on Windows CI).
- SHA-1 (BitTorrent's info-hash) goes through librqbit's `sha1w` wrapper with
  the `sha1-crypto-hash` backend: OS crypto APIs (CryptoAPI on Windows,
  OpenSSL on Linux, CommonCrypto on macOS) — no C toolchain additions.
- librqbit's internal reqwest client needs a process-level crypto provider;
  `TorrentService` installs the same ring provider the HTTP engine uses.

## Intentional v1 limits

- No per-file selection inside a multi-file torrent yet (whole torrent
  downloads; `only_files` support is a follow-up).
- No seeding after completion (uploads are stopped when the supervisor exits;
  seeding policy is a future setting).
- No bandwidth cap specific to torrents (the global per-download cap applies to
  HTTP; torrent ratelimits default to librqbit's).
