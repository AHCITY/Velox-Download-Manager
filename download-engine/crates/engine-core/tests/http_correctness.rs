//! P0 download-correctness regression suite (user-mandated scenarios).
//!
//! Every test drives the REAL manager + reqwest transport against the fault-injecting
//! local server and verifies the final file byte-for-byte (SHA-256) or asserts an
//! honest failure. The invariant under test: when Velox reports `completed`, the file
//! on disk IS the requested resource, with the correct name and extension — and a web
//! page is never silently reported as a successful download.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_core::config::{DownloadConfig, EngineConfig};
use engine_core::manager::DownloadManager;
use engine_core::task::compute_hash_blocking;
use engine_http::ReqwestBackend;
use engine_testserver::{content_pattern, Behavior, TestServer};

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

fn expected_hash(len: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("pattern");
    std::fs::write(&p, content_pattern(len)).unwrap();
    compute_hash_blocking(&p, engine_core::config::HashAlgo::Sha256).unwrap()
}

fn engine_cfg(data: &Path, out: &Path) -> EngineConfig {
    let mut c = EngineConfig {
        data_dir: data.to_path_buf(),
        default_out_dir: out.to_path_buf(),
        ..Default::default()
    };
    c.retry.base_delay = Duration::from_millis(120);
    c.retry.max_delay = Duration::from_secs(1);
    c
}

fn manager(cfg: &EngineConfig) -> Arc<DownloadManager> {
    DownloadManager::new(cfg.clone(), Arc::new(ReqwestBackend::new(cfg).unwrap())).unwrap()
}

fn temp_dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    (dir, data, out)
}

async fn wait_terminal(
    mgr: &Arc<DownloadManager>,
    id: i64,
    timeout: Duration,
) -> engine_core::manager::StatusView {
    let start = Instant::now();
    loop {
        let s = mgr.status(id).expect("status");
        if matches!(s.state.as_str(), "completed" | "failed" | "cancelled") {
            return s;
        }
        assert!(
            start.elapsed() < timeout,
            "timeout waiting for terminal state; last={} received={} err={:?}: {:?}",
            s.state,
            s.received,
            s.error_code,
            s.error_message
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn out_files(out: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            !n.ends_with(".lock") && !n.contains(".part-")
        })
        .collect();
    v.sort();
    v
}

/// Standard E2E: add → start → wait terminal → return (status, final_path).
async fn run_download(
    mgr: &Arc<DownloadManager>,
    url: &str,
    cfg: DownloadConfig,
) -> (engine_core::manager::StatusView, Option<PathBuf>) {
    let added = mgr
        .add(url, cfg)
        .await
        .unwrap_or_else(|e| panic!("add failed unexpectedly: {e}"));
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_terminal(mgr, added.id, Duration::from_secs(60)).await;
    let path = if s.state == "completed" {
        Some(PathBuf::from(&s.dest_dir).join(&s.final_name))
    } else {
        None
    };
    (s, path)
}

// ---------------------------------------------------------------------------
// 1. Direct MP4 download — correct name, extension, bytes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn direct_mp4_name_extension_and_bytes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(2 * MIB);
    b.serve_any_path = true;
    b.content_type = "video/mp4".into();
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, path) = run_download(
        &mgr,
        &server.url("/videos/clip.mp4"),
        DownloadConfig::default(),
    )
    .await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(s.final_name, "clip.mp4");
    let p = path.unwrap();
    assert!(p.exists());
    assert_eq!(
        compute_hash_blocking(&p, engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(2 * MIB)
    );
    assert_eq!(s.received, 2 * MIB as u64, "received == Content-Length");
    server.stop();
}

// ---------------------------------------------------------------------------
// 2. ZIP/PDF downloads via Content-Type + URL name
// ---------------------------------------------------------------------------
#[tokio::test]
async fn zip_and_pdf_types_resolve_extension_and_bytes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(700 * KIB);
    b.serve_any_path = true;
    b.content_type = "application/zip".into();
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    // URL without extension: Content-Type supplies it
    let (s, path) = run_download(
        &mgr,
        &server.url("/files/archive"),
        DownloadConfig::default(),
    )
    .await;
    assert_eq!(s.state, "completed");
    assert_eq!(s.final_name, "archive.zip", "MIME-derived extension");
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(700 * KIB)
    );

    let mut b2 = Behavior::new(300 * KIB);
    b2.serve_any_path = true;
    b2.content_type = "application/pdf".into();
    let server2 = TestServer::start(b2);
    let mgr2 = manager(&engine_cfg(&data, &out));
    let (s2, path2) = run_download(
        &mgr2,
        &server2.url("/docs/spec.pdf"),
        DownloadConfig::default(),
    )
    .await;
    assert_eq!(s2.state, "completed");
    assert_eq!(s2.final_name, "spec.pdf", "URL name kept as-is");
    assert_eq!(
        compute_hash_blocking(&path2.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(300 * KIB)
    );
    server.stop();
    server2.stop();
}

// ---------------------------------------------------------------------------
// 3. Redirect chain ending in a file with Content-Disposition
// ---------------------------------------------------------------------------
#[tokio::test]
async fn redirect_chain_uses_disposition_name_and_final_bytes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(400 * KIB);
    b.serve_any_path = true;
    b.content_disposition = Some("attachment; filename=\"real report.zip\"".into());
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    // /redirect/3 → ... → /file; the FINAL response carries the disposition
    let (s, path) = run_download(&mgr, &server.url("/redirect/3"), DownloadConfig::default()).await;
    assert_eq!(s.state, "completed");
    assert_eq!(
        s.final_name, "real report.zip",
        "name from Content-Disposition"
    );
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(400 * KIB)
    );
    assert_eq!(s.received, 400 * KIB as u64);
    server.stop();
}

// ---------------------------------------------------------------------------
// 4. HTML error page with HTTP 200 → must FAIL, never complete
// ---------------------------------------------------------------------------
#[tokio::test]
async fn html_error_page_with_200_is_rejected() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(4 * MIB);
    let page = b"<!DOCTYPE html><html><head><title>Not Found</title></head><body><h1>404</h1></body></html>".to_vec();
    b.page_html = Some(Arc::new(page));
    b.content_type = "text/html; charset=utf-8".into();
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let url = server.url("/videos/hero.mp4"); // page URL that claims to be a video
    let err = mgr
        .add(&url, DownloadConfig::default())
        .await
        .expect_err("must fail");
    assert_eq!(err.kind_code(), "NOT_A_FILE", "{err}");
    assert!(
        mgr.list().unwrap().is_empty(),
        "no download row is registered"
    );
    assert!(
        out_files(&out).is_empty(),
        "no file may appear in the output dir: {:?}",
        out_files(&out)
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 5. HTML page with a misleading Content-Disposition filename
// ---------------------------------------------------------------------------
#[tokio::test]
async fn html_with_misleading_disposition_is_rejected() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(4 * MIB);
    let page = b"<html><head><title>Login</title></head><body><form action=\"/login\"></form></body></html>".to_vec();
    b.page_html = Some(Arc::new(page));
    b.content_type = "text/html".into();
    b.content_disposition = Some("attachment; filename=\"movie.mp4\"".into());
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let url = server.url("/get?token=abc");
    let err = mgr
        .add(&url, DownloadConfig::default())
        .await
        .expect_err("must fail");
    assert_eq!(err.kind_code(), "NOT_A_FILE");
    let leftovers = out_files(&out);
    assert!(
        !leftovers
            .iter()
            .any(|p| p.file_name().unwrap().to_string_lossy() == "movie.mp4"),
        "a webpage must never be stored as the claimed filename: {leftovers:?}"
    );
    assert!(
        leftovers.is_empty(),
        "nothing at all should land: {leftovers:?}"
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 6. Genuine HTML download is preserved when the user asks for it
// ---------------------------------------------------------------------------
#[tokio::test]
async fn html_is_stored_when_user_requests_a_page() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(20 * KIB);
    let page =
        b"<!DOCTYPE html><html><body><h1>Saved page</h1><p>content</p></body></html>".to_vec();
    let page_len = page.len();
    b.page_html = Some(Arc::new(page));
    b.content_type = "text/html; charset=utf-8".into();
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));

    // explicit user intent: filename *.html
    let cfg = DownloadConfig {
        filename: Some("snapshot.html".into()),
        ..Default::default()
    };
    let (s, path) = run_download(&mgr, &server.url("/page"), cfg).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(s.final_name, "snapshot.html");
    let bytes = std::fs::read(path.unwrap()).unwrap();
    assert_eq!(bytes.len(), page_len);
    assert!(
        bytes.starts_with(b"<!DOCTYPE html>"),
        "stored bytes ARE the html page"
    );

    // URL-path intent: /reports/page.html
    let (s2, path2) = run_download(
        &mgr,
        &server.url("/reports/page.html"),
        DownloadConfig::default(),
    )
    .await;
    assert_eq!(s2.state, "completed", "{:?}", s2.error_message);
    assert_eq!(s2.final_name, "page.html");
    assert_eq!(std::fs::read(path2.unwrap()).unwrap().len(), page_len);
    server.stop();
}

// ---------------------------------------------------------------------------
// 7. Server ignores Range → plain GET path still completes byte-exact
// ---------------------------------------------------------------------------
#[tokio::test]
async fn range_ignoring_server_completes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(3 * MIB);
    b.serve_any_path = true;
    b.ignore_ranges = true;
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, path) = run_download(&mgr, &server.url("/file"), DownloadConfig::default()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(s.accept_ranges, "no");
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(3 * MIB)
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 8. Incorrect Content-Range → engine must not trust or splice wrong bytes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn wrong_content_range_never_completes_silently() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(2 * MIB);
    b.serve_any_path = true;
    b.wrong_content_range = true;
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let cfg = DownloadConfig {
        min_segment_size: Some(256 * KIB as u64), // force segmented: real ranged requests
        ..Default::default()
    };
    let (s, _path) = run_download(&mgr, &server.url("/file"), cfg).await;
    assert_eq!(
        s.state, "failed",
        "a server mis-echoing Content-Range must fail loudly, not complete"
    );
    let msg = format!("{:?} {:?}", s.error_code, s.error_message);
    assert!(
        msg.contains("range") || msg.contains("206") || msg.contains("restart"),
        "error should explain the range problem: {msg}"
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 9. Premature connection closure mid-body → recovers from the checkpoint
// ---------------------------------------------------------------------------
#[tokio::test]
async fn premature_close_recovers_and_matches_hash() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(2 * MIB);
    b.serve_any_path = true;
    b.close_after_bytes = 300 * KIB;
    b.close_sessions = Arc::new(std::sync::atomic::AtomicUsize::new(2));
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, path) = run_download(&mgr, &server.url("/file"), DownloadConfig::default()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert!(
        server.stats().closed_early >= 1,
        "the abort fixture must have fired"
    );
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(2 * MIB)
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 10. Truncated response (claims more bytes than sent, then closes) → FAIL
// ---------------------------------------------------------------------------
#[tokio::test]
async fn truncated_response_is_never_completed() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(512 * KIB);
    b.serve_any_path = true;
    b.claim_content_length = Some(3 * MIB as u64); // claims 3 MiB, sends 512 KiB, closes
    b.close_after_body = true;
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, _path) = run_download(&mgr, &server.url("/file"), DownloadConfig::default()).await;
    assert_eq!(
        s.state, "failed",
        "truncation must fail, not complete: {:?}",
        s.error_message
    );
    let msg = s.error_message.unwrap_or_default();
    // honest diagnostics for a lying server: the resume range lands beyond the real
    // body (416), or the close is detected directly — either way it FAILS
    assert!(
        msg.contains("closed early")
            || msg.contains("retries")
            || msg.contains("416")
            || msg.contains("range"),
        "error should explain the premature end: {msg}"
    );
    assert!(
        out_files(&out).is_empty(),
        "no final file may exist: {:?}",
        out_files(&out)
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 11. Unknown Content-Length (chunked) → stream completes with exact bytes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chunked_unknown_length_completes_with_exact_bytes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(800 * KIB);
    b.serve_any_path = true;
    b.chunked = true;
    b.ignore_ranges = true; // no 206 → no Content-Range total → genuinely unknown length
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, path) = run_download(&mgr, &server.url("/file"), DownloadConfig::default()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(s.total_size, None, "length was never announced");
    assert_eq!(s.received, 800 * KIB as u64);
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(800 * KIB)
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 11b. Overlong body (server sends MORE than the claimed Content-Length):
//      per RFC 7230 §3.3.3 the message body ends at Content-Length — the extra
//      bytes are keep-alive framing, not file content. The engine must cap its
//      reads at the announced length and complete with exactly the claimed
//      bytes (never corrupted by the overlong stream, never hung reading it).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn overlong_body_completes_with_exactly_claimed_bytes() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(2 * MIB); // real body: 2 MiB
    b.serve_any_path = true;
    b.claim_content_length = Some(MIB as u64); // announced: 1 MiB
    b.ignore_ranges = true; // single 200 response, plain Content-Length framing
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let (s, path) = run_download(&mgr, &server.url("/file"), DownloadConfig::default()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(
        s.received, MIB as u64,
        "must stop exactly at the announced length"
    );
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(MIB),
        "file bytes must be the FIRST claimed MiB of the stream"
    );
    server.stop();
}

// ---------------------------------------------------------------------------
// 12. Google Drive-style confirm interstitial → real file with real name
// ---------------------------------------------------------------------------
#[tokio::test]
async fn drive_style_confirm_page_resolves_to_the_real_file() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(600 * KIB);
    b.content_type = "application/x-iso9660-image".into();
    b.content_disposition = Some("attachment; filename=\"backup-image.iso\"".into());
    let confirm_page = r#"<!DOCTYPE html><html><head><title>Google Drive - Virus scan warning</title></head>
        <body><p>Google Drive can't scan this file for viruses.</p>
        <form id="download-form" action="/drive/confirm" method="get">
          <input type="hidden" name="id" value="fixture">
          <input type="hidden" name="export" value="download">
          <input type="hidden" name="confirm" value="t">
          <input type="hidden" name="uuid" value="fixture-uuid">
        </form></body></html>"#;

    b.drive_confirm_html = Some(Arc::new(confirm_page.as_bytes().to_vec()));
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    // the user pastes a /view share link — the engine must resolve it, walk the confirm
    // page, and land on the real file
    let url = server.url("/drive/uc?export=download&id=fixture");
    let (s, path) = run_download(&mgr, &url, DownloadConfig::default()).await;
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    assert_eq!(
        s.final_name, "backup-image.iso",
        "name from the real file's disposition"
    );
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), engine_core::config::HashAlgo::Sha256).unwrap(),
        expected_hash(600 * KIB)
    );
    assert_eq!(s.received, 600 * KIB as u64);
    server.stop();
}

// ---------------------------------------------------------------------------
// 13. YouTube watch URL → honest UNSUPPORTED_URL, never an HTML "video"
// ---------------------------------------------------------------------------
#[tokio::test]
async fn youtube_watch_url_fails_with_clear_guidance() {
    let (_d, data, out) = temp_dirs();
    let server = TestServer::start(Behavior::new(KIB)); // unreachable; classify first
    let mgr = manager(&engine_cfg(&data, &out));
    let url = "https://www.youtube.com/watch?v=XdNjEct5mqc";
    let err = mgr
        .add(url, DownloadConfig::default())
        .await
        .expect_err("must fail");
    assert_eq!(err.kind_code(), "UNSUPPORTED_URL", "{err}");
    assert!(
        err.message.contains("yt-dlp") && err.message.contains("extractor"),
        "the error must explain the remedy: {}",
        err.message
    );
    assert!(mgr.list().unwrap().is_empty(), "nothing registered");
    assert!(out_files(&out).is_empty());
    server.stop();
}

// ---------------------------------------------------------------------------
// 14. The full user-reported Drive share link shape (view URL)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn drive_view_share_link_is_resolved_not_stored_as_page() {
    let (_d, data, out) = temp_dirs();
    let mut b = Behavior::new(150 * KIB);
    b.content_type = "text/html; charset=utf-8".into();
    // the exact field-reported share link: the fixture serves what Drive serves —
    // the /view interstitial page — at the same path shape
    let view_path = "/file/d/1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13/view".to_string();
    let view_page = b"<html><head><title>holiday-photo.jpg - Google Drive</title></head><body><div class=\"uc-main\"></div></body></html>".to_vec();
    b.page_html = Some(Arc::new(view_page));
    let server = TestServer::start(b);
    let mgr = manager(&engine_cfg(&data, &out));
    let url = server.url(&view_path);
    let err = mgr
        .add(&url, DownloadConfig::default())
        .await
        .expect_err("view page must fail");
    assert_eq!(err.kind_code(), "NOT_A_FILE", "{err}");
    assert!(mgr.list().unwrap().is_empty());
    server.stop();
}

// ---------------------------------------------------------------------------
// 15. Byte-exact completion with checksum verification configured
// ---------------------------------------------------------------------------
#[tokio::test]
async fn configured_sha256_verify_matches_stored_bytes() {
    use engine_core::config::{HashAlgo, VerifyRequest};
    let (_d, data, out) = temp_dirs();
    let mut b15 = Behavior::new(MIB);
    b15.serve_any_path = true;
    let server = TestServer::start(b15);
    let mgr = manager(&engine_cfg(&data, &out));
    let cfg = DownloadConfig {
        verify: Some(VerifyRequest {
            algo: HashAlgo::Sha256,
            expected: expected_hash(MIB),
        }),
        ..Default::default()
    };
    let (s, path) = run_download(&mgr, &server.url("/file"), cfg).await;
    assert_eq!(s.state, "completed");
    assert!(s.verified, "row must record verification");
    assert_eq!(s.sha256.as_deref(), Some(expected_hash(MIB).as_str()));
    assert_eq!(
        compute_hash_blocking(&path.unwrap(), HashAlgo::Sha256).unwrap(),
        expected_hash(MIB)
    );
    server.stop();
}
