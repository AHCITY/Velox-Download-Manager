//! Media format selection (task: MP4 / MP3 + honest quality lists).
//!
//! Pure functions over yt-dlp `-J` metadata — no I/O, fully unit-tested
//! (tests.rs). The quality list shown in the UI is derived ONLY from what the
//! extractor reports (`formats[].height`): nothing is fabricated, and formats
//! whose height is unknown never appear as a quality option.
//!
//! Selection model:
//!   * MP4: best video-only stream at/below the chosen height + best audio
//!     stream, merged by the existing engine-media pipeline (`-c copy`).
//!     Falls back to the best progressive stream when no adaptive pair exists.
//!   * MP3: best audio-only stream (or the best combined stream's audio);
//!     conversion to MP3 happens in engine-media with libmp3lame.
use engine_core::config::MediaFormat;

/// One selectable quality, derived from real extractor data.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MediaQuality {
    /// Label shown to the user, e.g. "1080p", "1080p60", "720p".
    pub label: String,
    /// Video height in pixels (the selector value).
    pub height: u32,
    /// Frames per second when the extractor reports it (None → omit from label).
    pub fps: Option<u32>,
}

/// Summary of a media page's downloadable formats (the `media.formats` RPC).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FormatSummary {
    pub title: String,
    /// Video qualities available (desc order, deduped). Empty for audio-only media.
    #[serde(rename = "qualities")]
    pub video_qualities: Vec<MediaQuality>,
    /// Best-audio-only stream exists (adaptive audio without video).
    pub has_audio_only: bool,
    /// Any progressive (video+audio in one file) format exists.
    pub has_progressive: bool,
    /// Estimated size in bytes of the best format at each quality (approximate —
    /// YouTube frequently omits exact sizes; this is a display hint, never a promise).
    #[serde(rename = "sizesByHeight")]
    pub sizes_by_height: std::collections::BTreeMap<u32, Option<u64>>,
}

/// Classify one yt-dlp format entry.
pub(crate) enum StreamKind {
    /// video+audio in one file
    Progressive,
    /// video only
    VideoOnly,
    /// audio only
    AudioOnly,
}

pub(crate) fn classify(f: &serde_json::Value) -> Option<StreamKind> {
    let vcodec = f.get("vcodec").and_then(|c| c.as_str()).unwrap_or("none");
    let acodec = f.get("acodec").and_then(|c| c.as_str()).unwrap_or("none");
    match (vcodec != "none", acodec != "none") {
        (true, true) => Some(StreamKind::Progressive),
        (true, false) => Some(StreamKind::VideoOnly),
        (false, true) => Some(StreamKind::AudioOnly),
        (false, false) => None, // metadata-only rows (sb0/storyboard) — not media
    }
}

fn is_http_stream(f: &serde_json::Value) -> bool {
    matches!(
        f.get("protocol").and_then(|p| p.as_str()).unwrap_or(""),
        "https" | "http"
    )
}

fn fmt_height(f: &serde_json::Value) -> Option<u32> {
    f.get("height")
        .and_then(|h| h.as_u64())
        .map(|h| h as u32)
        .filter(|h| *h > 0)
}

fn fmt_fps(f: &serde_json::Value) -> Option<u32> {
    let fps = f.get("fps").and_then(|x| x.as_f64()).unwrap_or(0.0);
    (fps >= 1.0).then_some(fps.round() as u32)
}

fn fmt_size(f: &serde_json::Value) -> Option<u64> {
    f.get("filesize")
        .and_then(|s| s.as_u64())
        .or_else(|| f.get("filesize_approx").and_then(|s| s.as_u64()))
}

fn fmt_tbr(f: &serde_json::Value) -> f64 {
    f.get("tbr").and_then(|t| t.as_f64()).unwrap_or(0.0)
}

fn fmt_abr(f: &serde_json::Value) -> f64 {
    f.get("abr").and_then(|t| t.as_f64()).unwrap_or(0.0)
}

/// Standard quality buckets shown in the UI. Derived from observed heights —
/// a bucket only appears when the media actually offers it.
const QUALITY_BUCKETS: [(u32, &str); 7] = [
    (2160, "4K"),
    (1440, "2K"),
    (1080, "1080p"),
    (720, "720p"),
    (480, "480p"),
    (360, "360p"),
    (240, "240p"),
];

/// Build the honest quality list from yt-dlp metadata: unique heights of
/// VIDEO-carrying formats (video-only + progressive), snapped to standard
/// labels, descending. `fps` variants become separate entries when a height
/// exists at both 30 and >45 fps (e.g. "1080p" and "1080p60").
pub fn quality_list(meta: &serde_json::Value) -> Vec<MediaQuality> {
    let mut out: Vec<MediaQuality> = Vec::new();
    let formats = meta
        .get("formats")
        .and_then(|f| f.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    // (height, has_high_fps) for every video format
    let mut heights: Vec<(u32, bool)> = Vec::new();
    for f in formats {
        if !is_http_stream(f) {
            continue;
        }
        let Some(kind) = classify(f) else { continue };
        if !matches!(kind, StreamKind::Progressive | StreamKind::VideoOnly) {
            continue;
        }
        let Some(h) = fmt_height(f) else {
            continue; // unknown height → never offered as a quality
        };
        let high_fps = fmt_fps(f).map(|x| x > 45).unwrap_or(false);
        match heights.iter_mut().find(|(eh, _)| *eh == h) {
            Some(e) => e.1 |= high_fps,
            None => heights.push((h, high_fps)),
        }
    }
    heights.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for (h, high_fps) in heights {
        let bucket = QUALITY_BUCKETS
            .iter()
            .find(|(bh, _)| *bh == h)
            .map(|(_, l)| (*l, true));
        let label = match bucket {
            Some((l, _)) => l.to_string(),
            None => format!("{h}p"),
        };
        out.push(MediaQuality {
            label,
            height: h,
            fps: None,
        });
        if high_fps {
            // separate 60fps entry at the same height (labeled, e.g. "1080p60")
            if let Some((l, _)) = bucket {
                out.push(MediaQuality {
                    label: format!("{l}60"),
                    height: h,
                    fps: Some(60),
                });
            }
        }
    }
    out
}

/// Full `media.formats` summary for a URL's yt-dlp metadata.
pub fn format_summary(meta: &serde_json::Value) -> FormatSummary {
    let formats = meta
        .get("formats")
        .and_then(|f| f.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut has_audio_only = false;
    let mut has_progressive = false;
    let mut sizes: std::collections::BTreeMap<u32, Option<u64>> = Default::default();
    for f in formats {
        if !is_http_stream(f) {
            continue;
        }
        match classify(f) {
            Some(StreamKind::AudioOnly) => has_audio_only = true,
            Some(StreamKind::Progressive) => {
                has_progressive = true;
                if let Some(h) = fmt_height(f) {
                    // keep the largest known size per height (best-format hint)
                    let e = sizes.entry(h).or_insert(None);
                    let s = fmt_size(f);
                    *e = match (*e, s) {
                        (None, None) => None,
                        (None, Some(x)) => Some(x),
                        (Some(x), None) => Some(x),
                        (Some(x), Some(y)) => Some(x.max(y)),
                    };
                }
            }
            Some(StreamKind::VideoOnly) => {
                if let Some(h) = fmt_height(f) {
                    let e = sizes.entry(h).or_insert(None);
                    if e.is_none() {
                        if let Some(s) = fmt_size(f) {
                            *e = Some(s);
                        }
                    }
                }
            }
            None => {}
        }
    }
    FormatSummary {
        title: meta
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("media")
            .to_string(),
        video_qualities: quality_list(meta),
        has_audio_only,
        has_progressive,
        sizes_by_height: sizes,
    }
}

// ---------- selection ----------

/// Pick the best format entry among candidates by (preference, bitrate).
fn best_of<'a>(
    cands: impl Iterator<Item = (&'a serde_json::Value, usize, f64)>,
) -> Option<&'a serde_json::Value> {
    cands
        .max_by(|a, b| {
            (a.1, a.2.to_bits())
                .partial_cmp(&(b.1, b.2.to_bits()))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(f, _, _)| f)
}

fn ext_pref_video(ext: &str) -> usize {
    match ext {
        "mp4" => 2,
        "webm" => 1,
        _ => 0,
    }
}

fn ext_pref_audio(ext: &str) -> usize {
    match ext {
        "m4a" => 3,
        "mp3" | "aac" => 2,
        "webm" => 1,
        _ => 0,
    }
}

fn fmt_ext(f: &serde_json::Value) -> String {
    f.get("ext")
        .and_then(|e| e.as_str())
        .unwrap_or("mp4")
        .to_string()
}

/// A chosen pair for MP4 output: which yt-dlp entries to download.
#[derive(Debug)]
#[allow(dead_code)] // `progressive` documents the no-merge path; pick_plan relies on audio.is_some()
pub(crate) struct SelectedPair<'a> {
    pub video: &'a serde_json::Value,
    pub audio: Option<&'a serde_json::Value>,
    /// True when `video` alone carries both streams (no merge needed).
    pub progressive: bool,
}

/// Select formats for the MP4 path with an optional height cap.
/// Preference: adaptive mp4-preferring pair at/below the cap → progressive
/// at/below the cap → (fallback) the uncapped best pair, so a stale quality
/// list never blocks the download.
pub(crate) fn select_mp4<'a>(
    formats: &'a [serde_json::Value],
    max_height: Option<u32>,
    want_fps: Option<u32>,
) -> Option<SelectedPair<'a>> {
    let at_cap = |f: &serde_json::Value| {
        max_height.map_or(true, |cap| fmt_height(f).is_none_or(|h| h <= cap))
    };
    // optional fps preference: 60 requested → prefer >45fps formats at equal height
    let fps_rank = |f: &serde_json::Value| -> i32 {
        match want_fps {
            Some(want) if want > 45 => i32::from(fmt_fps(f).is_some_and(|x| x > 45)),
            _ => 0, // no preference: either fps is fine
        }
    };
    let video_cands = formats.iter().filter(|f| {
        is_http_stream(f)
            && matches!(
                classify(f),
                Some(StreamKind::VideoOnly) | Some(StreamKind::Progressive)
            )
            && at_cap(f)
    });
    // best video-only under the cap: (fps preference, ext preference, height, tbr)
    let best_video_only = video_cands
        .clone()
        .filter(|f| matches!(classify(f), Some(StreamKind::VideoOnly)))
        .max_by(|a, b| {
            (
                fps_rank(a),
                ext_pref_video(&fmt_ext(a)),
                fmt_height(a).unwrap_or(0),
                fmt_tbr(a).to_bits(),
            )
                .partial_cmp(&(
                    fps_rank(b),
                    ext_pref_video(&fmt_ext(b)),
                    fmt_height(b).unwrap_or(0),
                    fmt_tbr(b).to_bits(),
                ))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let best_progressive = video_cands
        .clone()
        .filter(|f| matches!(classify(f), Some(StreamKind::Progressive)))
        .max_by(|a, b| {
            (
                fps_rank(a),
                ext_pref_video(&fmt_ext(a)),
                fmt_height(a).unwrap_or(0),
                fmt_tbr(a).to_bits(),
            )
                .partial_cmp(&(
                    fps_rank(b),
                    ext_pref_video(&fmt_ext(b)),
                    fmt_height(b).unwrap_or(0),
                    fmt_tbr(b).to_bits(),
                ))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let best_audio = formats
        .iter()
        .filter(|f| is_http_stream(f) && matches!(classify(f), Some(StreamKind::AudioOnly)))
        .max_by(|a, b| {
            (
                ext_pref_audio(&fmt_ext(a)),
                fmt_abr(a).max(fmt_tbr(a)).to_bits(),
            )
                .partial_cmp(&(
                    ext_pref_audio(&fmt_ext(b)),
                    fmt_abr(b).max(fmt_tbr(b)).to_bits(),
                ))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    match (best_video_only, best_audio, best_progressive) {
        // adaptive pair (the quality path)
        (Some(v), Some(a), _) => Some(SelectedPair {
            video: v,
            audio: Some(a),
            progressive: false,
        }),
        // no pair but a progressive stream → single download
        (None, _, Some(p)) => Some(SelectedPair {
            video: p,
            audio: None,
            progressive: true,
        }),
        // video-only without audio at the cap: try the progressive fallback
        (Some(_), None, Some(p)) => Some(SelectedPair {
            video: p,
            audio: None,
            progressive: true,
        }),
        // last resort: uncapped retry happens in the caller (pick_plan_for)
        _ => None,
    }
}

/// Select the stream for the MP3 (audio-only) path: best audio-only format,
/// else the audio of the best combined format (engine-media strips the video
/// during conversion).
pub(crate) fn select_audio<'a>(formats: &'a [serde_json::Value]) -> Option<&'a serde_json::Value> {
    best_of(
        formats
            .iter()
            .filter(|f| is_http_stream(f) && matches!(classify(f), Some(StreamKind::AudioOnly)))
            .map(|f| (f, ext_pref_audio(&fmt_ext(f)), fmt_abr(f).max(fmt_tbr(f)))),
    )
    .or_else(|| {
        best_of(
            formats
                .iter()
                .filter(|f| {
                    is_http_stream(f) && matches!(classify(f), Some(StreamKind::Progressive))
                })
                .map(|f| (f, ext_pref_audio(&fmt_ext(f)), fmt_abr(f).max(fmt_tbr(f)))),
        )
    })
}

/// The user's format choice for a media download, threaded from the Add dialog
/// through download.create → engine-core config. `None` keeps the legacy
/// "best available" behaviour.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaFormatChoice {
    Mp4,
    Mp3,
}

impl From<MediaFormat> for MediaFormatChoice {
    fn from(f: MediaFormat) -> Self {
        match f {
            MediaFormat::Mp4 => MediaFormatChoice::Mp4,
            MediaFormat::Mp3 => MediaFormatChoice::Mp3,
        }
    }
}
