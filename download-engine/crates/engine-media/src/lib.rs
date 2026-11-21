//! MediaPipeline merge stage (architecture review §9, docs/FFMPEG_INTEGRATION.md).
//!
//! Manager-level wrapper job — NOT part of the transfer engine. FFmpeg runs as a
//! subprocess only, stream-copy (`-c copy`) only, kill-safe, LGPL-policy enforced,
//! ffprobe post-validated, honest errors. All subprocess operations go through the
//! `MediaPipelinePort` port so tests are deterministic without real binaries.

pub mod pipeline;

pub use pipeline::{
    build_merge_args, MediaPipeline, MediaPipelinePort, MergeInput, SubprocessOutcome,
};

#[cfg(test)]
mod tests;
