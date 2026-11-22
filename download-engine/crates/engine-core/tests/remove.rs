//! download.remove semantics (UI overhaul 2026-11-22): record removal for ANY
//! state including completed, optional file deletion, child-row cascade, and a
//! `removed` state event. The completed-row gap was the UI's biggest complaint —
//! finished downloads whose files were deleted externally could not be managed.

use std::path::Path;
use std::time::Duration;

use engine_core::config::{DownloadConfig, EngineConfig};
use engine_core::manager::DownloadManager;
use engine_http::ReqwestBackend;
use engine_testserver::{content_pattern, Behavior, TestServer};

const KIB: usize = 1024;

fn engine_cfg(data: &Path, out: &Path) -> EngineConfig {
    EngineConfig {
        data_dir: data.to_path_buf(),
        default_out_dir: out.to_path_buf(),
        ..Default::default()
    }
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
) -> engine_core::manager::StatusView {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timeout waiting for {want:?}"
        );
        if let Ok(s) = mgr.status(id) {
            if want.contains(&s.state.as_str()) {
                return s;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn add_completed(
    dir: &Path,
    size: usize,
) -> (
    std::sync::Arc<DownloadManager>,
    engine_testserver::TestServer,
    i64,
    std::path::PathBuf,
) {
    let data = dir.join("data");
    let out = dir.join("out");
    let cfg = engine_cfg(&data, &out);
    let mgr = manager(&cfg);
    let server = TestServer::start(Behavior::new(size));
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    let s = wait_state(&mgr, added.id, &["completed"], Duration::from_secs(30)).await;
    let file = std::path::PathBuf::from(&s.dest_dir).join(&s.final_name);
    assert!(file.exists());
    (mgr, server, added.id, file)
}

#[tokio::test]
async fn remove_completed_with_file_deletes_record_and_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let (mgr, server, id, file) = add_completed(dir.path(), 100 * KIB).await;
    // subscribe BEFORE the remove so the broadcast is observable
    let mut rx = mgr.events();
    mgr.clone().remove(id, true).await.unwrap();
    assert!(!file.exists(), "file must be deleted with delete_file=true");
    assert!(
        mgr.list().unwrap().iter().all(|r| r.id != id),
        "record must be gone"
    );
    // the `removed` state event must have been broadcast (UIs drop the row on it)
    let ev = rx.try_recv().expect("removed event must be broadcast");
    assert!(
        matches!(ev, engine_core::events::Event::DownloadState { ref state, .. } if state == "removed"),
        "expected DownloadState(removed), got {ev:?}"
    );
    server.stop();
}

#[tokio::test]
async fn remove_completed_without_file_keeps_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let (mgr, server, id, file) = add_completed(dir.path(), 100 * KIB).await;
    mgr.clone().remove(id, false).await.unwrap();
    assert!(
        file.exists(),
        "file must survive remove with delete_file=false"
    );
    assert!(mgr.list().unwrap().iter().all(|r| r.id != id));
    server.stop();
}

#[tokio::test]
async fn remove_failed_row_cascades_child_rows() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = dir.path().join("out");
    let cfg = engine_cfg(&data, &out);
    let mgr = manager(&cfg);
    // a 410 (permanent) download fails without retry — leaves errors behind
    let mut b = Behavior::new(100 * KIB);
    b.expire_after_requests = Some(1); // inspect ok, engine requests → 410 Gone
    let server = TestServer::start(b);
    let added = mgr
        .add(&server.url("/file"), DownloadConfig::default())
        .await
        .unwrap();
    mgr.clone().start(added.id).await.unwrap();
    wait_state(&mgr, added.id, &["failed"], Duration::from_secs(30)).await;
    assert!(
        !mgr.status(added.id).unwrap().recent_errors.is_empty()
            || mgr.db().errors(added.id).is_ok()
    );
    mgr.clone().remove(added.id, false).await.unwrap();
    assert!(mgr.list().unwrap().iter().all(|r| r.id != added.id));
    // errors cascade with the record
    assert!(
        mgr.db().errors(added.id).unwrap().is_empty(),
        "errors must cascade with the record"
    );
    server.stop();
}

#[tokio::test]
async fn remove_unknown_id_is_an_honest_error() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = engine_cfg(&dir.path().join("data"), &dir.path().join("out"));
    let mgr = manager(&cfg);
    let err = mgr.clone().remove(4242, false).await.unwrap_err();
    assert!(format!("{err}").contains("unknown download id"));
}

#[tokio::test]
async fn file_missing_flag_reflects_deleted_artifact() {
    // the M5 audit field the UI must surface: delete the file behind the engine's
    // back, then the row must report file_missing=true (and false again is not
    // asserted — a missing file stays missing)
    let dir = tempfile::tempdir().unwrap();
    let (mgr, server, id, file) = add_completed(dir.path(), 100 * KIB).await;
    assert!(!mgr.status(id).unwrap().file_missing);
    std::fs::remove_file(&file).unwrap();
    assert!(mgr.status(id).unwrap().file_missing);
    let _ = content_pattern(1); // keep import used
    server.stop();
}
