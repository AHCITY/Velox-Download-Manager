//! Velox API v2 transport (docs/API_V2.md): HTTP JSON-RPC + Server-Sent Events.
//!
//! - `POST /rpc`   — one JSON-RPC 2.0 request per body; **reuses engine_api::handle_line**,
//!   so v1 and v2 can never drift in method semantics.
//! - `GET /events` — SSE stream: sequence-tagged, bounded replay ring (512),
//!   Last-Event-ID reconnect, `resync` when history is gone, progress coalesced
//!   to ≤4 Hz, slow clients disconnected instead of blocking the engine.
//! - `GET /config.json` — small bootstrap object (sample URL for the demo UI).
//! - `GET /…`      — static files from `--ui-dir` (traversal-safe).

use engine_api::handle_line;
use engine_core::events::Event;
use engine_core::DownloadManager;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const RING_CAP: usize = 512;
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(250);
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// A sequenced wire event: `(id, event_name, data_json)`.
type Wire = (u64, String, String);

struct Hub {
    seq: AtomicU64,
    ring: Mutex<VecDeque<Wire>>,
    /// broadcast of already-sequenced events to live connections
    tx: tokio::sync::broadcast::Sender<Wire>,
}

impl Hub {
    fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            seq: AtomicU64::new(0),
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
            tx,
        }
    }

    /// Publish one wire event: assign the sequence id, append to the replay
    /// ring, fan out to live connections. Never blocks on slow consumers.
    fn publish(&self, event: &str, data: &str) -> u64 {
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let wire: Wire = (id, event.to_string(), data.to_string());
        let mut ring = self.ring.lock().unwrap();
        ring.push_back(wire.clone());
        while ring.len() > RING_CAP {
            ring.pop_front();
        }
        drop(ring);
        let _ = self.tx.send(wire); // lagging receivers surface as RecvError::Lagged
        id
    }
}

/// Map an engine event to the SSE wire form (docs/API_V2.md §2.2). `None` for
/// events a UI does not need (none today — every variant maps).
pub fn wire_event(e: &Event) -> (String, String) {
    match e {
        Event::DownloadState { id, state } => {
            // DB state names are already canonical for the UI
            (state.clone(), json!({ "id": id }).to_string())
        }
        Event::Progress {
            id,
            received,
            total,
            speed_bps,
            connections,
        } => (
            "progress".to_string(),
            json!({
                "id": id,
                "received": received,
                "total": total,
                "bps": speed_bps,
                "segments": connections,
            })
            .to_string(),
        ),
        Event::Restarting { id, reason } => (
            "retrying".to_string(),
            json!({ "id": id, "reason": reason }).to_string(),
        ),
        Event::Completed {
            id,
            path,
            size,
            sha256,
            blake3,
        } => (
            "completed".to_string(),
            json!({ "id": id, "path": path, "size": size, "sha256": sha256, "blake3": blake3 })
                .to_string(),
        ),
        Event::Failed { id, kind, message } => (
            "failed".to_string(),
            json!({ "id": id, "kind": kind, "message": message }).to_string(),
        ),
        Event::Verified { id, ok, algo } => (
            "verified".to_string(),
            json!({ "id": id, "ok": ok, "algo": algo }).to_string(),
        ),
        // audit D4: resolve-phase feedback (extraction/confirm-walk in progress;
        // emitted with the original URL — no download row exists yet)
        Event::Resolving { url } => ("resolving".to_string(), json!({ "url": url }).to_string()),
        Event::Resolved { url, note } => (
            "resolved".to_string(),
            json!({ "url": url, "note": note }).to_string(),
        ),
        // MediaPipeline (architecture review §9): merge phase events, ids per contract
        Event::Merging { id } => ("merging".to_string(), json!({ "id": id }).to_string()),
        Event::Merged { id, path } => (
            "merged".to_string(),
            json!({ "id": id, "path": path }).to_string(),
        ),
    }
}

/// The event pump: consumes the engine bus, coalesces progress (≤4 Hz, latest
/// value wins), publishes structural events immediately. A slow SSE client is
/// handled by the broadcast channel (lag → that connection resyncs/disconnects);
/// the pump itself never blocks the engine.
async fn pump(manager: Arc<DownloadManager>, hub: Arc<Hub>) {
    let mut rx = manager.events();
    let mut pending: HashMap<i64, String> = HashMap::new();
    let mut ticker = tokio::time::interval(PROGRESS_MIN_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(e) => {
                        if let Event::Progress { id, .. } = &e {
                            // coalesce: latest values only, flushed on the 250 ms tick
                            let (name, data) = wire_event(&e);
                            pending.insert(*id, format!("{name}\n{data}"));
                            continue;
                        }
                        // any non-Progress event (state change, completed, failed,
                        // retrying, verified) invalidates coalesced progress for that
                        // download FIRST — otherwise a pre-`completed` progress flushes
                        // AFTER the terminal event with a higher seq id and flips the
                        // row back to "downloading" (review finding)
                        match &e {
                            Event::Progress { .. } => unreachable!(),
                            Event::DownloadState { id, .. }
                            | Event::Restarting { id, .. }
                            | Event::Completed { id, .. }
                            | Event::Failed { id, .. }
                            | Event::Verified { id, .. }
                            | Event::Merging { id, .. }
                            | Event::Merged { id, .. } => {
                                pending.remove(id);
                            }
                            // audit D4: resolve-phase events carry no download id
                            // (no row exists yet) — publish immediately
                            Event::Resolving { .. } | Event::Resolved { .. } => {}
                        }
                        let (name, data) = wire_event(&e);
                        hub.publish(&name, &data);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // the pump itself lagged (engine outpaced it): force clients to resync
                        hub.publish("resync", "{}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            _ = ticker.tick() => {
                // flush latest progress per download at most every 250 ms
                for payload in pending.values() {
                    if let Some((name, data)) = payload.split_once('\n') {
                        hub.publish(name, data);
                    }
                }
                pending.clear();
            }
        }
    }
}

/// Serve API v2 on an already-bound listener. Returns only on error/stop.
pub async fn serve_http(
    manager: Arc<DownloadManager>,
    listeners: Vec<TcpListener>,
    ui_dir: Option<PathBuf>,
    sample_url: Option<String>,
) -> std::io::Result<()> {
    let hub = Arc::new(Hub::new());
    tokio::spawn(pump(manager.clone(), hub.clone()));
    // bound concurrent connections: a local desktop UI needs a handful, not thousands
    let conn_sem = Arc::new(tokio::sync::Semaphore::new(64));
    // one accept loop per listener (IPv4 + IPv6 so `localhost` resolves on either stack)
    let mut accept_loops = Vec::new();
    for listener in listeners {
        let manager = manager.clone();
        let hub = hub.clone();
        let conn_sem = conn_sem.clone();
        let ui_dir = ui_dir.clone();
        let sample_url = sample_url.clone();
        accept_loops.push(tokio::spawn(async move {
            loop {
                let (stream, _addr) = match listener.accept().await {
                    Ok(s) => s,
                    Err(e) => {
                        // transient accept errors (EMFILE, ECONNABORTED) must not kill the server
                        tracing::warn!(error = %e, "accept failed");
                        continue;
                    }
                };
                let sem = conn_sem.clone(); // keep the Arc alive: the permit borrows it
                let manager = manager.clone();
                let hub = hub.clone();
                let ui_dir = ui_dir.clone();
                let sample_url = sample_url.clone();
                tokio::spawn(async move {
                    // the permit must be acquired INSIDE the task: scoped to the accept
                    // loop it dropped at the end of the iteration, so the 64-connection
                    // cap was never actually enforced (review finding)
                    let _permit = match sem.try_acquire() {
                        Ok(p) => p,       // held for the connection's whole lifetime
                        Err(_) => return, // at capacity: shed the connection silently
                    };
                    let _ = connection(manager, hub, ui_dir, sample_url, stream).await;
                });
            }
        }));
    }
    for handle in accept_loops {
        let _ = handle.await;
    }
    Ok(())
}

async fn connection(
    manager: Arc<DownloadManager>,
    hub: Arc<Hub>,
    ui_dir: Option<PathBuf>,
    sample_url: Option<String>,
    mut stream: TcpStream,
) -> std::io::Result<()> {
    // bounded request phase: a slowloris socket cannot pin a task indefinitely
    let request =
        match tokio::time::timeout(Duration::from_secs(10), read_request(&mut stream)).await {
            Ok(Ok(Some(r))) => Some(r),
            _ => None, // io error, EOF, or timeout → 400/408 below
        };
    let Some((method, path, headers, body)) = request else {
        eprintln!("[bridge] read_request returned None (EOF/cap/framing)");
        return respond_simple(&mut stream, 400, "bad request").await;
    };
    eprintln!(
        "[bridge] {} {} ({} hdrs, {} byte body)",
        method,
        path,
        headers.len(),
        body.len()
    );
    // loopback-only service: strict Host/Origin validation — an attacker page's
    // "simple request" or a DNS-rebinding origin cannot drive the RPC. Deliberately
    // no CORS headers to relax; anything not exactly loopback is refused.
    if let Err(status) = validate_client_identity(&headers) {
        eprintln!("[bridge] identity check rejected with {status}");
        tracing::warn!(
            status,
            host = %header(&headers, "host").unwrap_or_default(),
            origin = %header(&headers, "origin").unwrap_or_default(),
            "request refused by loopback identity check"
        );
        return respond_simple(&mut stream, status, "local only").await;
    }
    match (method.as_str(), path.as_str()) {
        ("POST", "/rpc") => {
            // framing hardening: duplicate/conflicting Content-Length or any
            // Transfer-Encoding is rejected (no smuggling through a reverse proxy)
            let cls: Vec<&str> = headers
                .iter()
                .filter(|(k, _)| k == "content-length")
                .map(|(_, v)| v.as_str())
                .collect();
            if cls.len() > 1
                || cls.first().map(|v| v.parse::<usize>().ok()) == Some(None)
                || headers.iter().any(|(k, _)| k == "transfer-encoding")
            {
                return respond_simple(&mut stream, 400, "bad request framing").await;
            }
            let reply = handle_line(&manager, &body).await;
            let body = reply.unwrap_or_else(|| {
                // request without id = notification: no response body, 204
                String::new()
            });
            let status = if body.is_empty() { 204 } else { 200 };
            respond(
                &mut stream,
                status,
                "application/json",
                body.as_bytes(),
                &[("X-Velox-Api", "2"), ("Cache-Control", "no-store")],
            )
            .await
        }
        ("GET", "/events") => sse(&mut stream, hub, header(&headers, "last-event-id")).await,
        ("GET", "/config.json") => {
            let body = json!({ "sample_url": sample_url }).to_string();
            respond(&mut stream, 200, "application/json", body.as_bytes(), &[]).await
        }
        ("GET", p) if ui_dir.is_some() => {
            let root = ui_dir.unwrap();
            match static_file(&root, p) {
                Some((ctype, bytes)) => {
                    respond(
                        &mut stream,
                        200,
                        &ctype,
                        &bytes,
                        &[("Cache-Control", "no-store")],
                    )
                    .await
                }
                None => respond_simple(&mut stream, 404, "not found").await,
            }
        }
        _ => respond_simple(&mut stream, 404, "not found").await,
    }
}

/// Read one HTTP/1.1 request (request line, headers, Content-Length body).
/// Bounded: header block ≤ 8 KiB, body ≤ 1 MiB (untrusted input cannot grow
/// engine memory — same policy as the v1 line cap).
async fn read_request(
    stream: &mut TcpStream,
) -> std::io::Result<Option<(String, String, Vec<(String, String)>, String)>> {
    let mut head = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    // read until \r\n\r\n (bounded)
    loop {
        if head.len() > MAX_HEADER_BYTES {
            return Ok(None);
        }
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Ok(None); // EOF before a full request
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let content_length: usize = header(&headers, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Ok(None);
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).await?;
    }
    Ok(Some((
        method,
        path,
        headers,
        String::from_utf8_lossy(&body).to_string(),
    )))
}

/// Strict loopback identity check for the Host (and Origin, when present) header.
/// - host part must be EXACTLY `localhost`, `127.0.0.1` or `[::1]` (IPv6 requires
///   brackets per RFC 3986; a bare `::1` is malformed and rejected)
/// - port is optional; when present it must be valid (empty or 1..=65535 digits)
/// - rejects lookalikes: `localhost.attacker.example`, `127.0.0.1.attacker.example`,
///   `127.0.0.01` (non-canonical octets), `[::1]extra`, userinfo tricks (`a@localhost`)
/// - more than one Host header is a smuggling attempt: rejected
/// - a foreign `Origin` (browsers attach Origin to every cross-origin request, even
///   "simple" ones) is rejected; absent Origin (curl, same-origin UI) passes
fn validate_client_identity(headers: &[(String, String)]) -> Result<(), u16> {
    let hosts: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k == "host")
        .map(|(_, v)| v.as_str())
        .collect();
    if hosts.len() > 1 {
        return Err(400); // duplicate Host = request smuggling
    }
    let Some(host) = hosts.first() else {
        return Err(400); // HTTP/1.1 requires Host
    };
    let host_part = split_host_port(host).ok_or(400u16)?;
    if !is_loopback_host(host_part) {
        return Err(403);
    }
    // Origin (when present) must also be exactly loopback; `null` is refused
    if let Some(origin) = header(headers, "origin") {
        if origin == "null" {
            return Err(403);
        }
        let authority = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(origin.as_str());
        let authority = authority.split('/').next().unwrap_or("");
        let Some(o_host) = split_host_port(authority) else {
            return Err(403);
        };
        if !is_loopback_host(o_host) {
            return Err(403);
        }
    }
    Ok(())
}

/// Split `host[:port]`, handling bracketed IPv6; validates the port syntax.
/// Returns the host part. `None` = malformed value.
fn split_host_port(value: &str) -> Option<&str> {
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix('[') {
        // bracketed IPv6: `[::1]` or `[::1]:8080`; nothing else may follow `]`
        let (addr, after) = rest.split_once(']')?;
        match after.as_bytes().first() {
            None => Some(addr),
            Some(b':') => {
                validate_port(&after[1..])?;
                Some(addr)
            }
            _ => None,
        }
    } else {
        match value.split_once(':') {
            None => Some(value),
            Some((h, p)) => {
                validate_port(p)?;
                Some(h)
            }
        }
    }
}

/// Empty port (= default) or a number 1..=65535, no leading zeros beyond "0".
fn validate_port(p: &str) -> Option<()> {
    if p.is_empty() {
        return Some(());
    }
    let n: u32 = p.parse().ok()?;
    if (1..=65535).contains(&n) && !(p.len() > 1 && p.starts_with('0')) {
        Some(())
    } else {
        None
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.to_string())
}

fn static_file(root: &Path, path: &str) -> Option<(String, Vec<u8>)> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    // traversal guard: reject any path component that escapes the ui root
    if rel.split(['/', '\\']).any(|c| c == "..") {
        return None;
    }
    let full = root.join(rel);
    let canonical_ok = full
        .canonicalize()
        .map(|c| c.starts_with(root.canonicalize().unwrap_or_else(|_| root.to_path_buf())))
        .unwrap_or(false);
    if !canonical_ok {
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    let ctype = match full.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };
    Some((ctype.to_string(), bytes))
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        _ => "OK",
    };
    // RFC 9110 §8.6: a 204 MUST NOT carry Content-Length (and needs no Content-Type)
    let head = if status == 204 {
        let mut h = "HTTP/1.1 204 No Content\r\nConnection: close\r\n".to_string();
        for (k, v) in extra {
            h.push_str(&format!("{k}: {v}\r\n"));
        }
        h.push_str("\r\n");
        h
    } else {
        let mut h = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (k, v) in extra {
            h.push_str(&format!("{k}: {v}\r\n"));
        }
        h.push_str("\r\n");
        h
    };
    stream.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }
    stream.flush().await?;
    // Connection: close — shut the write side politely
    let _ = stream.shutdown().await;
    Ok(())
}

async fn respond_simple(stream: &mut TcpStream, status: u16, msg: &str) -> std::io::Result<()> {
    respond(
        stream,
        status,
        "text/plain; charset=utf-8",
        msg.as_bytes(),
        &[],
    )
    .await
}

/// SSE stream with replay. Lifecycle:
/// 1. optional replay of ring entries with id > Last-Event-ID;
/// 2. if the client is behind the ring start, send `resync` instead;
/// 3. live events; on broadcast lag → `resync` and close (client reconnects).
async fn sse(
    stream: &mut TcpStream,
    hub: Arc<Hub>,
    last_event_id: Option<String>,
) -> std::io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        Cache-Control: no-cache\r\n\
        X-Accel-Buffering: no\r\n\
        Connection: keep-alive\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await?;

    let seq_now = hub.seq.load(Ordering::SeqCst);
    // hello carries NO `id:` line — a browser adopting the hello id as its
    // Last-Event-ID would skip ring entries on the next reconnect (review finding)
    let hello = format!("event: hello\ndata: {}\n\n", json!({ "last_id": seq_now }));
    stream.write_all(hello.as_bytes()).await?;
    stream.flush().await?;

    // Order matters: subscribe to live events BEFORE snapshotting the ring, so no
    // event can fall into the gap between replay and live delivery; duplicates are
    // filtered by id below (review finding #1)
    let mut rx = hub.tx.subscribe();
    let mut last_sent = 0u64;

    // replay: everything after the client's last seen id, or resync if the client is
    // behind the ring start (or ahead of the sequence counter — stale server restart)
    let from = last_event_id.and_then(|v| v.parse::<u64>().ok());
    if let Some(from) = from {
        let seq_now2 = hub.seq.load(Ordering::SeqCst);
        if from > seq_now2 {
            stream.write_all(b"event: resync\ndata: {}\n\n").await?;
        } else {
            // ring guard must NOT live across an await (connection future must stay
            // Send): snapshot the slice, drop the guard, then write
            let replay: Vec<Wire> = {
                let ring = hub.ring.lock().unwrap();
                let oldest = ring.front().map(|(id, _, _)| *id).unwrap_or(seq_now2 + 1);
                if from.saturating_add(1) < oldest {
                    Vec::new()
                } else {
                    ring.iter()
                        .skip_while(|(id, _, _)| *id <= from)
                        .cloned()
                        .collect()
                }
            };
            if from.saturating_add(1)
                < hub
                    .ring
                    .lock()
                    .unwrap()
                    .front()
                    .map(|(id, _, _)| *id)
                    .unwrap_or(seq_now2 + 1)
            {
                stream.write_all(b"event: resync\ndata: {}\n\n").await?;
            } else {
                for (id, name, data) in &replay {
                    stream
                        .write_all(format!("id: {id}\nevent: {name}\ndata: {data}\n\n").as_bytes())
                        .await?;
                }
                if let Some((id, _, _)) = replay.last() {
                    last_sent = *id;
                }
            }
        }
        stream.flush().await?;
    }

    loop {
        match rx.recv().await {
            Ok((id, name, data)) => {
                if id <= last_sent {
                    continue; // already delivered by the replay
                }
                last_sent = id;
                stream
                    .write_all(format!("id: {id}\nevent: {name}\ndata: {data}\n\n").as_bytes())
                    .await?;
                stream.flush().await?;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // this client is too slow: never block the engine — force a resync
                stream.write_all(b"event: resync\ndata: {}\n\n").await?;
                stream.flush().await?;
                let _ = stream.shutdown().await;
                return Ok(());
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let _ = stream.shutdown().await;
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests: run over in-memory duplexes — no network needed.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::config::{DownloadConfig, EngineConfig};
    use engine_http::ReqwestBackend;
    use tokio::io::duplex;

    fn manager() -> Arc<DownloadManager> {
        let dir = tempfile::tempdir().unwrap();
        let cfg = EngineConfig {
            data_dir: dir.path().join("data"),
            default_out_dir: dir.path().join("out"),
            ..Default::default()
        };
        let backend = Arc::new(ReqwestBackend::new(&cfg).unwrap());
        DownloadManager::new(cfg, backend).unwrap()
    }

    /// Minimal harness: drive serve_http against in-memory TcpListener on
    /// 127.0.0.1:0 and speak real HTTP — this also exercises the actual parser.
    async fn spawn_server() -> (std::net::SocketAddr, Arc<DownloadManager>) {
        let mgr = manager();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_http(mgr.clone(), vec![listener], None, None));
        (addr, mgr)
    }

    async fn rpc(addr: std::net::SocketAddr, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        let status: u16 = text
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    #[tokio::test]
    async fn rpc_routes_through_v1_handle_line() {
        let (addr, _m) = spawn_server().await;
        let (status, text) = rpc(addr, r#"{"jsonrpc":"2.0","id":1,"method":"engine.info"}"#).await;
        assert_eq!(status, 200);
        assert!(
            text.contains("\"api_version\":\"1\""),
            "same handler as v1: {text}"
        );
        assert!(text.contains("X-Velox-Api: 2") || text.to_lowercase().contains("x-velox-api: 2"));
        // bad scheme is a JSON-RPC error, not a transport error
        let (status, text) = rpc(
            addr,
            r#"{"jsonrpc":"2.0","id":2,"method":"download.create","params":{"url":"ftp://x/f"}}"#,
        )
        .await;
        assert_eq!(status, 200);
        assert!(text.contains("INVALID_URL"), "{text}");
        // notification → 204 empty
        let (status, text) = rpc(addr, r#"{"jsonrpc":"2.0","method":"engine.info"}"#).await;
        assert_eq!(status, 204, "notifications never get a response body");
        assert!(text.ends_with("\r\n\r\n") || text.ends_with("\r\n"));
        let _ = _m;
    }

    #[tokio::test]
    async fn sse_delivers_sequenced_events_and_replays() {
        let (addr, mgr) = spawn_server().await;
        // connect FIRST (hello arrives), then trigger a structural event so it is
        // delivered LIVE — this exercises the exact path a UI uses
        let server =
            engine_testserver::TestServer::start(engine_testserver::Behavior::new(64 * 1024));
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        // trigger the events while the socket buffer holds them, then read
        let _added = mgr
            .add(&server.url("/file"), DownloadConfig::default())
            .await
            .unwrap();
        let _ = mgr.clone().start(1).await;
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(700), s.read_to_end(&mut buf)).await;
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("event: hello"), "{text}");
        assert!(
            text.contains("id: "),
            "events must carry sequence ids: {text}"
        );
        // a DownloadState event was delivered live (state created or downloading)
        assert!(
            text.contains("event: created") || text.contains("event: downloading"),
            "live state event expected: {text}"
        );
        server.stop();
    }

    // ------------------------------------------------------------------
    // Host/Origin validation regression tests (PHASE-1 finding B)
    // ------------------------------------------------------------------

    #[test]
    fn host_validation_accepts_exact_loopback() {
        let ok = |h: &str| validate_client_identity(&[("host".into(), h.into())]).is_ok();
        for h in [
            "localhost",
            "localhost:3000",
            "127.0.0.1",
            "127.0.0.1:3000",
            "[::1]",
            "[::1]:3000",
        ] {
            assert!(ok(h), "must accept {h}");
        }
    }

    #[test]
    fn host_validation_rejects_lookalikes_and_malformed() {
        let check = |h: &str| validate_client_identity(&[("host".into(), h.into())]);
        // lookalike hostnames / rebinding
        for h in [
            "localhost.attacker.example",
            "127.0.0.1.attacker.example",
            "localhostx",
            "xlocalhost",
            "0.0.0.0",
            "127.0.0.2",
            "127.1",           // shorthand IPv4 — non-canonical, reject
            "127.0.0.01",      // non-canonical octet
            "[::1]extra",      // junk after bracket
            "[::2]",           // wrong IPv6
            "a@localhost",     // userinfo trick
            "localhost:0",     // invalid port
            "localhost:99999", // port out of range
            "localhost:80x",   // non-numeric port
            "localhost:080",   // leading zeros
            "",                // empty
        ] {
            assert!(check(h).is_err(), "must reject {h}");
        }
        // malformed → 400, foreign → 403
        assert_eq!(check("127.0.0.1:bad").unwrap_err(), 400);
        assert_eq!(check("evil.example").unwrap_err(), 403);
    }

    #[test]
    fn host_validation_requires_single_local_host() {
        // missing Host → 400 (HTTP/1.1 requires it)
        assert_eq!(validate_client_identity(&[]).unwrap_err(), 400);
        // duplicate Host → 400 (smuggling)
        let dup = vec![
            ("host".to_string(), "127.0.0.1".to_string()),
            ("host".to_string(), "localhost".to_string()),
        ];
        assert_eq!(validate_client_identity(&dup).unwrap_err(), 400);
        // bare ::1 without brackets is malformed (RFC 3986 requires brackets)
        assert!(validate_client_identity(&[("host".into(), "::1".into())]).is_err());
    }

    #[test]
    fn origin_validation_rules() {
        let with_origin = |h: &str, o: &str| {
            validate_client_identity(&[("host".into(), h.into()), ("origin".into(), o.into())])
        };
        // same-origin UI + curl-style absent Origin pass
        assert!(with_origin("127.0.0.1:3000", "http://localhost:3000").is_ok());
        assert!(with_origin("127.0.0.1:3000", "http://127.0.0.1:3000").is_ok());
        assert!(validate_client_identity(&[("host".into(), "127.0.0.1:3000".into())]).is_ok());
        // foreign origins — the CSRF/DNS-rebinding vector — are refused
        for o in [
            "https://evil.example",
            "http://evil.example",
            "http://localhost.attacker.example:3000",
            "null", // sandboxed frame
        ] {
            assert_eq!(
                with_origin("127.0.0.1:3000", o).unwrap_err(),
                403,
                "must reject Origin {o}"
            );
        }
        // malformed Origin authority → 403
        assert_eq!(with_origin("127.0.0.1:3000", "http://").unwrap_err(), 403);
    }

    #[tokio::test]
    async fn non_local_host_is_rejected() {
        let (addr, _m) = spawn_server().await;
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"engine.info"}"#;
        let mut s = TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /rpc HTTP/1.1\r\nHost: evil.example\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(500), s.read_to_end(&mut buf)).await;
        assert!(
            String::from_utf8_lossy(&buf).contains("403"),
            "foreign Host must be refused"
        );
        let _ = _m;
    }

    #[tokio::test]
    async fn static_files_are_traversal_safe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"<h1>ok</h1>").unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"secret").unwrap();
        let mgr = manager();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_http(
            mgr,
            vec![listener],
            Some(dir.path().to_path_buf()),
            None,
        ));
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /index.html HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(500), s.read_to_end(&mut buf)).await;
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("<h1>ok</h1>"), "{text}");
        // traversal attempt → 404, never the file
        let mut s2 = TcpStream::connect(addr).await.unwrap();
        s2.write_all(b"GET /..%2fsecret.txt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut buf2 = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(500), s2.read_to_end(&mut buf2)).await;
        let text2 = String::from_utf8_lossy(&buf2);
        assert!(
            text2.contains("404"),
            "encoded traversal must be rejected: {text2}"
        );
        let mut s3 = TcpStream::connect(addr).await.unwrap();
        s3.write_all(b"GET /../secret.txt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut buf3 = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(500), s3.read_to_end(&mut buf3)).await;
        let text3 = String::from_utf8_lossy(&buf3);
        assert!(
            text3.contains("404"),
            "raw traversal must be rejected: {text3}"
        );
    }

    #[tokio::test]
    async fn wire_event_mapping_is_complete() {
        let (name, data) = wire_event(&Event::Progress {
            id: 3,
            received: 10,
            total: Some(20),
            speed_bps: 5.0,
            connections: 2,
        });
        assert_eq!(name, "progress");
        assert!(data.contains("\"segments\":2"));
        let (name, _) = wire_event(&Event::DownloadState {
            id: 1,
            state: "paused".into(),
        });
        assert_eq!(name, "paused");
        let (name, data) = wire_event(&Event::Completed {
            id: 1,
            path: "/x".into(),
            size: 5,
            sha256: None,
            blake3: None,
        });
        assert_eq!(name, "completed");
        assert!(data.contains("\"size\":5"));
        let (name, _) = wire_event(&Event::Restarting {
            id: 1,
            reason: "429".into(),
        });
        assert_eq!(name, "retrying");
        let (name, _) = wire_event(&Event::Failed {
            id: 1,
            kind: "IO".into(),
            message: "x".into(),
        });
        assert_eq!(name, "failed");
        let (name, _) = wire_event(&Event::Verified {
            id: 1,
            ok: true,
            algo: "sha256".into(),
        });
        assert_eq!(name, "verified");
    }

    #[test]
    fn duplex_smoke() {
        // ensure the duplex import is exercised in unit builds (kept for future tests)
        let (_a, _b) = duplex(64);
    }
}
