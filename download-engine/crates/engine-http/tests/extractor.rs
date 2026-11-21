//! End-to-end media-extraction evidence: a STUB extractor (the same contract as yt-dlp:
//! `-J --no-playlist <url>` → single JSON on stdout) resolves a watch URL to a
//! progressive https stream hosted by the local test server. The engine must download
//! the real bytes with the extractor-proposed name — proving the full path
//! classify → spawn extractor → pick stream → generic HTTP download → verify,
//! independent of YouTube's own bot walls.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_core::config::{EngineConfig, HashAlgo};
use engine_core::manager::DownloadManager;
use engine_core::task::compute_hash_blocking;
use engine_http::ReqwestBackend;
use engine_testserver::{content_pattern, Behavior, TestServer};

const KIB: usize = 1024;

fn engine_cfg(data: &Path, out: &Path, extractor: &Path) -> EngineConfig {
    let mut c = EngineConfig {
        data_dir: data.to_path_buf(),
        default_out_dir: out.to_path_buf(),
        ..Default::default()
    };
    c.extractor_path = Some(extractor.to_path_buf());
    c.retry.base_delay = Duration::from_millis(100);
    c
}

/// Stub extractor: emits the same JSON shape yt-dlp -J produces (formats array with a
/// progressive https entry, HLS-only entry that must be rejected, adaptive halves that
/// must be rejected, and a top-level title).
///
/// Unix-only FIXTURE (`#!/bin/sh` stub) — see KNOWN-ISSUES (windows CI gate); the
/// engine behavior under test is platform-independent.
#[cfg(unix)]
#[tokio::test]
async fn stub_extractor_resolves_watch_url_to_real_stream_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();

    let server = TestServer::start(Behavior::new(512 * KIB));

    // the stub yt-dlp: ignores the URL, prints fixture metadata whose chosen stream
    // lives on the local test server
    let stub = dir.path().join("fake-yt-dlp.sh");
    let stream_url = server.url("/file");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\ncat <<'JSON'\n{{\n  \"title\": \"Sample Clip 1080p\",\n  \"formats\": [\n    {{\"format_id\": \"hls1\", \"ext\": \"mp4\", \"protocol\": \"m3u8_native\", \"vcodec\": \"h264\", \"acodec\": \"aac\", \"tbr\": 3000.0, \"url\": \"https://should-not-be-used/manifest.m3u8\"}},\n    {{\"format_id\": \"video-only\", \"ext\": \"mp4\", \"protocol\": \"https\", \"vcodec\": \"avc1\", \"acodec\": \"none\", \"tbr\": 4000.0, \"url\": \"https://should-not-be-used/v.mp4\"}},\n    {{\"format_id\": \"18\", \"ext\": \"mp4\", \"protocol\": \"https\", \"vcodec\": \"avc1.42001E\", \"acodec\": \"mp4a.40.2\", \"tbr\": 500.0, \"filesize\": {size}, \"url\": \"{url}\"}}\n  ]\n}}\nJSON\n",
            size = 512 * KIB,
            url = stream_url
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mgr = DownloadManager::new(
        engine_cfg(&data, &out, &stub),
        Arc::new(ReqwestBackend::new(&engine_cfg(&data, &out, &stub)).unwrap()),
    )
    .unwrap();

    // a real YouTube watch URL — classified as Media, routed to the extractor
    let added = mgr
        .add(
            "https://www.youtube.com/watch?v=XdNjEct5mqc",
            Default::default(),
        )
        .await
        .expect("extractor resolves the watch URL");
    assert_eq!(
        added.filename, "Sample Clip 1080p.mp4",
        "extractor title + real extension"
    );
    mgr.clone().start(added.id).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let s = loop {
        let s = mgr.status(added.id).unwrap();
        if s.state == "completed" || s.state == "failed" {
            break s;
        }
        assert!(Instant::now() < deadline, "timeout; state={}", s.state);
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(s.state, "completed", "{:?}", s.error_message);
    let path = PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert!(path.exists());
    assert_eq!(s.received, 512 * KIB as u64);
    let expected = {
        let p = dir.path().join("pattern");
        std::fs::write(&p, content_pattern(512 * KIB)).unwrap();
        compute_hash_blocking(&p, HashAlgo::Sha256).unwrap()
    };
    assert_eq!(
        compute_hash_blocking(&path, HashAlgo::Sha256).unwrap(),
        expected,
        "stream bytes must be byte-exact"
    );
    server.stop();
}

/// The un-configured extractor path fails with honest guidance (already covered at the
/// unit level in extract.rs; here through the full manager).
#[tokio::test]
async fn watch_url_without_extractor_fails_with_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let cfg = EngineConfig {
        data_dir: data.clone(),
        default_out_dir: out.clone(),
        ..Default::default()
    };
    let mgr =
        DownloadManager::new(cfg.clone(), Arc::new(ReqwestBackend::new(&cfg).unwrap())).unwrap();
    let e = mgr
        .add(
            "https://www.youtube.com/watch?v=XdNjEct5mqc",
            Default::default(),
        )
        .await
        .expect_err("must fail without an extractor");
    assert_eq!(e.kind_code(), "UNSUPPORTED_URL");
    assert!(e.message.contains("yt-dlp"));
    assert!(mgr.list().unwrap().is_empty());
}

/// Audit D2 evidence: the user's session context (cookies) must reach the extractor
/// process as --add-headers — the standard remedy for YouTube's bot wall. The stub
/// dumps its argv to a file; the test asserts the cookie is present on the extractor
/// command line.
///
/// Unix-only FIXTURE (the engine behavior is platform-independent): the stub is a
/// `#!/bin/sh` script. Windows coverage needs a stub that can dump argv and a
/// tasklist-based liveness check — tracked in KNOWN-ISSUES (windows CI gate).
#[cfg(unix)]
#[tokio::test]
async fn extractor_receives_user_cookies_as_add_headers() {
    let dir = tempfile::tempdir().unwrap();
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let server = TestServer::start(Behavior::new(64 * KIB));
    let argv_file = dir.path().join("argv.txt");

    // stub: record argv (newline-separated), then emit metadata with a local stream URL
    let stub = dir.path().join("fake-yt-dlp.sh");
    let stream_url = server.url("/file");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {argv}\ncat <<'JSON'\n{{\"title\": \"C\", \"formats\": [{{\"format_id\": \"18\", \"ext\": \"mp4\", \"protocol\": \"https\", \"vcodec\": \"avc1\", \"acodec\": \"mp4a\", \"tbr\": 500.0, \"url\": \"{url}\"}}]}}\nJSON\n",
            argv = argv_file.display(),
            url = stream_url
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut cfg = engine_cfg(&data, &out, &stub);
    cfg.extractor_timeout = Duration::from_secs(10);
    let mgr =
        DownloadManager::new(cfg.clone(), Arc::new(ReqwestBackend::new(&cfg).unwrap())).unwrap();
    let mut dl = engine_core::config::DownloadConfig::default();
    dl.cookies.insert("session".into(), "test-abc".into());
    let added = mgr
        .add("https://www.youtube.com/watch?v=fixture", dl)
        .await
        .expect("stub extractor resolves");
    mgr.clone().start(added.id).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = mgr.status(added.id).unwrap();
        if s.state == "completed" || s.state == "failed" {
            assert_eq!(s.state, "completed", "{:?}", s.error_message);
            break;
        }
        assert!(Instant::now() < deadline, "timeout; state={}", s.state);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(
        argv.contains("--add-headers"),
        "extractor must receive --add-headers; argv={argv}"
    );
    assert!(
        argv.contains("Cookie: session=test-abc"),
        "user cookie must reach the extractor process; argv={argv}"
    );
    server.stop();
}

/// Audit D1 evidence: a hung extractor (sleeps forever) must (a) surface a TIMEOUT
/// error honestly and (b) be KILLED — kill_on_drop(true) — instead of leaking an
/// orphan process per timed-out add.
///
/// Unix-only FIXTURE (`#!/bin/sh` stub + /proc PID liveness) — see the cookies test
/// note; Windows coverage tracked in KNOWN-ISSUES (windows CI gate).
#[cfg(unix)]
#[tokio::test]
async fn hung_extractor_times_out_and_the_process_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let (data, out) = (dir.path().join("data"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let pid_file = dir.path().join("pid.txt");

    // stub: record its own PID, then hang forever
    let stub = dir.path().join("sleepy-yt-dlp.sh");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\necho $$ > {pid}\nsleep 300\n",
            pid = pid_file.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut cfg = engine_cfg(&data, &out, &stub);
    cfg.extractor_timeout = Duration::from_millis(300);
    let mgr =
        DownloadManager::new(cfg.clone(), Arc::new(ReqwestBackend::new(&cfg).unwrap())).unwrap();
    let err = mgr
        .add(
            "https://www.youtube.com/watch?v=fixture",
            Default::default(),
        )
        .await
        .expect_err("hung extractor must time out");
    assert!(
        matches!(err.kind, engine_core::error::ErrorKind::Timeout),
        "honest TIMEOUT, got {:?}",
        err.kind
    );

    // the stub's PID file proves it ran; kill_on_drop must have reaped it
    let mut pid: i32 = 0;
    for _ in 0..50 {
        if let Ok(s) = std::fs::read_to_string(&pid_file) {
            if let Ok(p) = s.trim().parse::<i32>() {
                pid = p;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(pid > 0, "stub must have written its PID before hanging");
    // poll until the process is gone (SIGKILL from kill_on_drop), bounded 3 s
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
        if !alive {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "extractor process {pid} survived the timeout — orphan leak (D1 regression)"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
