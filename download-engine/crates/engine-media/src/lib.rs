//! MediaPipeline merge stage (architecture review §9, docs/FFMPEG_INTEGRATION.md).
//!
//! Manager-level wrapper job — NOT part of the transfer engine. FFmpeg runs as a
//! subprocess only, stream-copy (`-c copy`) only, kill-safe, LGPL-policy enforced,
//! ffprobe post-validated, honest errors. All subprocess operations go through the
//! `MediaPipelinePort` port so tests are deterministic without real binaries.

pub mod pipeline;

pub use pipeline::{
    build_merge_args, LicenseGate, MediaPipeline, MediaPipelinePort, MergeInput, SubprocessOutcome,
    SubprocessPort,
};

/// Manager-facing adapter (roadmap 4 slice 2): implements engine-core's
/// `MediaMergePort` by delegating to `MediaPipeline`. Injected via
/// `DownloadManager::set_media_merge` — the transfer engine never links ffmpeg.
pub struct MergePort<P: MediaPipelinePort> {
    pipeline: MediaPipeline<P>,
}

impl<P: MediaPipelinePort> MergePort<P> {
    pub fn new(pipeline: MediaPipeline<P>) -> Self {
        Self { pipeline }
    }
}

impl MergePort<SubprocessPort> {
    /// Real subprocess pipeline with PATH binaries, LGPL-gated at discovery
    /// (honest early `MISSING_FFMPEG`/`GPL_BUILD` before any download completes).
    pub async fn discover_subprocess() -> Result<Self, engine_core::error::EngineError> {
        Self::discover_subprocess_at(None).await
    }

    /// Prefer ffmpeg/ffprobe BUNDLED beside the app executable (the unified
    /// installer ships them next to Velox.exe), falling back to PATH lookup.
    /// This is the YouTube two-files fix: bare "ffmpeg" lookup missed the
    /// bundled binaries, the merge port was never injected, and both streams
    /// finished as separate files.
    ///
    /// License policy (YouTube two-files fix #2): the LGPL gate is enforced for
    /// binaries found in the BUNDLED directory — Velox distributes those and the
    /// policy must hold for what we ship. A binary resolved through PATH belongs
    /// to the user's own system (winget/gyan/distro builds are commonly
    /// GPL-flagged): Velox merely invokes it, which is not distribution, and the
    /// old blanket refusal turned every such install into "download finishes,
    /// merge errors, two separate files".
    pub async fn discover_subprocess_at(
        dir: Option<&std::path::Path>,
    ) -> Result<Self, engine_core::error::EngineError> {
        let name = |n: &str| {
            if cfg!(windows) {
                format!("{n}.exe")
            } else {
                n.to_string()
            }
        };
        let pick = |n: &str| -> (String, bool) {
            if let Some(d) = dir {
                let p = d.join(name(n));
                if p.exists() {
                    return (p.display().to_string(), true); // bundled
                }
            }
            (name(n), false) // user's own PATH binary
        };
        let (ff, ff_bundled) = pick("ffmpeg");
        let (fp, _) = pick("ffprobe");
        let gate = if ff_bundled {
            LicenseGate::Enforce
        } else {
            LicenseGate::Skip
        };
        Ok(Self {
            pipeline: MediaPipeline::discover_with_policy(
                std::sync::Arc::new(SubprocessPort),
                std::path::PathBuf::from(ff),
                std::path::PathBuf::from(fp),
                gate,
            )
            .await?,
        })
    }
}

#[async_trait::async_trait]
impl<P: MediaPipelinePort> engine_core::ports::MediaMergePort for MergePort<P> {
    async fn convert_to_mp3(
        &self,
        id: i64,
        input: &std::path::Path,
        output: &std::path::Path,
        bus: &engine_core::events::EventBus,
        kill: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), engine_core::error::EngineError> {
        use engine_core::events::Event;
        let _ = bus.send(Event::Merging { id });
        self.pipeline.convert_mp3(input, output, kill).await?;
        Ok(())
    }

    async fn merge(
        &self,
        id: i64,
        video: &std::path::Path,
        audio: &std::path::Path,
        output: &std::path::Path,
        bus: &engine_core::events::EventBus,
        kill: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), engine_core::error::EngineError> {
        let input = MergeInput {
            video: video.to_path_buf(),
            audio: audio.to_path_buf(),
            output: output.to_path_buf(),
        };
        self.pipeline.merge(id, &input, bus, kill).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests;
