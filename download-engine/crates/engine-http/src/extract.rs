//! Media-page URL handling via an EXTERNAL extractor (yt-dlp), fully separated from the
//! generic HTTP engine.
//!
//! Design (per requirements):
//! - Velox is an HTTP download manager. A YouTube `watch`/`shorts`/`youtu.be` URL is a
//!   web PAGE, not a media file — without extraction it downloads HTML (the exact
//!   "watch" failure from the field). This module resolves real media stream URLs so
//!   the ordinary HTTP engine downloads actual bytes with the real extension.
//! - yt-dlp is never bundled or linked: it is invoked as an external executable found
//!   at `EngineConfig::extractor_path` (path or bare name resolved through PATH).
//!   License: Unlicense (public-domain equivalent) — see docs/EXTRACTORS.md for
//!   installation, Windows packaging and update strategy.
//! - No DRM, paywall or access-control bypass: authentication errors are surfaced, DRM
//!   content is refused. Progressive https streams (video+audio in one file) download
//!   directly — the fast path. Adaptive media (separate video/audio https streams) is
//!   resolved into a two-stream plan (`ResourceInfo::media`); the manager downloads
//!   both streams with the ordinary engine and merges them via the LGPL ffmpeg
//!   pipeline (`-c copy`, ffprobe-validated) — no transcoding, original quality.
//! - On failure the error states precisely what is missing instead of pretending the
//!   generic HTTP engine supports the URL.

use engine_core::config::{AuthConfig, EngineConfig};
use engine_core::error::{EngineError, ErrorKind};
use engine_core::inspect::ResourceInfo;
use engine_core::ports::RequestHeaders;

use crate::ReqwestBackend;

/// Extractor argv header exposure (audit D2): user credentials are passed to the
/// extractor process as `--add-headers` arguments. Process arguments are visible to
/// other processes of the SAME user only (same trust boundary as the cookie jar file);
/// they never appear in engine events, status output or logs.

#[derive(Debug)]
struct ChosenStream {
    url: String,
    ext: String,
    mime: String,
    size: Option<u64>,
    format_id: String,
}

/// Build `--add-headers` arguments for the extractor from the user's request headers
/// (audit D2): a logged-in user's cookies are the standard remedy for YouTube's
/// "Sign in to confirm you're not a bot" bot wall, age-gated and members-only media.
/// Headers are value-sanitized (no CR/LF — request-splitting) and the User-Agent falls
/// back to the engine default.
fn extractor_header_args(hdrs: &RequestHeaders, engine: &EngineConfig) -> Vec<String> {
    let mut args = Vec::new();
    let mut push = |name: &str, value: &str| {
        let value = value.replace(['\r', '\n'], " ").trim().to_string();
        if !value.is_empty() {
            args.push(format!("{name}: {value}"));
        }
    };
    if !hdrs.cookies.is_empty() {
        let cookie = hdrs
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        push("Cookie", &cookie);
    }
    if let Some(r) = &hdrs.referer {
        push("Referer", r);
    }
    let ua = hdrs
        .user_agent
        .clone()
        .unwrap_or_else(|| engine.user_agent.clone());
    push("User-Agent", &ua);
    if let Some(auth) = &hdrs.auth {
        match auth {
            AuthConfig::Basic { user, password } => push(
                "Authorization",
                &format!("Basic {}", basic_token(user, password)),
            ),
            AuthConfig::Bearer { token } => push("Authorization", &format!("Bearer {token}")),
        }
    }
    args
}

/// RFC 4648 base64 (standard alphabet, padded) for the Basic auth credential —
/// implemented inline so no new dependency is added for one encoding call.
fn basic_token(user: &str, password: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = format!("{user}:{password}").into_bytes();
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Resolve a media-page URL to a progressive https stream via yt-dlp.
pub async fn resolve_media(
    backend: &ReqwestBackend,
    url: &str,
    hdrs: &RequestHeaders,
) -> Result<ResourceInfo, EngineError> {
    resolve_media_with(backend, url, hdrs, None, None, None).await
}

/// Format-aware media resolution (Add dialog "MP4 @ 720p" / "MP3"):
/// `choice` carries the user's format/height/fps selection from
/// download.create's DownloadConfig. `None` = legacy best-available.
pub async fn resolve_media_with(
    backend: &ReqwestBackend,
    url: &str,
    hdrs: &RequestHeaders,
    choice: Option<crate::formats::MediaFormatChoice>,
    max_height: Option<u32>,
    want_fps: Option<u32>,
) -> Result<ResourceInfo, EngineError> {
    // Shared yt-dlp -J run (audit D1 kill-on-drop + D2 header forwarding live
    // in run_extractor_json, used by both the resolve and the formats probe).
    let meta = run_extractor_json(backend, url, hdrs).await?;
    let title = meta
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("media")
        .to_string();
    let plan_choice = choice.map(|format| PlanChoice {
        format,
        max_height,
        want_fps,
    });
    let plan = pick_plan(&meta, plan_choice)?;
    let info = if plan.to_mp3 {
        // MP3 path: single audio-stream download; the media job converts it to
        // .mp3 (libmp3lame -q:a 0) once the raw stream lands on disk.
        ResourceInfo {
            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some("audio/mpeg".into()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            // the DOWNLOAD is the raw audio stream; conversion renames to .mp3
            suggested_name: Some(format!("{} (audio).{}", title, plan.video.ext)),
            resolve_note: Some(format!(
                "media extractor: “{}” — best audio [{}] → MP3",
                title, plan.video.format_id
            )),
            media: Some(engine_core::config::MediaMergeSpec {
                audio_url: String::new(),
                output_ext: Some("mp3".into()),
                output_name: Some(format!("{}.mp3", title)),
                audio_name: None,
                convert: Some(engine_core::config::MediaConvert::Mp3),
            }),
            thumbnail: pick_thumbnail(&meta),
            ..Default::default()
        }
    } else if let Some(audio) = &plan.audio {
        // Adaptive: separate video/audio https streams → two-stream media job.
        let output_ext = merge_container(&plan.video.ext, &audio.ext);
        ResourceInfo {
            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some(plan.video.mime.clone()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{} (video stream).{}", title, plan.video.ext)),
            resolve_note: Some(format!(
                "media extractor: “{}” — adaptive {}+{} → merged .{}",
                title, plan.video.format_id, audio.format_id, output_ext
            )),
            media: Some(engine_core::config::MediaMergeSpec {
                audio_url: audio.url.clone(),
                output_ext: Some(output_ext.clone()),
                output_name: Some(format!("{}.{}", title, output_ext)),
                audio_name: Some(format!("{} (audio stream).{}", title, audio.ext)),
                convert: None,
            }),
            thumbnail: pick_thumbnail(&meta),
            ..Default::default()
        }
    } else {
        // Fast path: progressive stream (video+audio in ONE file) — download directly.
        ResourceInfo {
            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some(plan.video.mime.clone()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{}.{}", title, plan.video.ext)),
            resolve_note: Some(format!(
                "media extractor: “{}” [{}]",
                title, plan.video.format_id
            )),
            thumbnail: pick_thumbnail(&meta),
            ..Default::default()
        }
    };
    Ok(info)
}

/// Best source-provided thumbnail for ANY media source (not only YouTube):
/// yt-dlp reports `thumbnail` plus a ranked `thumbnails[]` list. Prefer the
/// explicit field, then the largest-resolution entry. This is what makes
/// "any source that has a thumbnail keeps it" work for YouTube, Vimeo,
/// SoundCloud, Dailymotion, and every other extractor that reports one.
fn pick_thumbnail(meta: &serde_json::Value) -> Option<String> {
    let explicit = meta.get("thumbnail").and_then(|t| t.as_str());
    let best_from_list = meta
        .get("thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|list| {
            list.iter()
                .filter_map(|t| {
                    let url = t.get("url").and_then(|u| u.as_str())?;
                    if url.is_empty() {
                        return None;
                    }
                    let w = t.get("width").and_then(|w| w.as_u64()).unwrap_or(0);
                    let h = t.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
                    // prefer preference (yt-dlp ranks), fall back to resolution
                    let pref = t.get("preference").and_then(|p| p.as_f64()).unwrap_or(0.0);
                    Some((pref, w.saturating_mul(h), url))
                })
                .max_by_key(|(pref, area, _)| (*pref * 1_000_000.0) as u64 + *area)
                .map(|(_, _, url)| url.to_string())
        });
    match (explicit, best_from_list) {
        (Some(u), _) => Some(u.to_string()),
        (None, Some(u)) => Some(u),
        (None, None) => None,
    }
}

/// Container choice for a merged artifact without transcoding: mp4 hosts
/// H.264/AV1 + AAC/MP3 cleanly; VP9/Opus pairs go into Matroska (mkv), which
/// accepts any codec stream-copy.
fn merge_container(video_ext: &str, audio_ext: &str) -> String {
    let mp4_video = video_ext == "mp4";
    let mp4_audio = matches!(audio_ext, "m4a" | "aac" | "mp3");
    if mp4_video && mp4_audio {
        "mp4".into()
    } else {
        "mkv".into()
    }
}

/// The resolved download plan for one media URL.
///
/// `audio: None`  → progressive: `video` already contains both streams.
/// `audio: Some`  → adaptive: download `video` + `audio`, then merge (`-c copy`).
/// `to_mp3`       → the download is the best audio stream; the media job
///                  converts it to `.mp3` with the bundled ffmpeg (libmp3lame).
#[derive(Debug)]
struct MediaPlan {
    video: ChosenStream,
    audio: Option<ChosenStream>,
    to_mp3: bool,
}

/// User format choice: height/fps come from the caller's DownloadConfig, the
/// format (mp4/mp3) from the Add dialog.
#[derive(Debug, Clone, Copy)]
pub struct PlanChoice {
    pub format: crate::formats::MediaFormatChoice,
    pub max_height: Option<u32>,
    pub want_fps: Option<u32>,
}

impl From<crate::formats::MediaFormatChoice> for PlanChoice {
    fn from(f: crate::formats::MediaFormatChoice) -> Self {
        PlanChoice {
            format: f,
            max_height: None,
            want_fps: None,
        }
    }
}

fn chosen_from(f: &serde_json::Value) -> Option<ChosenStream> {
    let url = f.get("url").and_then(|u| u.as_str())?;
    let ext = fmt_ext_pub(f);
    let mime = f
        .get("mime_type")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| match ext.as_str() {
            "webm" => "video/webm".into(),
            "mkv" => "video/x-matroska".into(),
            "m4a" => "audio/mp4".into(),
            "mp3" => "audio/mpeg".into(),
            _ => "video/mp4".into(),
        });
    Some(ChosenStream {
        url: url.to_string(),
        ext,
        mime,
        size: f
            .get("filesize")
            .and_then(|s| s.as_u64())
            .or_else(|| f.get("filesize_approx").and_then(|s| s.as_u64())),
        format_id: f
            .get("format_id")
            .and_then(|i| i.as_str())
            .unwrap_or("unknown")
            .to_string(),
    })
}

fn fmt_ext_pub(f: &serde_json::Value) -> String {
    f.get("ext")
        .and_then(|e| e.as_str())
        .unwrap_or("mp4")
        .to_string()
}

/// Pick the download plan from yt-dlp metadata honouring the user's format
/// choice (see formats.rs for the selection model). When a format choice is
/// given but nothing matches at the requested height, we fall back to the
/// uncapped best set — a stale quality list must never block a download.
fn pick_plan(
    meta: &serde_json::Value,
    choice: Option<PlanChoice>,
) -> Result<MediaPlan, EngineError> {
    use crate::formats::{select_audio, select_mp4, MediaFormatChoice, StreamKind};

    let formats = meta
        .get("formats")
        .and_then(|f| f.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    // MP3 path: best audio stream (audio-only preferred, else combined-audio);
    // conversion happens in the media job after download.
    if let Some(ch) = choice {
        if ch.format == MediaFormatChoice::Mp3 {
            if let Some(a) = select_audio(formats) {
                if let Some(chosen) = chosen_from(a) {
                    return Ok(MediaPlan {
                        video: chosen,
                        audio: None,
                        to_mp3: true,
                    });
                }
            }
            return Err(EngineError::new(
                ErrorKind::UnsupportedUrl,
                "no audio stream is available for this media — it cannot be saved as MP3",
            ));
        }
    }

    // MP4 path (explicit choice or legacy best-available).
    let max_height = choice.and_then(|c| c.max_height);
    let want_fps = choice.and_then(|c| c.want_fps);
    let try_select = |cap: Option<u32>| -> Option<(ChosenStream, Option<ChosenStream>)> {
        let pair = select_mp4(formats, cap, want_fps)?;
        let video = chosen_from(pair.video)?;
        let audio = match pair.audio {
            Some(a) => Some(chosen_from(a)?),
            None => None,
        };
        Some((video, audio))
    };
    if let Some((video, audio)) = try_select(max_height) {
        return Ok(MediaPlan {
            video,
            audio,
            to_mp3: false,
        });
    }
    if max_height.is_some() {
        // requested height unavailable (stale list / format vanished): honest
        // fallback to the best available set rather than failing the download
        if let Some((video, audio)) = try_select(None) {
            return Ok(MediaPlan {
                video,
                audio,
                to_mp3: false,
            });
        }
    }
    // top-level single stream (some extractors put it outside formats[])
    if let Some(stream_url) = meta.get("url").and_then(|u| u.as_str()) {
        let proto = meta.get("protocol").and_then(|p| p.as_str()).unwrap_or("");
        if matches!(proto, "https" | "http") {
            let ext = fmt_ext_pub(meta);
            // A top-level stream is a SINGLE download regardless of codec type —
            // an audio-only top-level stream is just a file (podcast etc.).
            let video = ChosenStream {
                url: stream_url.to_string(),
                mime: match ext.as_str() {
                    "webm" => "video/webm".into(),
                    _ => "video/mp4".into(),
                },
                ext,
                size: meta
                    .get("filesize")
                    .and_then(|s| s.as_u64())
                    .or_else(|| meta.get("filesize_approx").and_then(|s| s.as_u64())),
                format_id: meta
                    .get("format_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("direct")
                    .to_string(),
            };
            return Ok(MediaPlan {
                video,
                audio: None,
                to_mp3: false,
            });
        }
    }
    // honest failure modes
    let has_video = formats.iter().any(|f| {
        matches!(
            classify_pub(f),
            Some(StreamKind::Progressive) | Some(StreamKind::VideoOnly)
        )
    });
    let has_audio = formats.iter().any(|f| {
        matches!(
            classify_pub(f),
            Some(StreamKind::Progressive) | Some(StreamKind::AudioOnly)
        )
    });
    if has_video && !has_audio {
        return Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "only video-only streams are available for this media — no audio stream to pair with them; the media may need authentication or is not downloadable",
        ));
    }
    if !has_video && has_audio {
        return Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "only audio-only streams are available for this media — no video stream to pair with them",
        ));
    }
    Err(EngineError::new(
        ErrorKind::UnsupportedUrl,
        "no downloadable https media stream found for this URL — only manifest formats (HLS/DASH), which are not file downloads",
    ))
}

fn classify_pub(f: &serde_json::Value) -> Option<crate::formats::StreamKind> {
    crate::formats::classify(f)
}

/// Probe a media page's honest format list (`media.formats` RPC): runs yt-dlp
/// `-J` (simulate-only, nothing is downloaded) and derives the available
/// qualities from the reported formats — nothing is fabricated.
pub async fn probe_formats(
    backend: &ReqwestBackend,
    url: &str,
    hdrs: &RequestHeaders,
) -> Result<crate::formats::FormatSummary, EngineError> {
    let meta = run_extractor_json(backend, url, hdrs).await?;
    Ok(crate::formats::format_summary(&meta))
}

/// Shared yt-dlp `-J` invocation (used by resolve_media_with and probe_formats).
async fn run_extractor_json(
    backend: &ReqwestBackend,
    url: &str,
    hdrs: &RequestHeaders,
) -> Result<serde_json::Value, EngineError> {
    let Some(program) = backend
        .engine
        .extractor_path
        .clone()
        .filter(|p| !p.to_string_lossy().trim().is_empty())
    else {
        return Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "this is a media-page URL (YouTube), not a direct file — an HTTP download \
             manager cannot fetch the video bytes from a watch page. Install the optional \
             yt-dlp extractor (docs/EXTRACTORS.md: winget install yt-dlp.yt-dlp) or paste \
             a direct media URL",
        ));
    };

    let mut cmd = tokio::process::Command::new(&program);
    // audit D1: kill_on_drop(true) — when the timeout below drops the future holding
    // the Child, a hung yt-dlp must die with it (default tokio behavior leaves an
    // orphan process running indefinitely, one per timed-out add)
    cmd.kill_on_drop(true);
    cmd.args(["-J", "--no-playlist", "--no-warnings"]);
    for h in extractor_header_args(hdrs, &backend.engine) {
        cmd.args(["--add-headers", &h]);
    }
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd.spawn().map_err(|e| {
        EngineError::new(
            ErrorKind::UnsupportedUrl,
            format!(
                "the configured media extractor '{}' could not be started ({e}); install \
                 yt-dlp and verify with `yt-dlp --version` (docs/EXTRACTORS.md)",
                program.display()
            ),
        )
    })?;
    let output = tokio::time::timeout(backend.engine.extractor_timeout, child.wait_with_output())
        .await
        .map_err(|_| {
            EngineError::new(
                ErrorKind::Timeout,
                "media extractor timed out (the page may be unreachable or the \
                 extractor outdated; yt-dlp must be updated frequently)",
            )
        })?
        .map_err(|e| EngineError::new(ErrorKind::Io, format!("extractor failed: {e}")))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown extractor error");
        let low = last.to_ascii_lowercase();
        if low.contains("drm") {
            return Err(EngineError::new(
                ErrorKind::UnsupportedUrl,
                "the media is DRM protected; Velox does not bypass DRM",
            ));
        }
        if low.contains("sign in")
            || low.contains("private")
            || low.contains("members-only")
            || low.contains("login")
            || low.contains("age")
        {
            return Err(EngineError::new(
                ErrorKind::AuthRequired,
                format!("the media requires authentication (reported by the extractor): {last}"),
            ));
        }
        return Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            format!("the extractor could not resolve this URL: {last}"),
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|e| {
        EngineError::new(
            ErrorKind::InvalidResponse,
            format!("extractor returned unparseable metadata: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(
        id: &str,
        ext: &str,
        proto: &str,
        v: &str,
        a: &str,
        tbr: f64,
        size: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "format_id": id, "ext": ext, "protocol": proto,
            "vcodec": v, "acodec": a, "tbr": tbr, "filesize": size,
            "url": format!("https://media.example/{}.{}", id, ext),
        })
    }

    #[test]
    fn thumbnail_any_source_explizit_field_and_best_list_entry() {
        // explicit thumbnail wins (any source, not only YouTube)
        let meta = serde_json::json!({
            "thumbnail": "https://cdn.example/maxres.jpg",
            "thumbnails": [{"url": "https://cdn.example/small.jpg", "width": 120, "height": 90}],
        });
        assert_eq!(pick_thumbnail(&meta).as_deref(), Some("https://cdn.example/maxres.jpg"));
        // no explicit field: the largest-resolution list entry is kept
        let meta = serde_json::json!({
            "thumbnails": [
                {"url": "https://cdn.example/small.jpg", "width": 120, "height": 90},
                {"url": "https://cdn.example/big.jpg", "width": 1280, "height": 720},
                {"url": "https://cdn.example/mid.jpg", "width": 640, "height": 360},
            ],
        });
        assert_eq!(pick_thumbnail(&meta).as_deref(), Some("https://cdn.example/big.jpg"));
        // nothing reported → None (UI falls back to the category icon)
        assert_eq!(pick_thumbnail(&serde_json::json!({})), None);
    }

    #[test]
    fn progressive_https_mp4_wins_over_webm_and_higher_tbr() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [
                fmt("hls-720", "mp4", "m3u8", "h264", "aac", 1200.0, 9_000_000),
                fmt("webm-360", "webm", "https", "vp9", "opus", 700.0, 4_000_000),
                fmt("mp4-360", "mp4", "https", "avc1", "mp4a", 640.0, 3_500_000),
                fmt("video-only", "mp4", "https", "avc1", "none", 4000.0, 30_000_000),
            ]
        });
        let plan = pick_plan(&meta, None).unwrap();
        assert!(
            plan.audio.is_none(),
            "progressive stream must keep the fast path"
        );
        assert_eq!(
            plan.video.format_id, "mp4-360",
            "progressive https, mp4 preferred"
        );
        assert_eq!(plan.video.mime, "video/mp4");
        assert_eq!(plan.video.size, Some(3_500_000));
    }

    #[test]
    fn adaptive_pair_is_selected_when_no_progressive_exists() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [
                fmt("v-1080", "mp4", "https", "avc1", "none", 4000.0, 30_000_000),
                fmt("v-720", "mp4", "https", "avc1", "none", 2000.0, 15_000_000),
                fmt("a-m4a", "m4a", "https", "none", "mp4a.40.2", 130.0, 4_000_000),
                fmt("a-webm", "webm", "https", "none", "opus", 160.0, 5_000_000),
                fmt("hls-x", "mp4", "m3u8", "h264", "aac", 5000.0, 40_000_000),
            ]
        });
        let plan = pick_plan(&meta, None).unwrap();
        let audio = plan
            .audio
            .expect("adaptive pair when no progressive stream");
        assert_eq!(
            plan.video.format_id, "v-1080",
            "highest-bitrate video-only wins"
        );
        assert_eq!(
            audio.format_id, "a-m4a",
            "m4a preferred over higher-abr opus (container pair)"
        );
    }

    #[test]
    fn video_only_without_audio_is_an_honest_error() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [fmt("v-only", "mp4", "https", "avc1", "none", 4000.0, 1)]
        });
        let e = pick_plan(&meta, None).unwrap_err();
        assert_eq!(e.kind_code(), "UNSUPPORTED_URL");
        assert!(e.message.contains("audio"), "{e}");
    }

    #[test]
    fn manifest_only_is_an_honest_error() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [fmt("hls-x", "mp4", "m3u8", "h264", "aac", 5000.0, 1)]
        });
        let e = pick_plan(&meta, None).unwrap_err();
        assert_eq!(e.kind_code(), "UNSUPPORTED_URL");
        assert!(e.message.contains("manifest"), "{e}");
    }

    #[test]
    fn merge_container_pairs_codecs_without_transcoding() {
        assert_eq!(merge_container("mp4", "m4a"), "mp4", "h264+aac → mp4");
        assert_eq!(merge_container("webm", "webm"), "mkv", "vp9+opus → mkv");
        assert_eq!(merge_container("mp4", "webm"), "mkv", "mixed → mkv");
        assert_eq!(merge_container("webm", "m4a"), "mkv");
        assert_eq!(merge_container("mp4", "mp3"), "mp4");
    }

    #[test]
    fn extractor_headers_carry_session_context() {
        let mut hdrs = RequestHeaders::default();
        hdrs.cookies.insert("SID".into(), "sess-123".into());
        hdrs.cookies.insert("HSID".into(), "h-456".into());
        hdrs.referer = Some("https://www.youtube.com/watch?v=x".into());
        hdrs.auth = Some(AuthConfig::Basic {
            user: "user".into(),
            password: "pass".into(),
        });
        let args = extractor_header_args(&hdrs, &EngineConfig::default());
        let cookie = args.iter().find(|a| a.starts_with("Cookie: ")).unwrap();
        assert!(cookie.contains("SID=sess-123") && cookie.contains("HSID=h-456"));
        assert!(args
            .iter()
            .any(|a| a.starts_with("Referer: https://www.youtube.com/")));
        assert!(
            args.iter()
                .any(|a| a.starts_with("Authorization: Basic dXNlcjpwYXNz")),
            "basic auth must be RFC 4648 base64 of user:pass"
        );
        assert!(
            args.iter().any(|a| a.starts_with("User-Agent: velox/")),
            "UA falls back to the engine default"
        );
    }

    #[test]
    fn extractor_headers_sanitize_crlf_and_skip_empty() {
        let hdrs = RequestHeaders {
            referer: Some("https://evil.example/a\r\nX-Injected: 1".into()),
            ..Default::default()
        };
        let args = extractor_header_args(&hdrs, &EngineConfig::default());
        let r = args.iter().find(|a| a.starts_with("Referer: ")).unwrap();
        assert!(
            !r.contains('\r') && !r.contains('\n'),
            "no header injection via argv"
        );
    }

    #[test]
    fn direct_top_level_stream_is_used() {
        let meta = serde_json::json!({
            "title": "Direct", "url": "https://media.example/v.mp4",
            "protocol": "https", "ext": "mp4", "filesize": 123
        });
        let plan = pick_plan(&meta, None).unwrap();
        assert!(plan.audio.is_none());
        let c = plan.video;
        assert_eq!(c.url, "https://media.example/v.mp4");
        assert_eq!(c.size, Some(123));
    }

    // ---- format selection (MP4 quality caps, MP3, honest fallbacks) ----

    fn yt_meta() -> serde_json::Value {
        serde_json::json!({
            "title": "Test Video",
            "formats": [
                {"format_id": "251", "url": "https://s/audio.webm", "protocol": "https", "ext": "webm", "vcodec": "none", "acodec": "opus", "abr": 160},
                {"format_id": "140", "url": "https://s/audio.m4a", "protocol": "https", "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "abr": 128},
                {"format_id": "137", "url": "https://s/v1080.mp4", "protocol": "https", "ext": "mp4", "vcodec": "h264", "acodec": "none", "height": 1080, "tbr": 4000, "filesize": 100000},
                {"format_id": "136", "url": "https://s/v720.mp4", "protocol": "https", "ext": "mp4", "vcodec": "h264", "acodec": "none", "height": 720, "tbr": 2500, "filesize": 60000},
                {"format_id": "135", "url": "https://s/v480.mp4", "protocol": "https", "ext": "mp4", "vcodec": "h264", "acodec": "none", "height": 480, "tbr": 1200, "filesize": 30000},
                {"format_id": "299", "url": "https://s/v1080_60.mp4", "protocol": "https", "ext": "mp4", "vcodec": "h264", "acodec": "none", "height": 1080, "fps": 60, "tbr": 5500},
                {"format_id": "18",  "url": "https://s/prog360.mp4", "protocol": "https", "ext": "mp4", "vcodec": "h264", "acodec": "aac", "height": 360, "tbr": 600},
                {"format_id": "94",  "url": "https://s/hls.m3u8", "protocol": "m3u8_native", "ext": "mp4", "vcodec": "h264", "acodec": "aac", "height": 480}
            ]
        })
    }

    #[test]
    fn quality_list_reports_only_real_heights_desc() {
        let q = crate::formats::quality_list(&yt_meta());
        let labels: Vec<&str> = q.iter().map(|x| x.label.as_str()).collect();
        // 1080 (30+60fps variants), 720, 480, 360 progressive — m3u8 excluded
        assert_eq!(labels, vec!["1080p", "1080p60", "720p", "480p", "360p"]);
    }

    #[test]
    fn format_summary_flags_audio_and_sizes() {
        let sum = crate::formats::format_summary(&yt_meta());
        assert!(sum.has_audio_only);
        assert!(sum.has_progressive);
        assert_eq!(sum.sizes_by_height.get(&1080), Some(&Some(100000)));
        assert_eq!(sum.sizes_by_height.get(&720), Some(&Some(60000)));
    }

    #[test]
    fn mp4_at_height_cap_selects_under_cap() {
        let meta = yt_meta();
        let plan = pick_plan(
            &meta,
            Some(PlanChoice {
                format: crate::formats::MediaFormatChoice::Mp4,
                max_height: Some(720),
                want_fps: None,
            }),
        )
        .unwrap();
        // 720 pair: video 136 + best audio (webm 160abr wins by abr, m4a by ext pref → m4a pref 3 > webm 1)
        assert_eq!(plan.video.format_id, "136");
        let audio = plan.audio.as_ref().unwrap();
        assert_eq!(audio.format_id, "140");
        assert!(!plan.to_mp3);
    }

    #[test]
    fn mp4_best_default_picks_top_quality() {
        let plan = pick_plan(&yt_meta(), None).unwrap();
        assert_eq!(plan.video.format_id, "299"); // 1080p60: same height, higher tbr wins
        assert!(plan.audio.is_some());
    }

    #[test]
    fn mp4_fps_preference_picks_60fps_variant() {
        let plan = pick_plan(
            &yt_meta(),
            Some(PlanChoice {
                format: crate::formats::MediaFormatChoice::Mp4,
                max_height: Some(1080),
                want_fps: Some(60),
            }),
        )
        .unwrap();
        assert_eq!(plan.video.format_id, "299");
    }

    #[test]
    fn mp3_choice_takes_best_audio_only_stream() {
        let plan = pick_plan(
            &yt_meta(),
            Some(PlanChoice {
                format: crate::formats::MediaFormatChoice::Mp3,
                max_height: None,
                want_fps: None,
            }),
        )
        .unwrap();
        assert!(plan.to_mp3);
        assert!(plan.audio.is_none());
        assert_eq!(plan.video.format_id, "140"); // m4a ext preference wins
    }

    #[test]
    fn stale_height_falls_back_to_best_available() {
        let plan = pick_plan(
            &yt_meta(),
            Some(PlanChoice {
                format: crate::formats::MediaFormatChoice::Mp4,
                max_height: Some(4800), // 4K not offered — never blocks the download
                want_fps: None,
            }),
        )
        .unwrap();
        assert_eq!(plan.video.format_id, "299"); // falls back to the true best
    }

    #[test]
    fn mp3_without_any_audio_is_an_honest_error() {
        let meta = serde_json::json!({
            "title": "Video only",
            "formats": [
                {"format_id": "137", "url": "https://s/v.mp4", "protocol": "https", "ext": "mp4",
                 "vcodec": "h264", "acodec": "none", "height": 1080, "tbr": 4000}
            ]
        });
        let e = pick_plan(
            &meta,
            Some(PlanChoice {
                format: crate::formats::MediaFormatChoice::Mp3,
                max_height: None,
                want_fps: None,
            }),
        )
        .unwrap_err();
        assert!(e.message.contains("no audio stream"), "{}", e.message);
    }
}
