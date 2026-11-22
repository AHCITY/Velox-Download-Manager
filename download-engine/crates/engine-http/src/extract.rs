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
    // audit D2: forward the user's session context (cookies/referer/UA/auth) — the
    // remedy for the YouTube bot wall and age-gated media. Value order: yt-dlp applies
    // --add-headers to every HTTP request it makes, including the media stream probe.
    for h in extractor_header_args(hdrs, &backend.engine) {
        cmd.args(["--add-headers", &h]);
    }
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Windows: yt-dlp is a console-subsystem executable; spawned from the GUI app
    // it would flash a console window in front of Velox on EVERY media add. These
    // flags suppress the window while stdout/stderr stay fully piped (diagnostics
    // are still captured). No effect on other platforms.
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

    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        EngineError::new(
            ErrorKind::InvalidResponse,
            format!("extractor returned unparseable metadata: {e}"),
        )
    })?;

    let title = meta
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("media")
        .to_string();
    let plan = pick_plan(&meta)?;
    let info = match &plan.audio {
        // Fast path: progressive stream (video+audio in ONE file) — download directly.
        None => ResourceInfo {
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
            ..Default::default()
        },
        // Adaptive: separate video/audio https streams → two-stream media job.
        Some(audio) => {
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
                }),
                ..Default::default()
            }
        }
    };
    Ok(info)
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
#[derive(Debug)]
struct MediaPlan {
    video: ChosenStream,
    audio: Option<ChosenStream>,
}

/// Pick the best stream set from yt-dlp metadata. Preference order:
///
/// 1. PROGRESSIVE https stream (video+audio in one file) — the fast path, no merge.
///    mp4 preferred over webm, then highest total bitrate.
/// 2. Best ADAPTIVE PAIR: highest-bitrate video-only stream + highest-bitrate
///    audio-only stream (mp4/m4a preferred within each). Container compatibility is
///    decided by `merge_container` — codecs are never transcoded.
///
/// HLS/DASH manifest protocols (m3u8_native, dash, …) are rejected in every branch:
/// Velox downloads files, not segment manifests. A media with only manifest formats
/// or only one adaptive half fails honestly.
fn pick_plan(meta: &serde_json::Value) -> Result<MediaPlan, EngineError> {
    let mut best_prog: Option<(usize, f64, ChosenStream)> = None;
    let mut best_video: Option<(usize, f64, ChosenStream)> = None;
    let mut best_audio: Option<(usize, f64, ChosenStream)> = None;
    if let Some(formats) = meta.get("formats").and_then(|f| f.as_array()) {
        for f in formats {
            let Some(stream_url) = f.get("url").and_then(|u| u.as_str()) else {
                continue;
            };
            let proto = f.get("protocol").and_then(|p| p.as_str()).unwrap_or("");
            if !matches!(proto, "https" | "http") {
                continue; // m3u8/dash manifests are not file downloads
            }
            let vcodec = f.get("vcodec").and_then(|c| c.as_str()).unwrap_or("none");
            let acodec = f.get("acodec").and_then(|c| c.as_str()).unwrap_or("none");
            let ext = f
                .get("ext")
                .and_then(|e| e.as_str())
                .unwrap_or("mp4")
                .to_string();
            let size = f
                .get("filesize")
                .and_then(|s| s.as_u64())
                .or_else(|| f.get("filesize_approx").and_then(|s| s.as_u64()));
            let tbr = f.get("tbr").and_then(|t| t.as_f64()).unwrap_or(0.0);
            let mime = f
                .get("mime_type")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| match ext.as_str() {
                    "webm" => "video/webm".into(),
                    "mkv" => "video/x-matroska".into(),
                    "m4a" => "audio/mp4".into(),
                    _ => "video/mp4".into(),
                });
            let format_id = f
                .get("format_id")
                .and_then(|i| i.as_str())
                .unwrap_or("unknown")
                .to_string();
            let chosen = ChosenStream {
                url: stream_url.to_string(),
                ext: ext.clone(),
                mime,
                size,
                format_id,
            };
            if vcodec != "none" && acodec != "none" {
                // progressive candidate
                let pref = match ext.as_str() {
                    "mp4" => 2,
                    "webm" => 1,
                    _ => 0,
                };
                if best_prog
                    .as_ref()
                    .is_none_or(|(bp, bt, _)| (pref, tbr) > (*bp, *bt))
                {
                    best_prog = Some((pref, tbr, chosen));
                }
            } else if vcodec != "none" {
                // video-only candidate
                let pref = match ext.as_str() {
                    "mp4" => 2,
                    "webm" => 1,
                    _ => 0,
                };
                if best_video
                    .as_ref()
                    .is_none_or(|(bp, bt, _)| (pref, tbr) > (*bp, *bt))
                {
                    best_video = Some((pref, tbr, chosen));
                }
            } else if acodec != "none" {
                // audio-only candidate: m4a pairs with mp4, then bitrate
                let pref = match ext.as_str() {
                    "m4a" => 3,
                    "mp3" => 2,
                    "aac" => 2,
                    "webm" => 1,
                    _ => 0,
                };
                let rank = f
                    .get("abr")
                    .and_then(|t| t.as_f64())
                    .filter(|v| *v > 0.0)
                    .unwrap_or(tbr);
                if best_audio
                    .as_ref()
                    .is_none_or(|(bp, bt, _)| (pref, rank) > (*bp, *bt))
                {
                    best_audio = Some((pref, rank, chosen));
                }
            }
        }
    }

    // some pages expose a single direct stream at the top level
    if best_prog.is_none() {
        if let Some(stream_url) = meta.get("url").and_then(|u| u.as_str()) {
            let proto = meta.get("protocol").and_then(|p| p.as_str()).unwrap_or("");
            if matches!(proto, "https" | "http") {
                let ext = meta
                    .get("ext")
                    .and_then(|e| e.as_str())
                    .unwrap_or("mp4")
                    .to_string();
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
                return Ok(MediaPlan { video, audio: None });
            }
        }
    }

    if let Some((_, _, video)) = best_prog {
        return Ok(MediaPlan { video, audio: None });
    }
    match (best_video, best_audio) {
        (Some((_, _, v)), Some((_, _, a))) => Ok(MediaPlan {
            video: v,
            audio: Some(a),
        }),
        (Some((_, _, _)), None) => Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "only video-only streams are available for this media — no audio stream to              pair with them; the media may need authentication or is not downloadable",
        )),
        (None, Some((_, _, _))) => Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "only audio-only streams are available for this media — no video stream to              pair with them",
        )),
        (None, None) => Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "no downloadable https media stream found for this URL — only manifest              formats (HLS/DASH), which are not file downloads",
        )),
    }
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
        let plan = pick_plan(&meta).unwrap();
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
        let plan = pick_plan(&meta).unwrap();
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
        let e = pick_plan(&meta).unwrap_err();
        assert_eq!(e.kind_code(), "UNSUPPORTED_URL");
        assert!(e.message.contains("audio"), "{e}");
    }

    #[test]
    fn manifest_only_is_an_honest_error() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [fmt("hls-x", "mp4", "m3u8", "h264", "aac", 5000.0, 1)]
        });
        let e = pick_plan(&meta).unwrap_err();
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
        let plan = pick_plan(&meta).unwrap();
        assert!(plan.audio.is_none());
        let c = plan.video;
        assert_eq!(c.url, "https://media.example/v.mp4");
        assert_eq!(c.size, Some(123));
    }
}
