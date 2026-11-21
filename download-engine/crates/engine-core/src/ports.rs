//! Transport port: the core depends on this trait only; engine-http implements it on reqwest.
//! Tests can mock it. Headers/auth flow through `RequestHeaders` so the backend can apply
//! host-scoped Authorization rules (RFC 9110 §15.4) and cookies.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bytes::Bytes;

use crate::config::AuthConfig;
use crate::error::EngineError;

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

    /// Open a ranged GET. Backends must follow redirects manually per request (chain transparent to core),
    /// preserve Range/If-Range across hops, and only re-apply Authorization while the host is unchanged.
    async fn open_range(&self, req: RangeRequest) -> Result<RangeResponse, EngineError>;
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
