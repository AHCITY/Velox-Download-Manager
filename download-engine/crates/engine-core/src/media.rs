//! Container-format knowledge shared by the manager and the media pipeline.
//!
//! The merge stage writes its output to a temp file whose extension is
//! `.part` (`<final>.merging.part`), so ffmpeg cannot infer the muxer from
//! the filename — the container format must be passed explicitly with `-f`.
//! This module is the single mapping from the logical container extension
//! (what `MediaMergeSpec::output_ext` carries: `mp4`, `mkv`, …) to the
//! ffmpeg muxer name, so core and pipeline never disagree.

/// Map a container extension to the ffmpeg muxer name that writes it.
///
/// Velox's extractor only ever produces `mp4` (h264/aac family) or `mkv`
/// (anything mixed); the wider map exists so a user-set `output_ext` in the
/// media-merge spec degrades to a real muxer instead of a broken argv.
pub fn container_to_ffmpeg_format(ext: &str) -> &'static str {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "mp4",
        "m4a" | "m4b" => "ipod",
        "mov" => "mov",
        "mkv" => "matroska",
        "webm" => "webm",
        "ts" | "mts" | "m2ts" => "mpegts",
        "mp3" => "mp3",
        "flac" => "flac",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        // unknown container: mp4 is the historical default and the format the
        // UI offers; forcing it keeps `.part` temp files muxable.
        _ => "mp4",
    }
}

/// True for muxers that understand `-movflags` (faststart & friends). Passing
/// it to e.g. the matroska muxer is an error, so it must stay conditional.
pub fn accepts_movflags(muxer: &str) -> bool {
    matches!(muxer, "mp4" | "mov" | "ipod")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_map_to_real_muxers() {
        assert_eq!(container_to_ffmpeg_format("mp4"), "mp4");
        assert_eq!(container_to_ffmpeg_format("MKV"), "matroska");
        assert_eq!(container_to_ffmpeg_format("mkv"), "matroska");
        assert_eq!(container_to_ffmpeg_format(".mp4"), "mp4");
        assert_eq!(container_to_ffmpeg_format("webm"), "webm");
        assert_eq!(container_to_ffmpeg_format("m4a"), "ipod");
        assert_eq!(container_to_ffmpeg_format("totally-unknown"), "mp4");
    }

    #[test]
    fn movflags_only_for_mov_family() {
        assert!(accepts_movflags("mp4"));
        assert!(accepts_movflags("mov"));
        assert!(accepts_movflags("ipod"));
        assert!(!accepts_movflags("matroska"));
        assert!(!accepts_movflags("webm"));
    }
}
