//! P2: true out-of-process crash recovery. The real `velox` binary runs as a child process and
//! is killed with SIGKILL at several crash points; a *fresh* process must resume safely.
//!
//! Invariants checked after every kill:
//!   1. every segment position claimed by the SQLite manifest exists in the part file
//!      (fsync-before-commit, ADR-0007 — the manifest is a durable LOWER bound);
//!   2. `downloads.received` equals Σ segment positions (no aggregate drift);
//!   3. after resume in a new process, the final file is byte-exact (SHA-256).
//!
//! Crash points exercised by the kill timings: during segment writes, across checkpoint
//! commits (received vs file sampled at different phases), during resume (double-kill), and
//! before any checkpoint at all. The rename-orphan case is simulated explicitly (scenario D).

use std::path::{Path, PathBuf};
use std::time::Duration;

use engine_core::config::HashAlgo;
use engine_core::task::compute_hash_blocking;
use engine_state::Db;
use engine_testserver::{content_pattern, Behavior, TestServer};

const MIB: usize = 1024 * 1024;
const SIZE: usize = 20 * MIB; // default min_segment 8 MiB → 3 segments → segmented by default

fn expected_hash(len: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("pattern");
    std::fs::write(&p, content_pattern(len)).unwrap();
    compute_hash_blocking(&p, HashAlgo::Sha256).unwrap()
}

fn velox() -> &'static str {
    env!("CARGO_BIN_EXE_velox")
}

async fn run_velox(args: &[&str], home: &Path, timeout: Duration) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(velox());
    cmd.args(args)
        .env("DXDL_HOME", home)
        .env("RUST_LOG", "error")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .expect("velox command timed out")
        .expect("spawn velox");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn db(home: &Path) -> Db {
    Db::open(&home.join("state.db")).expect("open manifest")
}

/// Invariant 1+2: manifest positions are a durable lower bound of what is on disk.
fn check_manifest_consistency(home: &Path) {
    let d = db(home);
    let row = d
        .list_downloads()
        .expect("list")
        .into_iter()
        .next()
        .expect("row");
    let file_len = std::fs::metadata(row.temp_path.as_deref().unwrap_or(""))
        .map(|m| m.len())
        .unwrap_or(0);
    let segs = d.segments(row.id).expect("segments");
    assert!(!segs.is_empty(), "segments planned");
    let mut sum = 0i64;
    for s in &segs {
        assert!(s.pos >= 0);
        let claimed = s.start_off + s.pos;
        if row.state == "completed" {
            // after completion the file is renamed; nothing more to check here
            continue;
        }
        assert!(
            claimed <= file_len as i64,
            "manifest claims bytes {}..{} but the part file has only {file_len} bytes \
             (fsync-before-commit violated)",
            s.start_off,
            claimed
        );
        sum += s.pos;
    }
    assert_eq!(
        row.received, sum,
        "downloads.received must equal Σ segment positions"
    );
}

async fn wait_part_bytes(part: &Path, min: u64, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        let len = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if len >= min {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "part file never reached {min} bytes (at {len})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn spawn_download(server_url: &str, out: &Path, home: &Path) -> tokio::process::Child {
    tokio::process::Command::new(velox())
        .args([
            "download",
            server_url,
            "-o",
            out.to_str().unwrap(),
            "--name",
            "crash-test.bin",
            "--no-preallocate",
        ])
        .env("DXDL_HOME", home)
        .env("RUST_LOG", "error")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn velox download")
}

async fn assert_final_hash(out: &Path, name: &str) {
    let final_path = out.join(name);
    assert!(final_path.exists(), "final file missing after resume");
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(SIZE),
        "recovered file is not byte-exact"
    );
}

#[tokio::test]
async fn sigkill_mid_download_resume_in_fresh_process_is_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let (home, out) = (dir.path().join("home"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let mut b = Behavior::new(SIZE);
    b.slow_bps = 4 * MIB as u64; // ~2.5s across 3 segments; deterministic kill window
    let server = TestServer::start(b);

    let mut child = spawn_download(&server.url("/file"), &out, &home);
    let part = {
        // wait for the manifest row, then for real progress
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let part = loop {
            let d = db(&home);
            if let Some(row) = d.list_downloads().unwrap().first() {
                break PathBuf::from(row.temp_path.clone().unwrap());
            }
            assert!(std::time::Instant::now() < deadline, "row never created");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        tokio::time::sleep(Duration::from_millis(300)).await; // let it start writing
        part
    };
    wait_part_bytes(&part, 4 * MIB as u64, Duration::from_secs(60)).await;
    child.kill().await.expect("SIGKILL");
    let _ = child.wait().await;

    check_manifest_consistency(&home);
    let id = db(&home).list_downloads().unwrap().first().unwrap().id;
    let (code, so, se) = run_velox(
        &["resume", &id.to_string()],
        &home,
        Duration::from_secs(180),
    )
    .await;
    assert_eq!(code, 0, "resume failed: stdout={so} stderr={se}");
    assert_final_hash(&out, "crash-test.bin").await;
    server.stop();
}

#[tokio::test]
async fn sigkill_during_resume_then_resume_again_is_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let (home, out) = (dir.path().join("home"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let mut b = Behavior::new(SIZE);
    b.slow_bps = 4 * MIB as u64;
    let server = TestServer::start(b);

    // first process: download and kill mid-way
    let mut child = spawn_download(&server.url("/file"), &out, &home);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let part = loop {
        let d = db(&home);
        if let Some(row) = d.list_downloads().unwrap().first() {
            break PathBuf::from(row.temp_path.clone().unwrap());
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    wait_part_bytes(&part, 3 * MIB as u64, Duration::from_secs(60)).await;
    child.kill().await.unwrap();
    let _ = child.wait().await;
    check_manifest_consistency(&home);

    // second process: resume and kill DURING the resume
    let id = db(&home).list_downloads().unwrap().first().unwrap().id;
    let mut child2 = tokio::process::Command::new(velox())
        .args(["resume", &id.to_string()])
        .env("DXDL_HOME", &home)
        .env("RUST_LOG", "error")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    child2.kill().await.unwrap();
    let _ = child2.wait().await;
    check_manifest_consistency(&home);

    // third process: finish
    let (code, so, se) = run_velox(
        &["resume", &id.to_string()],
        &home,
        Duration::from_secs(180),
    )
    .await;
    assert_eq!(code, 0, "second resume failed: stdout={so} stderr={se}");
    assert_final_hash(&out, "crash-test.bin").await;
    server.stop();
}

#[tokio::test]
async fn kill_before_first_checkpoint_resume_restarts_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let (home, out) = (dir.path().join("home"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let mut b = Behavior::new(2 * MIB);
    b.slow_bps = 4 * MIB as u64;
    let server = TestServer::start(b);

    let mut child = spawn_download(&server.url("/file"), &out, &home);
    tokio::time::sleep(Duration::from_millis(150)).await; // likely before any durable checkpoint
    child.kill().await.unwrap();
    let _ = child.wait().await;
    check_manifest_consistency(&home);

    let id = db(&home).list_downloads().unwrap().first().unwrap().id;
    let (code, so, se) = run_velox(
        &["resume", &id.to_string()],
        &home,
        Duration::from_secs(120),
    )
    .await;
    assert_eq!(code, 0, "resume failed: stdout={so} stderr={se}");
    let final_path = out.join("crash-test.bin");
    assert!(final_path.exists());
    assert_eq!(
        compute_hash_blocking(&final_path, HashAlgo::Sha256).unwrap(),
        expected_hash(2 * MIB)
    );
    server.stop();
}

#[tokio::test]
async fn rename_orphan_final_file_is_never_adopted_without_evidence() {
    // Crash window "after rename, before DB commit": the part file is already gone (renamed),
    // the manifest still says downloading. The engine must NOT trust the orphan; it re-downloads
    // and never exposes unverified bytes as a completed download.
    let dir = tempfile::tempdir().unwrap();
    let (home, out) = (dir.path().join("home"), dir.path().join("out"));
    std::fs::create_dir_all(&out).unwrap();
    let mut b = Behavior::new(2 * MIB);
    b.slow_bps = 4 * MIB as u64;
    let server = TestServer::start(b);

    let mut child = spawn_download(&server.url("/file"), &out, &home);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let part = loop {
        let d = db(&home);
        if let Some(row) = d.list_downloads().unwrap().first() {
            break PathBuf::from(row.temp_path.clone().unwrap());
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    wait_part_bytes(&part, MIB as u64, Duration::from_secs(60)).await;
    child.kill().await.unwrap();
    let _ = child.wait().await;

    // simulate the orphan: part file "renamed away" into the final position
    let orphan = out.join("crash-test.bin");
    std::fs::rename(&part, &orphan).unwrap();
    assert!(!part.exists());

    let id = db(&home).list_downloads().unwrap().first().unwrap().id;
    let (code, so, se) = run_velox(
        &["resume", &id.to_string()],
        &home,
        Duration::from_secs(180),
    )
    .await;
    assert_eq!(code, 0, "resume failed: stdout={so} stderr={se}");
    // the engine re-downloaded to a collision-safe name; both files exist and the new one is exact
    let files: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert!(
        files.len() >= 2,
        "orphan kept + fresh download expected: {files:?}"
    );
    let fresh = files
        .iter()
        .find(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n != "crash-test.bin" && !n.ends_with(".lock") && !n.contains(".part-")
        })
        .expect("fresh file");
    assert_eq!(
        compute_hash_blocking(fresh, HashAlgo::Sha256).unwrap(),
        expected_hash(2 * MIB),
        "the re-downloaded file must be byte-exact"
    );
    server.stop();
}
