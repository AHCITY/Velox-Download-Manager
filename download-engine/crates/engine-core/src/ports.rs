//! Transport port: the core depends on this trait only; engine-http implements it on reqwest.
//! Tests can mock it. Headers/auth flow through `RequestHeaders` so the backend can apply
//! host-scoped Authorization rules (RFC 9110 §15.4) and cookies.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bytes::Bytes;

use crate::config::AuthConfig;
use crate::error::{EngineError, ErrorKind};

#[derive(Debug, Clone, Default)]
pub struct RequestHeaders {
    /// Extra headers sent verbatim.
    pub extra: BTreeMap<String, String>,
    pub referer: Option<String>,
    /// Cookies for the initial request ("name=value").
    pub cookies: BTreeMap<String, String>,
    pub auth: Option<AuthConfig>,
    /// User-Agent override; backend falls back to EngineConfig.user_agent.
    pub user_agent: Option<String>,
    /// Strong ETag/HTTP-date for a conditional inspection probe (resume validation).
    pub if_range: Option<String>,
    /// Explicit per-download TLS verification opt-out (SECURITY §3) — forwarded to the transport.
    pub insecure_tls: bool,
}

impl RequestHeaders {
    pub fn from_config(
        extra: &std::collections::BTreeMap<String, String>,
        referer: &Option<String>,
        cookies: &std::collections::BTreeMap<String, String>,
        auth: &Option<AuthConfig>,
        insecure_tls: bool,
    ) -> Self {
        Self {
            extra: extra.clone(),
            referer: referer.clone(),
            cookies: cookies.clone(),
            auth: auth.clone(),
            user_agent: None,
            if_range: None,
            insecure_tls,
        }
    }

    /// Audit D3 (2026-11-21): explicit credentials must not cross origins at RESOLVE
    /// level. `follow()` already scopes across redirects; this scopes the header set
    /// itself when the engine hands it to a different origin than the user's original
    /// URL — Drive confirm-walk targets and extractor-resolved stream URLs are
    /// page-controlled, so a hostile page could otherwise harvest the user's
    /// Cookie/Authorization for the site they came from. The RFC 6265 cookie jar keeps
    /// its own domain scoping and is intentionally untouched (standard browser
    /// behavior); referer is kept (identifies the referring page, carries no secret).
    pub fn scoped_credentials(mut self, original_url: &str, target_url: &str) -> Self {
        if origin_triple(original_url) == origin_triple(target_url) {
            return self;
        }
        self.cookies.clear();
        self.auth = None;
        self.extra.clear();
        self
    }
}

/// Scheme + host + effective port — the credential-scoping origin. Mirrors
/// `engine-http::origin_of` (kept in sync; tested in both crates).
pub fn origin_triple(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let port = u
                .port_or_known_default()
                .map(|p| p.to_string())
                .unwrap_or_default();
            format!("{}|{}|{}", u.scheme(), u.host_str().unwrap_or(""), port)
        }
        Err(_) => url.to_string(),
    }
}

/// One open-ended chunk of a response body.
#[async_trait]
pub trait ByteStream: Send {
    /// Next chunk of body bytes, or None when the body ended cleanly.
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, EngineError>;
}

pub type BodyStream = Box<dyn ByteStream + Send>;

pub struct RangeRequest {
    pub url: String,
    /// Inclusive start offset in the (identity-encoded) representation.
    pub start: u64,
    /// Inclusive end offset; None = open-ended (`bytes=start-`).
    pub end: Option<u64>,
    /// Strong ETag or HTTP-date for If-Range (core guarantees only strong forms are passed).
    pub if_range: Option<String>,
    pub headers: RequestHeaders,
    /// Probe proved ranges unsupported and nothing is written yet → plain GET
    /// (RFC 9110 §14.2: a server MAY ignore Range; the 200 full body is exactly what we want).
    pub no_range_header: bool,
}

pub struct RangeResponse {
    pub status: u16,
    /// Parsed Content-Range (start, end, total) — total may be `*`.
    pub content_range: Option<(u64, u64, Option<u64>)>,
    pub content_length: Option<u64>,
    pub etag: Option<String>,
    /// Retry-After from the response (e.g. 429/503 on a live range request).
    pub retry_after: Option<std::time::Duration>,
    pub body: BodyStream,
}

pub struct Inspection {
    pub info: crate::inspect::ResourceInfo,
}

#[async_trait]
pub trait HttpPort: Send + Sync {
    /// Downcast support (backend-specific extensions like cookie import/export).
    fn as_any(&self) -> &dyn std::any::Any;
    /// Inspect a resource: manual-redirect ranged GET probe (`Range: bytes=0-1023`), full
    /// chain recorded. Implementations must not consume large bodies and SHOULD sniff the
    /// first bytes into `ResourceInfo::sniffed`.
    async fn inspect(
        &self,
        url: &str,
        headers: &RequestHeaders,
    ) -> Result<crate::inspect::ResourceInfo, EngineError>;

    /// Resolve URL semantics beyond plain HTTP: web-page/interstitial handling (Google
    /// Drive confirm pages), media-page extraction via a configured external extractor,
    /// and body sniffing. The default implementation is a plain inspect; the core
    /// (manager, task) calls THIS method everywhere it used to call inspect, so backends
    /// opt into richer semantics without any core change.
    async fn resolve(
        &self,
        url: &str,
        headers: &RequestHeaders,
    ) -> Result<crate::inspect::ResourceInfo, EngineError> {
        self.inspect(url, headers).await
    }

    /// Format-aware media resolution (Add dialog format/quality choice).
    /// Backends with an extractor (engine-http) honour the format, height cap
    /// and fps preference; the default ignores them (plain resolve).
    async fn resolve_with_format(
        &self,
        url: &str,
        headers: &RequestHeaders,
        _choice: Option<crate::config::MediaFormat>,
        _max_height: Option<u32>,
        _want_fps: Option<u32>,
    ) -> Result<crate::inspect::ResourceInfo, EngineError> {
        self.resolve(url, headers).await
    }

    /// Probe a media page's available formats (yt-dlp -J simulate; honest
    /// quality list). Backends without an extractor return an error.
    async fn probe_media_formats(
        &self,
        _url: &str,
        _headers: &RequestHeaders,
    ) -> Result<serde_json::Value, EngineError> {
        Err(EngineError::new(
            ErrorKind::UnsupportedUrl,
            "format listing requires the media extractor (yt-dlp) — configure it first",
        ))
    }

    /// Open a ranged GET. Backends must follow redirects manually per request (chain transparent to core),
    /// preserve Range/If-Range across hops, and only re-apply Authorization while the host is unchanged.
    async fn open_range(&self, req: RangeRequest) -> Result<RangeResponse, EngineError>;
}

/// MediaPipeline seam (architecture review §9): the `-c copy` merge stage lives
/// OUTSIDE the transfer engine — engine-media implements this and injects it via
/// `DownloadManager::set_media_merge`. Contract for implementations: LGPL-gated
/// ffmpeg subprocess, kill-safe on `kill`, ffprobe post-validation, honest errors
/// (`MISSING_FFMPEG`/`GPL_BUILD`/`MERGE_FAILED`/`CORRUPT_MEDIA`), and `Merging`/`Merged`
/// events on the given bus. Docs: docs/FFMPEG_INTEGRATION.md.
/// BitTorrent/magnet support (engine-torrent). Same injection pattern as
/// `MediaMergePort`: the transfer engine never links a torrent library — the
/// host wires an implementation in, and the manager routes rows with
/// `mode == "torrent"` to it. `source` is a magnet URI, a local .torrent path,
/// or an http(s) URL to a .torrent file.
#[async_trait]
pub trait TorrentPort: Send + Sync {
    /// Attach the BitTorrent engine to an existing row (the row was created by
    /// the manager; `id` is the row id). Engine-owned from here on.
    async fn add(&self, id: i64, source: &str, out_dir: &std::path::Path)
        -> Result<(), EngineError>;

    /// Pause: stop the swarm traffic, keep the partial pieces on disk.
    async fn pause(&self, id: i64) -> Result<(), EngineError>;

    /// Resume a paused torrent (row already exists).
    async fn resume(&self, id: i64) -> Result<(), EngineError>;

    /// Detach the torrent from the session; `delete_files` also removes the
    /// downloaded pieces.
    async fn remove(&self, id: i64, delete_files: bool) -> Result<(), EngineError>;
}

#[async_trait]
pub trait MediaMergePort: Send + Sync {
    /// Stream-copy `video`+`audio` into `output` (the manager owns the output path).
    /// On cancel (kill trigger) implementations return `Cancelled` and must have
    /// killed the child. Never claim success without post-validation.
    async fn merge(
        &self,
        id: i64,
        video: &std::path::Path,
        audio: &std::path::Path,
        output: &std::path::Path,
        bus: &crate::events::EventBus,
        kill: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), EngineError>;

    /// Convert a completed single-stream download to MP3 (audio extraction,
    /// libmp3lame VBR best). Same contract as merge: LGPL-gated, kill-safe,
    /// honest errors, never claim success without the output existing.
    async fn convert_to_mp3(
        &self,
        id: i64,
        input: &std::path::Path,
        output: &std::path::Path,
        bus: &crate::events::EventBus,
        kill: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), EngineError> {
        let _ = (id, input, output, bus, kill);
        Err(EngineError::new(
            ErrorKind::MergeFailed,
            "media pipeline does not support MP3 conversion",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_credentials_same_origin_is_identity() {
        let h = RequestHeaders::from_config(
            &std::iter::once(("X-Session".to_string(), "s".to_string())).collect(),
            &Some("https://a.example/page".into()),
            &std::iter::once(("sid".to_string(), "secret".to_string())).collect(),
            &Some(AuthConfig::Bearer {
                token: "tok".into(),
            }),
            false,
        );
        let scoped = h
            .clone()
            .scoped_credentials("https://a.example/page", "https://a.example:443/other");
        assert_eq!(scoped.cookies.len(), 1, "same origin: cookies kept");
        assert!(scoped.auth.is_some());
        assert_eq!(scoped.extra.len(), 1);
    }

    #[test]
    fn scoped_credentials_cross_origin_strips_secrets() {
        let h = RequestHeaders::from_config(
            &std::iter::once(("X-Session".to_string(), "s".to_string())).collect(),
            &Some("https://a.example/page".into()),
            &std::iter::once(("sid".to_string(), "secret".to_string())).collect(),
            &Some(AuthConfig::Bearer {
                token: "tok".into(),
            }),
            false,
        );
        let scoped = h.scoped_credentials("https://a.example/page", "https://evil.example/confirm");
        assert!(scoped.cookies.is_empty(), "cookies must not cross origins");
        assert!(
            scoped.auth.is_none(),
            "authorization must not cross origins"
        );
        assert!(
            scoped.extra.is_empty(),
            "arbitrary headers may carry secrets"
        );
        // transport-neutral parts stay
        assert_eq!(scoped.referer.as_deref(), Some("https://a.example/page"));
        assert!(!scoped.insecure_tls);
    }

    #[test]
    fn origin_triple_matches_engine_http_semantics() {
        assert_ne!(
            origin_triple("https://a.example/f"),
            origin_triple("http://a.example/f")
        );
        assert_ne!(
            origin_triple("https://a.example/f"),
            origin_triple("https://a.example:8443/f")
        );
        assert_eq!(
            origin_triple("https://a.example/f"),
            origin_triple("https://a.example:443/g")
        );
    }
}
