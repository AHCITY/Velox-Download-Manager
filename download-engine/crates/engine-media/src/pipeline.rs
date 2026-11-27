//! Merge core: port trait, default subprocess runner, LGPL policy, ffprobe validation.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use engine_core::error::{EngineError, ErrorKind};
use engine_core::events::{Event, EventBus};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::watch;

/// Inputs for one merge. Paths only — the codec/argv policy is not caller-settable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeInput {
    pub video: PathBuf,
    pub audio: PathBuf,
    pub output: PathBuf,
    /// ffmpeg muxer name for the target container (e.g. "mp4", "matroska").
    /// The output is a `.merging.part` temp file, so the muxer cannot be
    /// inferred from the extension and MUST be passed with `-f`.
    pub format: String,
}

/// Result of one subprocess run.
#[derive(Debug, Clone)]
pub struct SubprocessOutcome {
    pub exit_ok: bool,
    /// True when the run was aborted via the kill trigger (§4); output_tail says why.
    pub cancelled: bool,
    /// Combined stdout+stderr (diagnostics for honest error messages).
    pub output_tail: String,
}

/// Port: every ffmpeg/ffprobe operation the pipeline needs, as plain command runs.
/// The default port spawns real processes (kill-safe); tests substitute a recorder.
#[async_trait]
pub trait MediaPipelinePort: Send + Sync {
    /// Run a command with the given argv; kill the child when `kill` flips to true.
    /// Dropping the returned future also kills the child (kill_on_drop).
    async fn run(
        &self,
        exe: &str,
        args: &[String],
        kill: watch::Receiver<bool>,
    ) -> Result<SubprocessOutcome, EngineError>;

    /// Version probe (`<exe> -version`), same kill semantics.
    async fn version(&self, exe: &str, kill: watch::Receiver<bool>) -> Result<String, EngineError>;
}

/// The fixed argv shape (FFMPEG_INTEGRATION.md §7). Pure function so tests pin the
/// exact contract; `-c copy` is hardcoded — callers cannot introduce transcoding.
/// ffmpeg args for MP3 extraction (yt-dlp -x parity: `-vn -q:a 0` VBR best).
/// libmp3lame is LGPL — allowed in the LGPL-gated builds Velox bundles.
pub fn build_mp3_args(input: &str, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        input.into(),
        "-vn".into(),
        "-acodec".into(),
        "libmp3lame".into(),
        "-q:a".into(),
        "0".into(),
        // the manager writes to `<final>.converting.part`: force the muxer,
        // the .part extension cannot select it
        "-f".into(),
        "mp3".into(),
        output.into(),
    ]
}

/// ffmpeg args for one poster frame (JPEG, 480 px wide, high quality).
/// `-f image2` is forced: the output may be a `.thumb.part` temp path whose
/// extension cannot select the muxer (same rule as the merge stage).
pub fn build_frame_args(input: &str, at_secs: u64, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        at_secs.to_string(),
        "-i".into(),
        input.into(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=480:-2".into(),
        "-q:v".into(),
        "4".into(),
        "-f".into(),
        "image2".into(),
        output.into(),
    ]
}

pub fn build_merge_args(input: &MergeInput) -> Vec<String> {
    let mut args = vec![
        "-y".into(),
        "-i".into(),
        input.video.display().to_string(),
        "-i".into(),
        input.audio.display().to_string(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "1:a:0?".into(),
        "-c".into(),
        "copy".into(),
        // temp output ends in .merging.part: the muxer MUST be named, or
        // ffmpeg fails with "use a standard extension for the filename"
        "-f".into(),
        input.format.clone(),
    ];
    // faststart only exists on the mov family; the matroska muxer rejects it
    if engine_core::media::accepts_movflags(&input.format) {
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push(input.output.display().to_string());
    args
}

/// LGPL policy check on `ffmpeg -version` output (audit §D procedure): any build that
/// advertises `--enable-gpl` or `--enable-nonfree` is refused. Returns the config
/// line when accepted.
pub fn verify_lgpl(version_output: &str) -> Result<String, EngineError> {
    let config = version_output
        .lines()
        .find(|l| l.contains("configuration:"))
        .unwrap_or("");
    for flag in ["--enable-gpl", "--enable-nonfree"] {
        if config.contains(flag) {
            return Err(EngineError::new(
                ErrorKind::GplBuild,
                format!(
                    "ffmpeg build is GPL-flagged ({flag}) — LGPL policy refuses it; \
                     install an LGPL build (e.g. BtbN win64-lgpl) instead"
                ),
            ));
        }
    }
    Ok(config.to_string())
}

/// ffprobe post-validation (§8): JSON with ≥1 stream and a format section, else
/// CORRUPT_MEDIA. Pure function over the probe output.
pub fn validate_probe_json(probe: &serde_json::Value) -> Result<(), EngineError> {
    let streams = probe
        .get("streams")
        .and_then(|s| s.as_array())
        .ok_or_else(|| {
            EngineError::new(
                ErrorKind::CorruptMedia,
                "ffprobe output has no streams array — output is not valid media",
            )
        })?;
    if streams.is_empty() {
        return Err(EngineError::new(
            ErrorKind::CorruptMedia,
            "ffprobe found zero streams — output is not valid media",
        ));
    }
    if probe.get("format").is_none() {
        return Err(EngineError::new(
            ErrorKind::CorruptMedia,
            "ffprobe found no format section — output is not valid media",
        ));
    }
    Ok(())
}

/// Merge pipeline: LGPL gate + merge + validation + events.
pub struct MediaPipeline<P: MediaPipelinePort> {
    port: Arc<P>,
    ffmpeg: String,
    ffprobe: String,
}

impl<P: MediaPipelinePort> std::fmt::Debug for MediaPipeline<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPipeline")
            .field("ffmpeg", &self.ffmpeg)
            .field("ffprobe", &self.ffprobe)
            .finish_non_exhaustive()
    }
}

impl<P: MediaPipelinePort> MediaPipeline<P> {
    /// Build a pipeline around explicit binaries. No discovery I/O.
    pub fn with_binaries(
        port: Arc<P>,
        ffmpeg: impl Into<String>,
        ffprobe: impl Into<String>,
    ) -> Self {
        Self {
            port,
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    /// Discover `ffmpeg`/`ffprobe` and enforce the LGPL gate up front, so a missing
    /// or GPL-flagged install fails before any download completes (honest early error).
    pub async fn discover(port: Arc<P>) -> Result<Self, EngineError> {
        Self::discover_with(port, "ffmpeg".into(), "ffprobe".into()).await
    }

    /// Discovery with explicit binaries — the bundled-beside-exe layout wins
    /// over a bare PATH lookup, which missed the installer-shipped ffmpeg and
    /// left YouTube downloads as two separate files (merge port never injected).
    /// The LGPL gate still applies to whichever ffmpeg is selected.
    pub async fn discover_with(
        port: Arc<P>,
        ffmpeg: PathBuf,
        ffprobe: PathBuf,
    ) -> Result<Self, EngineError> {
        let ff_name = ffmpeg.display().to_string();
        let fp_name = ffprobe.display().to_string();
        for exe in [ff_name.as_str(), fp_name.as_str()] {
            // Hold the sender for the whole run: dropping it closes the channel and
            // the port treats that as a cancel (kill-safe shutdown semantics).
            let (_kill_tx, kill_rx) = watch::channel(false);
            let version = port.version(exe, kill_rx).await.map_err(|_| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!(
                        "{exe} not found — install an LGPL ffmpeg build \
                                 (docs/FFMPEG_INTEGRATION.md §3)"
                    ),
                )
            })?;
            if exe == ff_name {
                verify_lgpl(&version)?;
            }
        }
        Ok(Self {
            port,
            ffmpeg: ff_name,
            ffprobe: fp_name,
        })
    }

    /// Extract ONE poster frame (JPEG) from a video file. LGPL gate + kill-safe;
    /// output may be a `.thumb.part` temp path (`-f image2` forced). `at_secs`
    /// positions the seek; 2 s skips most black intros cheaply.
    pub async fn extract_frame(
        &self,
        input: &std::path::Path,
        at_secs: u64,
        output: &std::path::Path,
        kill: watch::Receiver<bool>,
    ) -> Result<(), EngineError> {
        let (_gate_tx, gate_kill) = watch::channel(false);
        let version = self
            .port
            .version(&self.ffmpeg, gate_kill)
            .await
            .map_err(|e| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!("ffmpeg vanished before frame extraction: {}", e.message),
                )
            })?;
        verify_lgpl(&version)?;
        let args = build_frame_args(
            &input.display().to_string(),
            at_secs,
            &output.display().to_string(),
        );
        let outcome = self.port.run(&self.ffmpeg, &args, kill).await?;
        if outcome.cancelled {
            return Err(EngineError::new(ErrorKind::Cancelled, "frame extraction cancelled"));
        }
        if !outcome.exit_ok {
            return Err(EngineError::new(
                ErrorKind::MergeFailed,
                format!("ffmpeg failed to extract a frame: {}", tail(&outcome.output_tail)),
            ));
        }
        if !output.is_file() {
            return Err(EngineError::new(
                ErrorKind::MergeFailed,
                "ffmpeg reported success but the frame file is missing",
            ));
        }
        Ok(())
    }

    /// Merge `input` (stream copy), validate with ffprobe, emit `Merging`/`Merged`.
    /// On cancel the child is killed and `Cancelled` is returned (§4).
    /// Convert one input file to MP3 (LGPL gate + kill-safe + output check).
    pub async fn convert_mp3(
        &self,
        input: &std::path::Path,
        output: &std::path::Path,
        kill: watch::Receiver<bool>,
    ) -> Result<(), EngineError> {
        // LGPL gate on every conversion (a swapped binary between runs must be caught).
        let (_gate_tx, gate_kill) = watch::channel(false);
        let version = self
            .port
            .version(&self.ffmpeg, gate_kill)
            .await
            .map_err(|e| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!("ffmpeg vanished before conversion: {}", e.message),
                )
            })?;
        verify_lgpl(&version)?;
        let args = build_mp3_args(&input.display().to_string(), &output.display().to_string());
        let outcome = self.port.run(&self.ffmpeg, &args, kill).await?;
        if outcome.cancelled {
            return Err(EngineError::new(
                ErrorKind::Cancelled,
                "conversion cancelled",
            ));
        }
        if !outcome.exit_ok {
            return Err(EngineError::new(
                ErrorKind::MergeFailed,
                "ffmpeg failed to extract MP3 (see the download's error history)",
            ));
        }
        if !output.is_file() {
            return Err(EngineError::new(
                ErrorKind::MergeFailed,
                "ffmpeg reported success but the MP3 file is missing",
            ));
        }
        Ok(())
    }

    pub async fn merge(
        &self,
        id: i64,
        input: &MergeInput,
        bus: &EventBus,
        kill: watch::Receiver<bool>,
    ) -> Result<PathBuf, EngineError> {
        let _ = bus.send(Event::Merging { id });

        // LGPL gate on every merge (a swapped binary between runs must be caught).
        let (_gate_tx, gate_kill) = watch::channel(false);
        let version = self
            .port
            .version(&self.ffmpeg, gate_kill)
            .await
            .map_err(|e| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!("ffmpeg vanished before merge: {}", e.message),
                )
            })?;
        verify_lgpl(&version)?;

        let args = build_merge_args(input);
        let outcome = self.port.run(&self.ffmpeg, &args, kill).await?;
        if outcome.cancelled {
            return Err(EngineError::new(ErrorKind::Cancelled, "merge cancelled"));
        }
        if !outcome.exit_ok {
            return Err(EngineError::new(
                ErrorKind::MergeFailed,
                format!("ffmpeg failed: {}", tail(&outcome.output_tail)),
            ));
        }

        let (_probe_tx, probe_kill) = watch::channel(false);
        let probe_args = vec![
            "-v".into(),
            "error".into(),
            "-print_format".into(),
            "json".into(),
            "-show_format".into(),
            "-show_streams".into(),
            input.output.display().to_string(),
        ];
        let probe = self
            .port
            .run(&self.ffprobe, &probe_args, probe_kill)
            .await?;
        let json: serde_json::Value = if probe.exit_ok {
            serde_json::from_str(probe.output_tail.trim()).map_err(|e| {
                EngineError::new(
                    ErrorKind::CorruptMedia,
                    format!("ffprobe output unparseable ({e}) — output is not valid media"),
                )
            })?
        } else {
            return Err(EngineError::new(
                ErrorKind::CorruptMedia,
                format!(
                    "ffprobe rejected the merged file: {}",
                    tail(&probe.output_tail)
                ),
            ));
        };
        validate_probe_json(&json)?;

        let path = input.output.clone();
        let _ = bus.send(Event::Merged {
            id,
            path: path.display().to_string(),
        });
        Ok(path)
    }
}

fn tail(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 400 {
        s.to_string()
    } else {
        let skip = s.chars().count() - 400;
        s.chars().skip(skip).collect()
    }
}

/// Default port: real subprocesses. Kill-safe by construction — `kill_on_drop(true)`
/// plus an explicit select on the trigger (FFMPEG_INTEGRATION.md §4). Pipe drains run
/// as tasks so output is captured even when the child outlives a momentary block.
#[derive(Debug, Default, Clone, Copy)]
pub struct SubprocessPort;

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut r: R) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf).await;
    buf
}

#[async_trait]
impl MediaPipelinePort for SubprocessPort {
    async fn run(
        &self,
        exe: &str,
        args: &[String],
        kill: watch::Receiver<bool>,
    ) -> Result<SubprocessOutcome, EngineError> {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Windows: ffmpeg/ffprobe are console executables — spawned from the GUI app
        // during a merge they would flash console windows mid-download. Suppress the
        // window; stdout/stderr stay piped (diagnostics preserved in output_tail).
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| {
            EngineError::new(
                ErrorKind::MergeFailed,
                format!("could not spawn {exe}: {e}"),
            )
        })?;

        let out_task = tokio::spawn(drain(child.stdout.take().expect("stdout piped")));
        let err_task = tokio::spawn(drain(child.stderr.take().expect("stderr piped")));

        let mut kill = kill;
        // Two-phase select: handlers must not await — the wait_for future (holding an
        // internal RwLockReadGuard) is still alive during handler execution, and a
        // guard crossing an await would make this future !Send. Set a flag here, do
        // the kill+reap after the select completes.
        let mut triggered = false;
        // wait_for fires on an explicit `true` AND on channel close (all senders
        // dropped) — both mean "nobody wants this child alive": kill it (§4).
        let status = tokio::select! {
            s = child.wait() => Some(s),
            _ = kill.wait_for(|v| *v) => {
                triggered = true;
                None
            }
        };
        let cancelled = triggered;
        let status = if triggered {
            let _ = child.start_kill();
            child.wait().await
        } else {
            status.expect("non-cancelled select sets Some(status)")
        };
        let out = out_task.await.unwrap_or_default();
        let err = err_task.await.unwrap_or_default();

        if cancelled {
            return Ok(SubprocessOutcome {
                exit_ok: false,
                cancelled: true,
                output_tail: "merge killed on cancel trigger".into(),
            });
        }
        let exit_ok = status
            .map_err(|e| EngineError::new(ErrorKind::MergeFailed, format!("wait failed: {e}")))?
            .success();
        let mut combined = String::from_utf8_lossy(&out).into_owned();
        combined.push_str(&String::from_utf8_lossy(&err));
        Ok(SubprocessOutcome {
            exit_ok,
            cancelled: false,
            output_tail: combined,
        })
    }

    async fn version(&self, exe: &str, kill: watch::Receiver<bool>) -> Result<String, EngineError> {
        Ok(self
            .run(exe, &["-version".to_string()], kill)
            .await?
            .output_tail)
    }
}
