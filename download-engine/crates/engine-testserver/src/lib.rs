//! Controllable local HTTP/1.1 test server with fault injection (std-only, ADR-0005).
//!
//! Tests mutate `Behavior` mid-run and read `Stats`. Deliberately *not* RFC-perfect:
//! quirky server behavior is exactly what this oracle must reproduce.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Behavior {
    pub content: Arc<Vec<u8>>,
    pub etag_version: u64,
    pub last_modified: String,
    pub content_type: String,
    pub content_disposition: Option<String>,
    /// Delay before writing response headers.
    pub delay_ms: u64,
    /// Per-connection drip rate in bytes/second (0 = full speed).
    pub slow_bps: u64,
    /// Apply `slow_bps` only to range requests starting at or above this offset (steal tests).
    pub slow_above: Option<u64>,
    /// Body write granularity.
    pub chunk_size: usize,
    /// Transfer-Encoding: chunked, no Content-Length.
    pub chunked: bool,
    /// HTTP/1.0-style close-delimited body (no CL, no chunked).
    pub close_delimited: bool,
    /// Answer every request with 200 (ignore Range).
    pub ignore_ranges: bool,
    /// 206 with a deliberately wrong start in Content-Range.
    pub wrong_content_range: bool,
    /// 206 without a Content-Range header at all.
    pub omit_content_range: bool,
    /// Abort the connection after writing N body bytes.
    pub close_after_bytes: usize,
    /// Abort only for the next N connections (0 = every connection aborts).
    pub close_sessions: Arc<AtomicUsize>,
    /// Write N bytes then hang (only for the next `stall_sessions` connections).
    pub stall_after_bytes: usize,
    pub stall_sessions: Arc<AtomicUsize>,
    /// First N GETs get 429 + Retry-After: 1.
    pub return_429_times: Arc<AtomicUsize>,
    /// First N GETs get 503.
    pub return_503_times: Arc<AtomicUsize>,
    /// Reject HEAD with 501 (GET still works).
    pub refuse_head: bool,
    /// Serve weak ETags (W/"vN").
    pub weak_etag: bool,
    /// Send no ETag header at all.
    pub no_etag: bool,
    /// Once more than N total requests have been seen, answer 410 Gone (expiring URLs).
    pub expire_after_requests: Option<usize>,
    /// A hypertext page served with 200 + `content_type` on ANY non-reserved path
    /// (P0 regression fixtures: HTML error pages with HTTP 200, pages with a
    /// misleading Content-Disposition, view interstitials at their real path).
    pub page_html: Option<Arc<Vec<u8>>>,
    /// Claim a Content-Length LARGER than the body actually sent, then close the
    /// connection (truncated-response fixture; the engine must refuse completion).
    pub claim_content_length: Option<u64>,
    /// Serve the Google Drive-style confirm interstitial at /drive/uc when the confirm
    /// hop has not been visited yet (drive_stage == 0); the real file afterwards.
    pub drive_confirm_html: Option<Arc<Vec<u8>>>,
    /// Stage counter for the Drive fixture (0 = interstitial, ≥1 = real file).
    pub drive_stage: Arc<AtomicUsize>,
    /// Send the FULL claimed body, then close the socket without a clean end
    /// (with claim_content_length this is the truncated-response fixture).
    pub close_after_body: bool,
    /// Serve the /file representation for ANY non-reserved path (lets fixtures use
    /// realistic URL shapes like /videos/clip.mp4 without path whitelisting).
    pub serve_any_path: bool,
}

impl Behavior {
    pub fn new(len: usize) -> Self {
        Self {
            content: Arc::new(content_pattern(len)),
            etag_version: 1,
            last_modified: "Wed, 18 Nov 2026 10:00:00 GMT".into(),
            content_type: "application/octet-stream".into(),
            content_disposition: None,
            delay_ms: 0,
            slow_bps: 0,
            slow_above: None,
            chunk_size: 64 * 1024,
            chunked: false,
            close_delimited: false,
            ignore_ranges: false,
            wrong_content_range: false,
            omit_content_range: false,
            close_after_bytes: 0,
            close_sessions: Arc::new(AtomicUsize::new(0)),
            stall_after_bytes: 0,
            stall_sessions: Arc::new(AtomicUsize::new(0)),
            return_429_times: Arc::new(AtomicUsize::new(0)),
            return_503_times: Arc::new(AtomicUsize::new(0)),
            refuse_head: false,
            weak_etag: false,
            no_etag: false,
            expire_after_requests: None,
            page_html: None,
            claim_content_length: None,
            drive_confirm_html: None,
            drive_stage: Arc::new(AtomicUsize::new(0)),
            close_after_body: false,
            serve_any_path: false,
        }
    }

    pub fn etag(&self) -> String {
        let base = format!("\"v{}\"", self.etag_version);
        if self.weak_etag {
            format!("W/{base}")
        } else {
            base
        }
    }
}

/// Deterministic byte pattern so test hashes are stable.
pub fn content_pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub requests: usize,
    pub range_requests: usize,
    pub connections: usize,
    pub max_concurrent_seen: usize,
    pub current: usize,
    /// Concurrent bodies being transferred (what a connection cap actually governs —
    /// idle keep-alive sockets are not counted).
    pub transfer_current: usize,
    pub bytes_sent: u64,
    pub closed_early: usize,
    pub saw_auth_header: bool,
    pub saw_if_range: bool,
    pub last_range_start: Option<u64>,
    /// Every range-request start offset, in order (mechanical steal detection).
    pub range_starts: Vec<u64>,
}

struct Inner {
    behavior: Behavior,
    stats: Stats,
}

pub struct TestServer {
    pub addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    /// Bind on an ephemeral loopback port and start serving in a background thread.
    pub fn start(behavior: Behavior) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let inner = Arc::new(Mutex::new(Inner {
            behavior,
            stats: Stats::default(),
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let inner2 = inner.clone();
        let shutdown2 = shutdown.clone();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if shutdown2.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let inner = inner2.clone();
                std::thread::spawn(move || {
                    let _ = handle_connection(stream, inner);
                });
            }
        });
        TestServer {
            addr,
            inner,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn mutate_behavior(&self, f: impl FnOnce(&mut Behavior)) {
        let mut g = self.inner.lock().unwrap();
        f(&mut g.behavior);
    }

    pub fn stats(&self) -> Stats {
        self.inner.lock().unwrap().stats.clone()
    }

    pub fn reset_stats(&self) {
        self.inner.lock().unwrap().stats = Stats::default();
    }

    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // wake the accept loop
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(100));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    range: Option<(u64, Option<u64>)>,
    if_range: Option<String>,
    auth: bool,
    connection_close: bool,
}

fn handle_connection(mut stream: TcpStream, inner: Arc<Mutex<Inner>>) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    {
        let mut g = inner.lock().unwrap();
        g.stats.connections += 1;
        g.stats.current += 1;
        // NOTE: max_concurrent_seen is transfer-scoped (see serve_request); idle pooled
        // sockets must not inflate it.
    }
    let result = serve_one(&mut stream, &inner);
    let mut g = inner.lock().unwrap();
    g.stats.current = g.stats.current.saturating_sub(1);
    result
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Ok(None), // closed before request completed
            Ok(_) => {
                buf.push(byte[0]);
                if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
                    break;
                }
                if buf.len() > 64 * 1024 {
                    return Ok(None);
                }
            }
            Err(e) => return Err(e),
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let mut lines = text.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut req = Request {
        method,
        path,
        range: None,
        if_range: None,
        auth: false,
        connection_close: false,
    };
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().to_string();
        match k.as_str() {
            "range" => req.range = parse_range_header(&v),
            "if-range" => req.if_range = Some(v),
            "authorization" => req.auth = true,
            "connection" => req.connection_close = v.eq_ignore_ascii_case("close"),
            _ => {}
        }
    }
    Ok(Some(req))
}

/// Parse a single-range `bytes=a-b` / `bytes=a-` / `bytes=-N` header (suffix unsupported here).
fn parse_range_header(v: &str) -> Option<(u64, Option<u64>)> {
    let v = v.trim();
    let rest = v.strip_prefix("bytes=")?;
    if rest.contains(',') {
        return None; // multi-range: intentionally unhandled (server falls back to 200)
    }
    let (a, b) = rest.split_once('-')?;
    let a = a.trim();
    if a.is_empty() {
        return None; // suffix range: not needed by the engine; fall back to 200
    }
    let start: u64 = a.parse().ok()?;
    let end = b.trim().parse::<u64>().ok();
    Some((start, end))
}

fn serve_one(stream: &mut TcpStream, inner: &Arc<Mutex<Inner>>) -> std::io::Result<()> {
    loop {
        let Some(req) = read_request(stream)? else {
            return Ok(());
        };
        let keep_alive = serve_request(stream, inner, &req)?;
        if !keep_alive || req.connection_close {
            return Ok(());
        }
    }
}

fn serve_request(
    stream: &mut TcpStream,
    inner: &Arc<Mutex<Inner>>,
    req: &Request,
) -> std::io::Result<bool> {
    let (status_line_extra, body_plan, keep_alive);
    {
        let mut g = inner.lock().unwrap();
        g.stats.requests += 1;
        g.stats.saw_auth_header |= req.auth;
        g.stats.saw_if_range |= req.if_range.is_some();

        if req.path == "/__stats" {
            let s = &g.stats;
            let text = format!(
                "requests={} ranges={} conns={} max={} current={} bytes={} closed={} auth={} if_range={} last_range_start={}\n",
                s.requests, s.range_requests, s.connections, s.max_concurrent_seen, s.current,
                s.bytes_sent, s.closed_early, s.saw_auth_header as u8, s.saw_if_range as u8,
                s.last_range_start.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            );
            write_simple(stream, "200 OK", "text/plain", Some(text.as_bytes()))?;
            return Ok(true);
        }
        if req.path == "/__reset" {
            let s = &mut g.stats;
            *s = Stats::default();
            write_simple(stream, "200 OK", "text/plain", Some(b"ok\n"))?;
            return Ok(true);
        }
        if let Some(n) = req.path.strip_prefix("/redirect/") {
            let n: usize = n.parse().unwrap_or(1);
            let loc = if n <= 1 {
                "/file".to_string()
            } else {
                format!("/redirect/{}", n - 1)
            };
            write_redirect(stream, &loc)?;
            return Ok(true);
        }

        // ----- P0 regression fixtures -----
        if req.path.starts_with("/drive/confirm") {
            // the interstitial's form submits here; Drive then hands out the real file
            g.behavior.drive_stage.store(1, Ordering::SeqCst);
            write_redirect(stream, "/drive/uc?confirm=t&uuid=fixture-uuid")?;
            return Ok(true);
        }
        if req.path.starts_with("/drive/uc") {
            let stage = g.behavior.drive_stage.load(Ordering::SeqCst);
            if stage == 0 {
                if let Some(page) = g.behavior.drive_confirm_html.clone() {
                    let headers = vec![("Content-Type".into(), "text/html; charset=utf-8".into())];
                    write_with_body(stream, "200 OK", &headers, Some(&page), false)?;
                    return Ok(true);
                }
            }
            // real file (post-confirmation): full 200, Content-Disposition naming it
            let total = g.behavior.content.len() as u64;
            let mut headers: Vec<(String, String)> = vec![
                ("Content-Type".into(), g.behavior.content_type.clone()),
                ("Content-Length".into(), total.to_string()),
                ("Accept-Ranges".into(), "none".into()),
            ];
            if let Some(cd) = &g.behavior.content_disposition {
                headers.push(("Content-Disposition".into(), cd.clone()));
            }
            write_with_body(stream, "200 OK", &headers, Some(&g.behavior.content), false)?;
            return Ok(true);
        }

        // page fixtures win over file serving (any non-reserved path)
        if let Some(page) = g.behavior.page_html.clone() {
            let mut headers: Vec<(String, String)> =
                vec![("Content-Type".into(), g.behavior.content_type.clone())];
            if let Some(cd) = &g.behavior.content_disposition {
                headers.push(("Content-Disposition".into(), cd.clone()));
            }
            write_with_body(stream, "200 OK", &headers, Some(&page), false)?;
            return Ok(true);
        }
        if req.path != "/file" && !g.behavior.serve_any_path {
            write_simple(stream, "404 Not Found", "text/plain", Some(b"not found\n"))?;
            return Ok(true);
        }

        // ----- /file dispatch -----
        if g.behavior.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(g.behavior.delay_ms));
        }
        if req.method == "HEAD" && g.behavior.refuse_head {
            write_simple(
                stream,
                "501 Not Implemented",
                "text/plain",
                Some(b"no HEAD here\n"),
            )?;
            return Ok(true);
        }
        if req.method != "GET" && req.method != "HEAD" {
            write_simple(
                stream,
                "405 Method Not Allowed",
                "text/plain",
                Some(b"GET only\n"),
            )?;
            return Ok(true);
        }
        if g.behavior.return_429_times.load(Ordering::SeqCst) > 0 {
            g.behavior.return_429_times.fetch_sub(1, Ordering::SeqCst);
            write_simple_with(
                stream,
                "429 Too Many Requests",
                &[("Retry-After", "1")],
                None,
            )?;
            return Ok(true);
        }
        if g.behavior.return_503_times.load(Ordering::SeqCst) > 0 {
            g.behavior.return_503_times.fetch_sub(1, Ordering::SeqCst);
            write_simple_with(stream, "503 Service Unavailable", &[], None)?;
            return Ok(true);
        }
        // expiring URLs: after N total requests the resource is gone (RFC 9110 §15.5.13)
        if let Some(n) = g.behavior.expire_after_requests {
            if g.stats.requests > n {
                write_simple(stream, "410 Gone", "text/plain", Some(b"expired\n"))?;
                return Ok(true);
            }
        }

        let total = g.behavior.content.len() as u64;
        let etag = g.behavior.etag();
        let head_only = req.method == "HEAD";

        // conditional semantics: If-Range mismatch ⇒ full 200 (RFC 9110 §13.1.5)
        let if_range_failed = req.if_range.as_deref().is_some_and(|ir| ir != etag);

        // malformed int-range (last-pos < first-pos) is unsatisfiable → 416 (RFC 9110 §14.1.1)
        if let Some((a, Some(b))) = req.range {
            if b < a {
                let hdr = format!("bytes */{total}");
                write_416(stream, &hdr, &etag, &g.behavior)?;
                return Ok(true);
            }
        }

        let (start, end) = match (req.range, g.behavior.ignore_ranges || if_range_failed) {
            (Some((a, b)), false) if a < total => {
                g.stats.range_requests += 1;
                g.stats.last_range_start = Some(a);
                g.stats.range_starts.push(a);
                let e = b.map(|x| x.min(total - 1)).unwrap_or(total - 1);
                (a, e)
            }
            (Some((a, _)), false) if a >= total => {
                // unsatisfiable
                let hdr = format!("bytes */{total}");
                write_416(stream, &hdr, &etag, &g.behavior)?;
                return Ok(true);
            }
            _ => (0, total - 1), // full body (200)
        };

        // a ranged request is answered 206 unless it happens to cover the whole representation
        let range_requested = req.range.is_some() && !g.behavior.ignore_ranges && !if_range_failed;
        let is_partial = range_requested && !(start == 0 && end == total - 1);
        let status = if is_partial {
            "206 Partial Content"
        } else {
            "200 OK"
        };

        let mut headers: Vec<(String, String)> = vec![
            ("Last-Modified".into(), g.behavior.last_modified.clone()),
            ("Accept-Ranges".into(), "bytes".into()),
            ("Content-Type".into(), g.behavior.content_type.clone()),
        ];
        if !g.behavior.no_etag {
            headers.insert(0, ("ETag".into(), etag.clone()));
        }
        if let Some(cd) = &g.behavior.content_disposition {
            headers.push(("Content-Disposition".into(), cd.clone()));
        }
        if is_partial && !g.behavior.omit_content_range {
            let (s, e) = if g.behavior.wrong_content_range {
                (start + 1, end + 1)
            } else {
                (start, end)
            };
            let claimed_total = g.behavior.claim_content_length.unwrap_or(total);
            headers.push((
                "Content-Range".into(),
                format!("bytes {s}-{e}/{claimed_total}"),
            ));
        }
        if !g.behavior.chunked && !g.behavior.close_delimited {
            let real = end - start + 1;
            // truncated-response fixture: claim MORE than the body will carry
            let claimed = g.behavior.claim_content_length.unwrap_or(real);
            headers.push(("Content-Length".into(), claimed.to_string()));
        }
        if g.behavior.chunked {
            headers.push(("Transfer-Encoding".into(), "chunked".into()));
        }

        body_plan = BodyPlan {
            data: g.behavior.content.clone(),
            start,
            end,
            chunk_size: g.behavior.chunk_size,
            // slow_above=None → global drip; Some(a) → drip only for ranges starting ≥ a
            slow_bps: if g.behavior.slow_above.is_none_or(|a| start >= a) {
                g.behavior.slow_bps
            } else {
                0
            },
            chunked: g.behavior.chunked,
            close_delimited: g.behavior.close_delimited,
            // consume a close session only for bodies large enough to reach the mark
            close_after_bytes: {
                let body_len = end - start + 1;
                if g.behavior.close_sessions.load(Ordering::SeqCst) > 0
                    && body_len >= g.behavior.close_after_bytes as u64
                {
                    g.behavior.close_sessions.fetch_sub(1, Ordering::SeqCst);
                    g.behavior.close_after_bytes
                } else {
                    0
                }
            },
            // consume a stall session only for bodies large enough to actually reach the mark
            stall_after_bytes: {
                let body_len = end - start + 1;
                if g.behavior.stall_sessions.load(Ordering::SeqCst) > 0
                    && body_len >= g.behavior.stall_after_bytes as u64
                {
                    g.behavior.stall_sessions.fetch_sub(1, Ordering::SeqCst);
                    g.behavior.stall_after_bytes
                } else {
                    0
                }
            },
            head_only,
            close_after_body: g.behavior.close_after_body,
        };
        status_line_extra = (status.to_string(), headers);
        keep_alive = !g.behavior.close_delimited && !g.behavior.chunked;
    }

    let (status, headers) = status_line_extra;
    {
        let mut g = inner.lock().unwrap();
        g.stats.transfer_current += 1;
        g.stats.max_concurrent_seen = g.stats.max_concurrent_seen.max(g.stats.transfer_current);
    }
    let result = send_body(stream, &status, &headers, &body_plan, inner);
    inner.lock().unwrap().stats.transfer_current -= 1;
    let aborted = result?;
    if aborted {
        // a mid-body abort must actually CLOSE the socket, not return to the keep-alive
        // loop: a half-sent body with the connection held open is a stall, not a close
        return Ok(false);
    }
    Ok(keep_alive)
}

struct BodyPlan {
    data: Arc<Vec<u8>>,
    start: u64,
    end: u64,
    chunk_size: usize,
    slow_bps: u64,
    chunked: bool,
    close_delimited: bool,
    close_after_bytes: usize,
    stall_after_bytes: usize,
    head_only: bool,
    close_after_body: bool,
}

fn send_body(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(String, String)],
    plan: &BodyPlan,
    inner: &Arc<Mutex<Inner>>,
) -> std::io::Result<bool> {
    let transfer_log = std::env::var("VELOX_TESTSERVER_LOG").is_ok();
    if transfer_log {
        eprintln!(
            "[ts] transfer {}-{} ({} bytes) slow={} t={:?}",
            plan.start,
            plan.end,
            plan.end + 1 - plan.start,
            plan.slow_bps,
            std::time::Instant::now()
        );
    }
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if plan.close_delimited {
        head = head.replacen("HTTP/1.1", "HTTP/1.0", 1);
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if plan.head_only {
        stream.flush()?;
        return Ok(false);
    }
    let body = &plan.data[plan.start as usize..=(plan.end as usize)];
    let mut sent = 0usize;
    let stall_at = plan.stall_after_bytes;
    let close_at = plan.close_after_bytes;
    let chunk = plan.chunk_size.max(1);
    while sent < body.len() {
        let n = chunk.min(body.len() - sent);
        let piece = &body[sent..sent + n];
        if plan.chunked {
            stream.write_all(format!("{n:x}\r\n").as_bytes())?;
            stream.write_all(piece)?;
            stream.write_all(b"\r\n")?;
        } else {
            stream.write_all(piece)?;
        }
        sent += n;
        inner.lock().unwrap().stats.bytes_sent += n as u64;
        if plan.slow_bps > 0 {
            let secs = n as f64 / plan.slow_bps as f64;
            std::thread::sleep(Duration::from_secs_f64(secs.min(0.5)));
        }
        if stall_at > 0 && sent >= stall_at {
            // hang long enough that any sane read-timeout fires; then close
            std::thread::sleep(Duration::from_secs(60));
            if transfer_log {
                eprintln!("[ts] stall abort {}-{}", plan.start, plan.end);
            }
            return Ok(true);
        }
        if close_at > 0 && sent >= close_at {
            let mut g = inner.lock().unwrap();
            g.stats.closed_early += 1;
            if transfer_log {
                eprintln!(
                    "[ts] early-close abort {}-{} at {sent}",
                    plan.start, plan.end
                );
            }
            // signal the caller to CLOSE the socket mid-body (a real premature close)
            return Ok(true);
        }
    }
    if plan.chunked {
        stream.write_all(b"0\r\n\r\n")?;
    }
    stream.flush()?;
    if transfer_log {
        eprintln!("[ts] transfer complete {}-{}", plan.start, plan.end);
    }
    if plan.close_after_body {
        inner.lock().unwrap().stats.closed_early += 1;
        return Ok(true); // close mid-framing: the client must NOT see a clean end
    }
    Ok(false)
}

fn write_simple(
    stream: &mut TcpStream,
    status: &str,
    ctype: &str,
    body: Option<&[u8]>,
) -> std::io::Result<()> {
    let headers = vec![("Content-Type".to_string(), ctype.to_string())];
    write_with_body(stream, status, &headers, body, false)
}

fn write_simple_with(
    stream: &mut TcpStream,
    status: &str,
    extra: &[(&str, &str)],
    body: Option<&[u8]>,
) -> std::io::Result<()> {
    let headers: Vec<(String, String)> = extra
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    write_with_body(stream, status, &headers, body, false)
}

fn write_redirect(stream: &mut TcpStream, location: &str) -> std::io::Result<()> {
    write_with_body(
        stream,
        "302 Found",
        &[("Location".to_string(), location.to_string())],
        Some(b"redirecting\n"),
        false,
    )
}

fn write_416(
    stream: &mut TcpStream,
    content_range: &str,
    etag: &str,
    behavior: &Behavior,
) -> std::io::Result<()> {
    let headers = vec![
        ("Content-Range".to_string(), content_range.to_string()),
        ("ETag".to_string(), etag.to_string()),
        ("Content-Type".to_string(), behavior.content_type.clone()),
    ];
    write_with_body(
        stream,
        "416 Range Not Satisfiable",
        &headers,
        Some(b"range not satisfiable\n"),
        false,
    )
}

fn write_with_body(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    chunked: bool,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if chunked {
        head.push_str("Transfer-Encoding: chunked\r\n");
    } else if let Some(b) = body {
        head.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if let Some(b) = body {
        stream.write_all(b)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn get_all(url: &str) -> (u16, Vec<u8>, Vec<String>) {
        let rest = url.trim_start_matches("http://");
        let (addr, path) = rest.split_once('/').expect("url has a path");
        let mut stream = TcpStream::connect(addr).unwrap();
        let path = format!("/{path}");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
            )
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        let status = String::from_utf8_lossy(&buf[..40])
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let text = String::from_utf8_lossy(&buf);
        let headers: Vec<String> = text.lines().take(10).map(str::to_string).collect();
        let body_start = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(buf.len());
        (status, buf[body_start..].to_vec(), headers)
    }

    #[test]
    fn basic_get_and_range() {
        let server = TestServer::start(Behavior::new(1000));
        let (status, body, _) = get_all(&server.url("/file"));
        assert_eq!(status, 200);
        assert_eq!(body.len(), 1000);

        let mut stream = TcpStream::connect(server.addr).unwrap();
        stream.write_all(b"GET /file HTTP/1.1\r\nHost: x\r\nRange: bytes=100-199\r\nConnection: close\r\n\r\n").unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        let text = String::from_utf8_lossy(&buf);
        assert!(text.starts_with("HTTP/1.1 206"), "got {text}");
        assert!(text.contains("Content-Range: bytes 100-199/1000"));
        let body = &buf[buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4..];
        assert_eq!(body.len(), 100);

        let content = content_pattern(1000);
        assert_eq!(body, &content[100..200]);

        server.stop();
    }

    #[test]
    fn unsatisfiable_range_and_redirect() {
        let server = TestServer::start(Behavior::new(10));
        let mut stream = TcpStream::connect(server.addr).unwrap();
        stream
            .write_all(
                b"GET /file HTTP/1.1\r\nHost: x\r\nRange: bytes=99-\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 416"));
        assert!(String::from_utf8_lossy(&buf).contains("bytes */10"));

        let (status, _, _) = get_all(&server.url("/redirect/3"));
        assert_eq!(status, 302);
        server.stop();
    }
}
