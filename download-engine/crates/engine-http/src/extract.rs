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
//!   content is refused. Only progressive https media streams are consumed (video+audio
//!   in one file) — adaptive streams needing ffmpeg merging are reported honestly.
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
    let chosen = pick_stream(&meta)?;
    let suggested_name = Some(format!("{}.{}", title, chosen.ext));

    Ok(ResourceInfo {
        final_url: chosen.url,
        redirect_chain: vec![url.to_string()],
        status: 200,
        content_length: chosen.size,
        range_support: engine_core::inspect::RangeSupport::Unknown,
        mime: Some(chosen.mime),
        sniffed: engine_core::inspect::SniffedKind::Unknown,
        suggested_name,
        resolve_note: Some(format!(
            "media extractor: “{}” [{}]",
            title, chosen.format_id
        )),
        ..Default::default()
    })
}

/// Pick the best progressive https stream: video+audio in ONE file (no ffmpeg merge),
/// mp4 preferred over webm, then highest bitrate. HLS/DASH manifests are rejected.
fn pick_stream(meta: &serde_json::Value) -> Result<ChosenStream, EngineError> {
    let mut best: Option<(usize, f64, ChosenStream)> = None;
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
            if vcodec == "none" || acodec == "none" {
                continue; // adaptive halves need ffmpeg — out of scope, reported honestly
            }
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
                    _ => "video/mp4".into(),
                });
            let format_id = f
                .get("format_id")
                .and_then(|i| i.as_str())
                .unwrap_or("unknown")
                .to_string();
            let pref = match ext.as_str() {
                "mp4" => 2,
                "webm" => 1,
                _ => 0,
            };
            if best
                .as_ref()
                .is_none_or(|(bp, bt, _)| (pref, tbr) > (*bp, *bt))
            {
                best = Some((
                    pref,
                    tbr,
                    ChosenStream {
                        url: stream_url.to_string(),
                        ext,
                        mime,
                        size,
                        format_id,
                    },
                ));
            }
        }
    }

    // some pages expose a single direct stream at the top level
    if best.is_none() {
        if let Some(stream_url) = meta.get("url").and_then(|u| u.as_str()) {
            let proto = meta.get("protocol").and_then(|p| p.as_str()).unwrap_or("");
            if matches!(proto, "https" | "http") {
                let ext = meta
                    .get("ext")
                    .and_then(|e| e.as_str())
                    .unwrap_or("mp4")
                    .to_string();
                return Ok(ChosenStream {
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
                });
            }
        }
    }

    best.map(|(_, _, c)| c).ok_or_else(|| {
        EngineError::new(
            ErrorKind::UnsupportedUrl,
            "only adaptive (video-only/audio-only) streams are available for this media; \
                 merging them requires ffmpeg, which Velox does not bundle — the engine \
                 downloads single-file progressive streams only",
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
        let c = pick_stream(&meta).unwrap();
        assert_eq!(c.format_id, "mp4-360", "progressive https, mp4 preferred");
        assert_eq!(c.mime, "video/mp4");
        assert_eq!(c.size, Some(3_500_000));
    }

    #[test]
    fn adaptive_only_is_an_honest_error() {
        let meta = serde_json::json!({
            "title": "Test",
            "formats": [fmt("v-only", "mp4", "https", "avc1", "none", 4000.0, 1)]
        });
        let e = pick_stream(&meta).unwrap_err();
        assert_eq!(e.kind_code(), "UNSUPPORTED_URL");
        assert!(e.message.contains("ffmpeg"));
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
        let c = pick_stream(&meta).unwrap();
        assert_eq!(c.url, "https://media.example/v.mp4");
        assert_eq!(c.size, Some(123));
    }
}
