//! URL adapter layer (architecture review §3).
//!
//! Narrow, tested seam for URL parsing/validation/classification. Today the only
//! adapter reproduces the manager's URL gate verbatim (http/https only, host
//! required, no embedded credentials). Media-manifest detection arrives with the
//! MediaPipeline milestone and must keep this trait as its only extension point —
//! do not grow `DownloadManager` URL logic inline.

use crate::error::{EngineError, ErrorKind};

/// How the engine plans a validated URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlKind {
    /// Single resource with byte-range planning (the only kind today).
    Direct,
    /// Media manifest/playlist (DASH/HLS); consumed by the future MediaPipeline.
    Media,
}

/// Adapter contract for URL parsing/validation/classification. No I/O.
pub trait UrlAdapter: Send + Sync {
    /// Parse and validate a URL, returning the parsed URL.
    fn parse(&self, url: &str) -> Result<url::Url, EngineError>;
    /// Classify a validated URL for planning purposes.
    fn classify(&self, url: &url::Url) -> UrlKind;
}

/// Default adapter: the manager's URL gate, verbatim.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpUrlAdapter;

impl UrlAdapter for HttpUrlAdapter {
    fn parse(&self, url: &str) -> Result<url::Url, EngineError> {
        let parsed = url::Url::parse(url)
            .map_err(|e| EngineError::new(ErrorKind::InvalidUrl, format!("invalid url: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(EngineError::new(
                ErrorKind::InvalidUrl,
                format!(
                    "unsupported scheme {:?} (engine supports http/https)",
                    parsed.scheme()
                ),
            ));
        }
        if parsed.host_str().is_none() || parsed.host_str() == Some("") {
            return Err(EngineError::new(ErrorKind::InvalidUrl, "URL has no host"));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(EngineError::new(
                ErrorKind::InvalidUrl,
                "URL contains embedded credentials; use the auth config instead",
            ));
        }
        Ok(parsed)
    }

    fn classify(&self, _url: &url::Url) -> UrlKind {
        // No behavior change in this refactor: everything plans as Direct.
        // Media detection (DASH/HLS manifests) lands with the MediaPipeline and
        // extends this method with its own tests.
        UrlKind::Direct
    }
}

/// Module-level convenience: validate via the default adapter (static unit struct,
/// no per-call allocation).
pub fn validate(url: &str) -> Result<url::Url, EngineError> {
    static ADAPTER: HttpUrlAdapter = HttpUrlAdapter;
    ADAPTER.parse(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn err_of(url: &str) -> String {
        validate(url).unwrap_err().message
    }

    #[test]
    fn accepts_http_and_https() {
        assert!(validate("http://example.com/file").is_ok());
        assert!(validate("https://example.com/file").is_ok());
    }

    #[test]
    fn rejects_unsupported_schemes() {
        let msg = err_of("ftp://example.com/file");
        assert!(msg.contains("unsupported scheme"), "{msg}");
        assert!(msg.contains("http/https"), "{msg}");
        assert_eq!(err_of("file:///etc/passwd"), msg_of_scheme("file"));
        assert_eq!(err_of("ws://example.com/"), msg_of_scheme("ws"));
    }

    fn msg_of_scheme(s: &str) -> String {
        format!("unsupported scheme \"{s}\" (engine supports http/https)")
    }

    #[test]
    fn rejects_invalid_urls() {
        assert!(err_of("not a url").contains("invalid url"));
        assert!(err_of("").contains("invalid url"));
    }

    #[test]
    fn rejects_missing_host() {
        // The url crate rejects empty hosts for special schemes at parse time; the
        // "URL has no host" branch stays as a defensive verbatim port (unreachable
        // for http/https via this crate today).
        assert!(err_of("http://").contains("invalid url"));
        // Lenient parse parity with the old manager gate: `https:///path` parses
        // with host "path" and is accepted — same behavior before the refactor.
        let u = validate("https:///path").unwrap();
        assert_eq!(u.host_str(), Some("path"));
    }

    #[test]
    fn rejects_embedded_credentials() {
        assert_eq!(
            err_of("https://user:pass@example.com/file"),
            "URL contains embedded credentials; use the auth config instead"
        );
        assert_eq!(
            err_of("https://user@example.com/file"),
            "URL contains embedded credentials; use the auth config instead"
        );
    }

    #[test]
    fn userinfo_check_tolerates_empty_password() {
        // `user@` has an empty username but no password — still rejected by username.
        assert!(validate("https://:pass@example.com/f").is_err());
    }

    #[test]
    fn classify_is_direct_today() {
        let a = HttpUrlAdapter;
        let u = validate("https://example.com/video.m3u8").unwrap();
        assert_eq!(a.classify(&u), UrlKind::Direct);
    }

    #[test]
    fn trait_object_and_shared_usage() {
        let a: Arc<dyn UrlAdapter> = Arc::new(HttpUrlAdapter);
        assert!(a.parse("https://example.com/f").is_ok());
        assert!(a.parse("ftp://example.com/f").is_err());
    }
}
