# BitTorrent & magnet downloads

Velox downloads BitTorrent swarms through the same list, queue and UI as every
other download: paste a `magnet:` link or pick a `.torrent` file in the
Add-Download dialog and the row joins the ordinary manifest.

## How it works

- The torrent engine lives in `crates/engine-torrent` and wraps
  [librqbit](https://github.com/ikatson/rqbit) 9.x (Apache-2.0, pure Rust):
  DHT, HTTP/UDP trackers, uTP/TCP peers, magnet metadata resolution.
- Rows use `mode = "torrent"` in the SAME SQLite manifest as HTTP downloads.
  One list, one pause/resume/remove semantic, no second UI.
- The engine never links the torrent library directly: the host (desktop app,
  `velox` CLI) injects a `TorrentPort` implementation at startup, mirroring the
  media-merge injection.
- A 1 Hz poller syncs swarm state into the row (`received`, `total_size`,
  state) and emits the ordinary `progress`/`downloading`/`paused`/
  `completed`/`failed` events, so the UI updates live with zero
  torrent-specific code. `connections` on the wire is the live peer count.
- The torrent name is a placeholder (`torrent <info-hash prefix>`) until the
  swarm metadata resolves; it is replaced with the real name automatically.

## Lifecycle

| Action | What happens |
|---|---|
| Add magnet / .torrent | Row created (`created`), attached to the swarm, starts downloading |
| Pause | Swarm traffic stops; downloaded pieces stay on disk |
| Resume / Retry | The swarm resumes from the pieces on disk (no re-download) |
| Cancel | Same as pause, row marked cancelled (pieces kept for a later retry) |
| Remove from list | Torrent detached from the session; pieces stay |
| Delete files | Torrent detached AND its pieces are deleted |
| App restart | Incomplete torrent rows are re-attached at startup (crash recovery) |

## Files

Multi-file torrents download into a sub-folder named after the torrent (the
row's file name). Single-file torrents land directly in the destination folder.

## Networking notes

- Magnet links resolve via DHT and the trackers inside the magnet; `.torrent`
  files are read locally (also fetched from http(s) URLs when passed as URLs).
- No TLS inside the torrent client beyond the platform stack: librqbit uses
  the native TLS (OpenSSL on Linux, schannel on Windows) for https trackers
  and `.torrent` fetches — the same choice the Windows builds ship with.
- Outbound peer connections use the first free port (librqbit default); a
  reachable port improves swarm participation but is not required.

## Safety & scope

- No torrent creation (yet) — download only.
- No sequential/streaming mode; pieces are written through librqbit's storage
  and validated per-piece (SHA-1) before counting as received.
- The optional checksum field in the Add dialog applies to HTTP downloads
  only; torrent integrity is guaranteed by the protocol's piece hashes.
