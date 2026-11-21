//! MediaPipeline merge stage (architecture review §9, docs/FFMPEG_INTEGRATION.md).
//!
//! Manager-level wrapper job — NOT part of the transfer engine. FFmpeg runs as a
//! subprocess only, stream-copy (`-c copy`) only, kill-safe, LGPL-policy enforced,
//! ffprobe post-validated, honest errors. All subprocess operations go through the
//! `MediaPipelinePort` port so tests are deterministic without real binaries.

pub mod pipeline;

pub use pipeline::{
    build_merge_args, MediaPipeline, MediaPipelinePort, MergeInput, SubprocessOutcome,
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
        Ok(Self {
            pipeline: MediaPipeline::discover(std::sync::Arc::new(SubprocessPort)).await?,
        })
    }
}

#[async_trait::async_trait]
impl<P: MediaPipelinePort> engine_core::ports::MediaMergePort for MergePort<P> {
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
