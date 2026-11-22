//! P3: HTTP compatibility audit — controlled server behaviors from the directive list.
//! Complements integration.rs; every scenario uses the fault-injecting local server only.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine_core::config::{DownloadConfig, EngineConfig, HashAlgo};
use engine_core::manager::{DownloadManager, StatusView};
use engine_core::task::compute_hash_blocking;
use engine_http::ReqwestBackend;
use engine_testserver::{content_pattern, Behavior, TestServer};

const KIB: usize = 1024;

fn expected_hash(len: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("pattern");
    std::fs::write(&p, content_pattern(len)).unwrap();
    compute_hash_blocking(&p, HashAlgo::Sha256).unwrap()
}

fn engine_cfg(data: &Path, out: &Path, tweak: impl FnOnce(&mut EngineConfig)) -> EngineConfig {
    let mut c = EngineConfig {
        data_dir: data.to_path_buf(),
        default_out_dir: out.to_path_buf(),
        ..Default::default()
    };
    c.retry.base_delay = Duration::from_millis(200);
    c.retry.max_delay = Duration::from_secs(2);
    tweak(&mut c);
    c
}

fn manager(cfg: &EngineConfig) -> std::sync::Arc<DownloadManager> {
    DownloadManager::new(
        cfg.clone(),
        std::sync::Arc::new(ReqwestBackend::new(cfg).unwrap()),
    )
    .unwrap()
}

async fn wait_state(
    mgr: &std::sync::Arc<DownloadManager>,
    id: i64,
    want: &[&str],
    timeout: Duration,
) -> StatusView {
    let start = Instant::now();
    loop {
        let s = mgr.status(id).expect("status");
        if want.contains(&s.state.as_str()) {
            return s;
        }
        assert!(
            start.elapsed() < timeout,
            "timeout waiting for {want:?}; state={} err={:?}: {:?}",
            s.state,
            s.error_code,
            s.error_message
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn head_unsupported_server_still_downloads() {
    // some servers break on HEAD; the engine's probe is a ranged GET and never uses HEAD
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(200 * KIB);
    b.refuse_head = true;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(200 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn weak_etag_resume_never_sends_if_range() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(1_200 * KIB);
    b.weak_etag = true;
    b.slow_bps = 600 * KIB as u64;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    // wait for progress, pause, resume
    let start = Instant::now();
    loop {
        let s = mgr.status(added.id).unwrap();
        if s.received > 400 * KIB as u64 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    mgr.pause(added.id).await.unwrap();
    wait_state(&mgr, added.id, &["paused"], Duration::from_secs(10)).await;
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(60)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(1_200 * KIB)
    );
    // RFC 9110 §13.1.5: weak ETags must never appear in If-Range
    assert!(
        !server.stats().saw_if_range,
        "If-Range sent despite weak ETag"
    );
    server.stop();
}

#[tokio::test]
async fn missing_etag_resume_still_completes() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(600 * KIB);
    b.no_etag = true;
    b.slow_bps = 600 * KIB as u64;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let start = Instant::now();
    loop {
        let s = mgr.status(added.id).unwrap();
        if s.received > 200 * KIB as u64 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    mgr.pause(added.id).await.unwrap();
    wait_state(&mgr, added.id, &["paused"], Duration::from_secs(10)).await;
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(60)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn changed_last_modified_forces_restart_and_stays_correct() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(600 * KIB);
    b.no_etag = true; // Last-Modified is then the only validator
    b.slow_bps = 600 * KIB as u64;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let start = Instant::now();
    loop {
        let s = mgr.status(added.id).unwrap();
        if s.received > 200 * KIB as u64 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    mgr.pause(added.id).await.unwrap();
    wait_state(&mgr, added.id, &["paused"], Duration::from_secs(10)).await;
    let sent_before = server.stats().bytes_sent;
    server.mutate_behavior(|b| b.last_modified = "Thu, 19 Nov 2026 00:00:00 GMT".into());
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(60)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    let sent = server.stats().bytes_sent - sent_before;
    assert!(
        sent >= 600 * KIB as u64,
        "validator change must force a full restart, got {sent}"
    );
    server.stop();
}

#[tokio::test]
async fn server_503_is_retried_and_download_completes() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(100 * KIB);
    b.return_503_times = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    let _ = s;
    server.stop();
}

#[tokio::test]
async fn pause_settles_during_probe_retries() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(100 * KIB);
    // request 1 = add()'s inspect (must succeed). Requests 2+3 = the task's
    // post-start probe → 503,503 opens a ~3s retry window; pre-fix, the task
    // parked through it ignoring the command watch and pause hit its deadline.
    b.fail_503_at_requests = std::sync::Arc::new(std::sync::Mutex::new(vec![2, 3]));
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    // the task is now inside the probe retry window
    tokio::time::sleep(Duration::from_millis(300)).await;

    let t0 = std::time::Instant::now();
    mgr.pause(added.id)
        .await
        .expect("pause must settle during probe retries");
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "pause took {elapsed:?} — probe loop ignored the stop command"
    );
    wait_state(&mgr, added.id, &["paused"], Duration::from_secs(10)).await;

    // the paused row stays resumable and completes byte-exact afterwards
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn cross_origin_redirect_strips_credentials_but_keeps_range() {
    let dir = tempfile::tempdir().unwrap();
    let server_b = TestServer::start(Behavior::new(100 * KIB));
    let mut b_a = Behavior::new(0); // never serves content itself — always redirects
    *b_a.redirect_to.lock().unwrap() = Some(server_b.url("/file"));
    b_a.redirect_times = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let server_a = TestServer::start(b_a);

    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let dl = DownloadConfig {
        auth: Some(engine_core::config::AuthConfig::Bearer {
            token: "secret-token-123".into(),
        }),
        ..DownloadConfig::default()
    };
    let added = mgr.add(&server_a.url("/file"), dl).await.unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    // the bearer token reached the origin it was configured for...
    assert!(
        server_a.stats().saw_auth_header,
        "bearer token missing on the authenticated server"
    );
    // ...but was stripped on the cross-origin hop (audit P1-4 regression guard)
    assert!(
        !server_b.stats().saw_auth_header,
        "credentials leaked to the cross-origin redirect target"
    );
    // Range survives the hop (resume correctness across redirects)
    assert!(
        server_b.stats().range_requests > 0,
        "Range header lost across the cross-origin redirect"
    );
    server_a.stop();
    server_b.stop();
}

#[tokio::test]
async fn redirect_loop_fails_instead_of_spinning() {
    let dir = tempfile::tempdir().unwrap();
    let mut b_a = Behavior::new(0);
    let a_redir = b_a.redirect_to.clone();
    b_a.redirect_times = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let server_a = TestServer::start(b_a);
    let mut b_b = Behavior::new(0);
    let b_redir = b_b.redirect_to.clone();
    b_b.redirect_times = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let server_b = TestServer::start(b_b);
    // wire A → B → A: an infinite redirect loop across two origins
    *a_redir.lock().unwrap() = Some(server_b.url("/file"));
    *b_redir.lock().unwrap() = Some(server_a.url("/file"));

    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let err = mgr
        .add(&server_a.url("/file"), DownloadConfig::default())
        .await
        .expect_err("a redirect loop must fail the probe, not spin forever");
    assert!(
        err.message.contains("redirect loop"),
        "unexpected error: {err}"
    );
    server_a.stop();
    server_b.stop();
}

#[tokio::test]
async fn sustained_429_exhausts_retries_honestly_and_stays_resumable() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(100 * KIB);
    // requests 1 (add inspect) and 2 (task probe) succeed; from request 3 on the
    // server rate-limits forever → the worker retries 5 attempts then fails
    let limiter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(3));
    b.sustained_429_from_request = limiter.clone();
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();

    let s = wait_state(&mgr, added.id, &["failed"], Duration::from_secs(60)).await;
    assert!(
        s.error_message
            .as_deref()
            .unwrap_or_default()
            .contains("429"),
        "expected an honest HTTP 429 failure, got: {:?}",
        s.error_message
    );

    // no final file, but the row stays resumable
    let row = mgr.db().get_download(added.id).unwrap().unwrap();
    let part = row.temp_path.as_deref().map(std::path::PathBuf::from);
    assert!(
        part.map(|p| p.is_file()).unwrap_or(false),
        "part file vanished after retry exhaustion — resume impossible"
    );

    // clear the limiter → resume completes byte-exact
    limiter.store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn expired_resolved_url_403_re_resolves_once_and_completes() {
    // request 1 = add() inspect (ok). Request 2 = the task's probe of the
    // RESOLVED url → 403, the expired-media-URL signature (googlevideo links die
    // after hours). The engine must clear the resolved url, re-probe the original
    // page (request 3, re-running classification/resolve) and complete byte-exact.
    let dir = tempfile::tempdir().unwrap();
    let b = Behavior::new(100 * KIB);
    *b.fail_status_at_requests.lock().unwrap() = std::collections::HashMap::from([(2, 403u16)]);
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn sustained_403_on_resolved_url_fails_after_one_re_resolve() {
    // 403 on the resolved url AND on the re-resolved page: exactly one bounded
    // re-resolve, then an honest failed row (never a loop, never silent success).
    let dir = tempfile::tempdir().unwrap();
    let b = Behavior::new(100 * KIB);
    *b.fail_status_at_requests.lock().unwrap() =
        std::collections::HashMap::from([(2, 403u16), (3, 403u16)]);
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["failed"], Duration::from_secs(30)).await;
    assert!(
        s.error_message
            .as_deref()
            .unwrap_or_default()
            .contains("403"),
        "expected the honest 403 failure, got: {:?}",
        s.error_message
    );
    server.stop();
}

#[tokio::test]
async fn expiring_url_410_fails_permanently_without_exposing_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(300 * KIB);
    b.expire_after_requests = Some(1); // probe ok, everything afterwards → 410 Gone
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |c| {
        c.min_segment_size = 64 * KIB as u64
    }));
    let added = mgr
        .add(
            &server.url("/file"),
            DownloadConfig {
                max_connections: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["failed"], Duration::from_secs(30)).await;
    assert_eq!(s.error_code.as_deref(), Some("HTTP_410"));
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert!(
        !final_path.exists(),
        "failed download must never be exposed as complete"
    );
    server.stop();
}

#[tokio::test]
async fn zero_length_resource_completes_immediately() {
    // probe gets 416 with `Content-Range: bytes */0` → total 0 → empty completed file
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(Behavior::new(0));
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    assert_eq!(added.total_size, Some(0));
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(15)).await;
    assert_eq!(s.received, 0);
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert!(final_path.exists());
    assert_eq!(std::fs::metadata(&final_path).unwrap().len(), 0);
    server.stop();
}

#[tokio::test]
async fn chunked_transfer_downloads_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(500 * KIB);
    b.chunked = true; // Transfer-Encoding: chunked, no Content-Length
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(500 * KIB)
    );
    let _ = s;
    server.stop();
}

#[tokio::test]
async fn broken_206_without_content_range_recovers_via_plain_get() {
    // RFC-invalid 206 (no Content-Range) on the probe: the engine must not trust the partial
    // Content-Length; it falls back to a plain GET and still produces the exact bytes.
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(400 * KIB);
    b.omit_content_range = true;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    assert_eq!(
        added.total_size, None,
        "total must be unknown when 206 lacks Content-Range"
    );
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    assert_eq!(s.mode, "stream", "no trustworthy length → streaming mode");
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(400 * KIB)
    );
    server.stop();
}

#[tokio::test]
async fn mid_body_close_retries_from_checkpoint() {
    // two connections are aborted mid-body; the engine must resume each from its checkpoint
    // and still produce byte-exact output (premature EOF is never accepted as completion)
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(600 * KIB);
    b.close_after_bytes = 100 * KIB;
    b.close_sessions = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(2));
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(60)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    assert!(
        server.stats().closed_early >= 2,
        "expected the injected mid-body closes"
    );
    let _ = s;
    server.stop();
}

#[tokio::test]
async fn changed_etag_mid_flight_restarts_even_in_single_mode() {
    // audit A7 regression: probe sees ETag v1; the worker's full-range request is answered
    // 200 because If-Range no longer matches (v2). In Single mode there is no other segment
    // to trigger the restart — a 200 must NOT be consumed unless validators match.
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(100 * KIB);
    b.slow_bps = 200 * KIB as u64;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    // the probe saw v1; rotate the validator before any data request
    server.mutate_behavior(|b| b.etag_version = 2);
    let sent_before = server.stats().bytes_sent;
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(100 * KIB)
    );
    // the restart re-fetched the whole resource after the initial If-Range failure
    assert!(
        server.stats().bytes_sent - sent_before >= 100 * KIB as u64,
        "changed ETag must force a fresh fetch, got {} bytes",
        server.stats().bytes_sent - sent_before
    );
    let _ = s;
    server.stop();
}

#[tokio::test]
async fn validator_less_resume_with_changed_length_restarts_from_zero() {
    // audit H3 regression: a server without ETag/Last-Modified cannot prove identity;
    // a changed total length must force a restart, never splice new bytes over old
    // positions (the old failure mode silently committed a truncated file as completed).
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(1_000 * KIB);
    b.no_etag = true;
    b.slow_bps = 400 * KIB as u64;
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    // pause mid-download (~40% of 1 MiB)
    let start = Instant::now();
    loop {
        let s = mgr.status(added.id).unwrap();
        if s.received > 400 * KIB as u64 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    mgr.pause(added.id).await.unwrap();
    wait_state(&mgr, added.id, &["paused"], Duration::from_secs(10)).await;
    // the resource is replaced by a longer version with the SAME Last-Modified string
    // (only the length changes — exactly the case the old code missed)
    let big = content_pattern(1_600 * KIB);
    server.mutate_behavior(|b| b.content = std::sync::Arc::new(big));
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(60)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(1_600 * KIB),
        "resume must produce the NEW resource in full, not a spliced prefix"
    );
    server.stop();
}

#[tokio::test]
async fn stalled_connection_is_detected_and_retried() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Behavior::new(600 * KIB);
    b.stall_after_bytes = 64 * KIB;
    b.stall_sessions = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let server = TestServer::start(b);
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |c| {
        c.stall_timeout = Duration::from_secs(2);
    }));
    let t0 = Instant::now();
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    assert!(
        t0.elapsed() >= Duration::from_secs(2),
        "stall must be detected, not instantaneous"
    );
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    let _ = s;
    server.stop();
}

#[tokio::test]
async fn mid_download_503_is_retried_not_restarted() {
    // audit A6 regression: 503 on a live data request is a transient — the SAME segment
    // retries with backoff; a restart would re-probe and waste the whole restart budget.
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(Behavior::new(600 * KIB));
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    let mgr = manager(&engine_cfg(&data, &out, |_| {}));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    // arm 503s AFTER the probe so they hit the workers' data requests
    server.mutate_behavior(|b| {
        b.return_503_times = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(2));
    });
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let final_path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    // 1 probe + 2 rejected data requests + successful data request(s)
    assert!(
        server.stats().requests >= 4,
        "503s must be retried, got {} requests",
        server.stats().requests
    );
    server.stop();
}
