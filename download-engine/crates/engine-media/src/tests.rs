//! Port-faked tests for the merge core + a real-process kill-safety test (§9).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use engine_core::error::{EngineError, ErrorKind};
use engine_core::events::{make_bus, Event};
use tokio::sync::watch;

use crate::pipeline::{
    build_merge_args, merge_output_format, validate_probe_json, verify_lgpl, MediaPipeline,
    MediaPipelinePort, MergeInput, SubprocessOutcome, SubprocessPort,
};

fn input() -> MergeInput {
    MergeInput {
        video: PathBuf::from("/dl/video.m4s"),
        audio: PathBuf::from("/dl/audio.m4s"),
        output: PathBuf::from("/dl/out.mp4"),
    }
}

/// Recorder port: version returns a canned string; run returns canned outcomes —
/// `merge_out` for the ffmpeg call, `probe_out` for the ffprobe call (they fail
/// independently).
struct FakePort {
    version_out: Mutex<String>,
    merge_out: Mutex<SubprocessOutcome>,
    probe_out: Mutex<SubprocessOutcome>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl Default for FakePort {
    fn default() -> Self {
        Self {
            version_out: Mutex::new(String::new()),
            merge_out: Mutex::new(SubprocessOutcome {
                exit_ok: false,
                cancelled: false,
                output_tail: String::new(),
            }),
            probe_out: Mutex::new(SubprocessOutcome {
                exit_ok: false,
                cancelled: false,
                output_tail: String::new(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FakePort {
    fn ok() -> Arc<Self> {
        let p = Self::default();
        *p.version_out.lock().unwrap() =
            "ffmpeg version 7.1\nconfiguration: --enable-lgpl --disable-gpl".into();
        *p.merge_out.lock().unwrap() = SubprocessOutcome {
            exit_ok: true,
            cancelled: false,
            output_tail: String::new(),
        };
        *p.probe_out.lock().unwrap() = SubprocessOutcome {
            exit_ok: true,
            cancelled: false,
            output_tail: PROBE_OK.into(),
        };
        Arc::new(p)
    }
}

const PROBE_OK: &str = r#"{"streams":[{"codec_name":"aac"}],"format":{"duration":"12.5"}}"#;

#[async_trait::async_trait]
impl MediaPipelinePort for FakePort {
    async fn run(
        &self,
        exe: &str,
        args: &[String],
        _kill: watch::Receiver<bool>,
    ) -> Result<SubprocessOutcome, EngineError> {
        self.calls
            .lock()
            .unwrap()
            .push((exe.to_string(), args.to_vec()));
        if exe == "ffprobe" {
            return Ok(self.probe_out.lock().unwrap().clone());
        }
        Ok(self.merge_out.lock().unwrap().clone())
    }

    async fn version(
        &self,
        _exe: &str,
        _kill: watch::Receiver<bool>,
    ) -> Result<String, EngineError> {
        Ok(self.version_out.lock().unwrap().clone())
    }
}

#[test]
fn merge_args_are_pinned_copy_only() {
    let args = build_merge_args(&input());
    let joined = args.join(" ");
    assert!(args.contains(&"-c".to_string()) && args.contains(&"copy".to_string()));
    assert!(joined.contains("-movflags +faststart"));
    assert!(joined.contains("0:v:0"));
    assert!(joined.contains("1:a:0?"));
    for banned in ["libx264", "libx265", "-crf", "-preset", "--enable"] {
        assert!(!joined.contains(banned), "transcode/flag leak: {banned}");
    }
    assert_eq!(args.last().unwrap(), "/dl/out.mp4");
    // the merge runs against a crash-safe `.merging.part` temp file whose
    // extension ffmpeg cannot map to a muxer — the explicit format fixes it
    assert!(
        joined.contains("-f mp4"),
        "explicit muxer for the temp output: {joined}"
    );
}

#[test]
fn merge_output_format_covers_the_containers_the_extractor_chooses() {
    use std::path::Path;
    assert_eq!(merge_output_format(Path::new("/dl/Title.mp4")), Some("mp4"));
    assert_eq!(merge_output_format(Path::new("/dl/Title.m4v")), Some("mp4"));
    assert_eq!(
        merge_output_format(Path::new("/dl/Title.mkv")),
        Some("matroska")
    );
    assert_eq!(
        merge_output_format(Path::new("/dl/Title.webm")),
        Some("webm")
    );
    assert_eq!(merge_output_format(Path::new("/dl/Title.bin")), None);
}

#[test]
fn merge_output_format_sees_through_the_merging_part_temp_suffix() {
    // THE regression: Path::extension() of the crash-safe temp name is "part"
    // — the muxer must come from the real container extension beneath it.
    use std::path::Path;
    assert_eq!(
        merge_output_format(Path::new("/dl/Title.mp4.merging.part")),
        Some("mp4")
    );
    assert_eq!(
        merge_output_format(Path::new("/dl/Title.mkv.merging.part")),
        Some("matroska")
    );
    assert_eq!(
        merge_output_format(Path::new("/dl/Title.webm.merging.part")),
        Some("webm")
    );
}

#[test]
fn merge_args_carry_the_muxer_for_the_real_temp_file_shape() {
    let input = MergeInput {
        output: PathBuf::from("/dl/Title.mp4.merging.part"),
        ..input()
    };
    let args = build_merge_args(&input);
    let joined = args.join(" ");
    assert!(
        joined.contains("-f mp4"),
        "muxer must be derived from the container beneath .merging.part: {joined}"
    );
}

#[test]
fn lgpl_gate_accepts_clean_and_rejects_gpl() {
    let ok = "ffmpeg version 7.1\nconfiguration: --enable-version3 --disable-gpl";
    assert!(verify_lgpl(ok).is_ok());
    let gpl = "ffmpeg version 7.1\nconfiguration: --enable-gpl --enable-libx264";
    let err = verify_lgpl(gpl).unwrap_err();
    assert_eq!(err.kind, ErrorKind::GplBuild);
    assert!(err.message.contains("--enable-gpl"));
    let nonfree = "configuration: --enable-nonfree";
    assert_eq!(verify_lgpl(nonfree).unwrap_err().kind, ErrorKind::GplBuild);
    // No configuration line at all: accepted (can't prove GPL), merge-time gate re-checks.
    assert!(verify_lgpl("ffmpeg version 7.1").is_ok());
}

#[test]
fn probe_validation_honest_paths() {
    assert!(validate_probe_json(&serde_json::from_str(PROBE_OK).unwrap()).is_ok());
    let empty = serde_json::json!({"streams": [], "format": {}});
    assert_eq!(
        validate_probe_json(&empty).unwrap_err().kind,
        ErrorKind::CorruptMedia
    );
    let no_format = serde_json::json!({"streams": [{"codec_name": "h264"}]});
    assert_eq!(
        validate_probe_json(&no_format).unwrap_err().kind,
        ErrorKind::CorruptMedia
    );
    let no_streams_key = serde_json::json!({"format": {}});
    assert_eq!(
        validate_probe_json(&no_streams_key).unwrap_err().kind,
        ErrorKind::CorruptMedia
    );
}

#[tokio::test]
async fn full_merge_emits_merging_and_merged() {
    // FakePort::ok() presets: merge exits clean, ffprobe returns PROBE_OK.
    let port = FakePort::ok();
    let pipeline = MediaPipeline::with_binaries(port, "ffmpeg", "ffprobe");
    let (bus, mut rx) = make_bus(16);
    let (_, kill) = watch::channel(false);
    let path = pipeline
        .merge(7, &input(), &bus, kill)
        .await
        .expect("merge ok");
    assert_eq!(path, PathBuf::from("/dl/out.mp4"));
    let mut saw_merging = false;
    let mut saw_merged = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            Event::Merging { id } => {
                assert_eq!(id, 7);
                saw_merging = true;
            }
            Event::Merged { id, path } => {
                assert_eq!(id, 7);
                assert_eq!(path, "/dl/out.mp4");
                saw_merged = true;
            }
            _ => {}
        }
    }
    assert!(saw_merging && saw_merged, "events missing");
}

#[tokio::test]
async fn ffmpeg_failure_is_honest_merge_failed() {
    let port = FakePort::ok();
    *port.merge_out.lock().unwrap() = SubprocessOutcome {
        exit_ok: false,
        cancelled: false,
        output_tail: "Invalid data found when handling input".into(),
    };
    let pipeline = MediaPipeline::with_binaries(port, "ffmpeg", "ffprobe");
    let (bus, _rx) = make_bus(16);
    let (_, kill) = watch::channel(false);
    let err = pipeline.merge(1, &input(), &bus, kill).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::MergeFailed);
    assert!(err.message.contains("Invalid data found"));
}

#[tokio::test]
async fn ffprobe_rejection_is_corrupt_media() {
    let port = FakePort::ok();
    // Merge succeeds; ffprobe itself fails (exit nonzero) → CorruptMedia with its stderr.
    *port.probe_out.lock().unwrap() = SubprocessOutcome {
        exit_ok: false,
        cancelled: false,
        output_tail: "moov atom not found".into(),
    };
    let pipeline = MediaPipeline::with_binaries(port, "ffmpeg", "ffprobe");
    let (bus, _rx) = make_bus(16);
    let (_, kill) = watch::channel(false);
    let err = pipeline.merge(2, &input(), &bus, kill).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::CorruptMedia);
    assert!(err.message.contains("moov atom not found"));
}

#[tokio::test]
async fn gpl_swapped_binary_caught_at_merge_time() {
    // BUNDLED pipeline (Enforce): a binary swapped in after discovery — or any
    // GPL-flagged build we distribute — is refused again at merge time.
    let port = FakePort::ok();
    *port.version_out.lock().unwrap() =
        "ffmpeg version 7.1\nconfiguration: --enable-gpl --enable-libx264".into();
    let pipeline =
        MediaPipeline::with_binaries_gated(port, "ffmpeg", "ffprobe", crate::LicenseGate::Enforce);
    let (bus, _rx) = make_bus(16);
    let (_, kill) = watch::channel(false);
    let err = pipeline.merge(3, &input(), &bus, kill).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::GplBuild);
}

#[tokio::test]
async fn cancel_trigger_beats_completion() {
    let port = FakePort::ok();
    // Simulate a long-running merge that gets cancelled.
    *port.merge_out.lock().unwrap() = SubprocessOutcome {
        exit_ok: false,
        cancelled: true,
        output_tail: "merge killed on cancel trigger".into(),
    };
    let pipeline = MediaPipeline::with_binaries(port, "ffmpeg", "ffprobe");
    let (bus, _rx) = make_bus(16);
    let (kill_tx, kill_rx) = watch::channel(false);
    kill_tx.send(true).unwrap();
    let err = pipeline
        .merge(4, &input(), &bus, kill_rx)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
}

/// Real-process kill-safety (§9): the default port kills a live `sleep` on the trigger.
#[tokio::test]
async fn subprocess_port_kills_live_child_on_trigger() {
    let port = SubprocessPort;
    let (kill_tx, kill_rx) = watch::channel(false);
    // Fire the trigger 100 ms in, while `sleep 30` is still running.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = kill_tx.send(true);
    });
    let started = std::time::Instant::now();
    let outcome = port
        .run("sleep", &["30".to_string()], kill_rx)
        .await
        .expect("spawn sleep ok");
    let elapsed = started.elapsed();
    assert!(outcome.cancelled, "expected cancelled outcome");
    assert!(!outcome.exit_ok);
    // A full `sleep 30` would take 30s; the kill path must end the run in ≤5s.
    assert!(elapsed.as_secs() < 5, "kill took {elapsed:?}");
    // No trigger (sender held alive): the child exits cleanly.
    let (_kill_tx2, kill_rx2) = watch::channel(false);
    let out = port
        .run("sleep", &["0".to_string()], kill_rx2)
        .await
        .unwrap();
    assert!(
        out.exit_ok && !out.cancelled,
        "sleep 0 should exit clean: {out:?}"
    );
}

/// Discovery: missing binaries produce honest MISSING_FFMPEG (fake port that always errs).
struct DeadPort;
#[async_trait::async_trait]
impl MediaPipelinePort for DeadPort {
    async fn run(
        &self,
        _exe: &str,
        _args: &[String],
        _kill: watch::Receiver<bool>,
    ) -> Result<SubprocessOutcome, EngineError> {
        Err(EngineError::new(ErrorKind::Io, "spawn failed"))
    }
    async fn version(
        &self,
        _exe: &str,
        _kill: watch::Receiver<bool>,
    ) -> Result<String, EngineError> {
        Err(EngineError::new(ErrorKind::Io, "spawn failed"))
    }
}

#[tokio::test]
async fn discovery_missing_binary_is_honest() {
    let err = MediaPipeline::<DeadPort>::discover(Arc::new(DeadPort))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::MissingFfmpeg);
    assert!(err.message.contains("ffmpeg not found"));
}

/// YouTube two-files fix #2 regression: a USER-installed GPL-flagged ffmpeg
/// (winget/gyan/distro builds) must still be accepted — Velox only invokes it,
/// it does not distribute it. The bundled layout keeps the gate.
struct GplPort;

#[async_trait::async_trait]
impl MediaPipelinePort for GplPort {
    async fn run(
        &self,
        _exe: &str,
        _args: &[String],
        _kill: watch::Receiver<bool>,
    ) -> Result<SubprocessOutcome, EngineError> {
        Ok(SubprocessOutcome {
            exit_ok: true,
            cancelled: false,
            output_tail: String::new(),
        })
    }

    async fn version(
        &self,
        _exe: &str,
        _kill: watch::Receiver<bool>,
    ) -> Result<String, EngineError> {
        Ok("ffmpeg version 7.1\nconfiguration: --enable-gpl --enable-libx264".into())
    }
}

#[tokio::test]
async fn user_path_ffmpeg_with_gpl_flags_is_accepted() {
    // PATH-resolved binaries (bare names) use LicenseGate::Skip: a GPL-flagged
    // user install is invoked, not distributed, so merging must proceed.
    let p = MediaPipeline::<GplPort>::discover_with_policy(
        Arc::new(GplPort),
        PathBuf::from("ffmpeg"),
        PathBuf::from("ffprobe"),
        crate::LicenseGate::Skip,
    )
    .await
    .expect("user-installed GPL ffmpeg must be usable for merging");
    let _ = p;
}

#[tokio::test]
async fn bundled_ffmpeg_with_gpl_flags_is_refused() {
    // a "bundled" dir whose fake ffmpeg advertises --enable-gpl → Enforce gate
    let dir = tempfile::tempdir().unwrap();
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let probe = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    std::fs::write(dir.path().join(exe), "not a real binary").unwrap();
    std::fs::write(dir.path().join(probe), "not a real binary").unwrap();
    // GplPort answers the version probe regardless of the fake file bytes.
    let err = MediaPipeline::<GplPort>::discover_with_policy(
        Arc::new(GplPort),
        dir.path().join(exe),
        dir.path().join(probe),
        crate::LicenseGate::Enforce,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::GplBuild);
}
