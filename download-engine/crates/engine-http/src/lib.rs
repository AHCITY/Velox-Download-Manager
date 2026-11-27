//! Velox HTTP/HTTPS transport: reqwest 0.13 + rustls(ring), manual redirect handling.
//!
//! Design notes (ADR-0002): redirects are followed manually so the engine can record the chain,
//! detect loops, preserve Range/If-Range and scope Authorization to the original host.
//! Decompression is not enabled — the bytes we store are exactly the bytes the server sent.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;

use engine_core::config::{AuthConfig, EngineConfig};
use engine_core::error::{EngineError, ErrorKind};
use engine_core::inspect::{
    parse_retry_after, sniff_kind, RangeSupport, ResourceInfo, SniffedKind,
};
use engine_core::ports::{ByteStream, HttpPort, RangeRequest, RangeResponse, RequestHeaders};

const MAX_REDIRECTS: usize = 10;
/// Probe body prefix read for content sniffing (and the Range asked for it).
const SNIFF_PROBE_LEN: usize = 1024;
/// Confirm/interstitial pages are small; this bound keeps the fetch finite.
const MAX_PAGE_BYTES: usize = 512 * 1024;

pub mod extract;
pub mod formats;
pub mod gdrive;
pub mod netscape;

type SharedCookieStore = std::sync::Arc<reqwest_cookie_store::CookieStoreMutex>;

pub struct ReqwestBackend {
    default_client: reqwest::Client,
    insecure_client: Mutex<Option<reqwest::Client>>,
    engine: EngineConfig,
    /// Persistent cookie jar (RFC 6265 via cookie_store); None = per-client in-memory store.
    cookie_jar: Option<(std::path::PathBuf, SharedCookieStore)>,
}

impl ReqwestBackend {
    pub fn new(cfg: &EngineConfig) -> Result<Self, EngineError> {
        let cookie_jar = load_or_empty_jar(cfg);
        let default_client = build_client(cfg, false, cookie_jar.as_ref().map(|(_, s)| s.clone()))?;
        // periodic persistence: cookie_store has no change callback, so a low-frequency
        // std thread keeps the file fresh (0600 — cookies may carry session tokens)
        if let Some((path, store)) = cookie_jar.clone() {
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                save_cookie_jar(&path, &store);
            });
        }
        Ok(Self {
            default_client,
            insecure_client: Mutex::new(None),
            engine: cfg.clone(),
            cookie_jar,
        })
    }

    /// Persist the cookie jar now (also runs on Drop and every 30 s).
    /// Access the shared cookie store for import/export (None = no persistent jar).
    pub fn cookie_store(&self) -> Option<&SharedCookieStore> {
        self.cookie_jar.as_ref().map(|(_, s)| s)
    }

    pub fn save_cookies(&self) {
        if let Some((path, store)) = &self.cookie_jar {
            save_cookie_jar(path, store);
        }
    }

    fn client_for(&self, insecure: bool) -> reqwest::Client {
        if !insecure {
            return self.default_client.clone();
        }
        let mut g = self.insecure_client.lock().unwrap();
        if let Some(c) = g.as_ref() {
            return c.clone();
        }
        let c = build_client(
            &self.engine,
            true,
            self.cookie_jar.as_ref().map(|(_, s)| s.clone()),
        )
        .expect("insecure client build");
        *g = Some(c.clone());
        c
    }

    /// GET with manual redirect following; returns (response, hop chain, final URL).
    async fn follow(
        &self,
        client: &reqwest::Client,
        url: &str,
        hdrs: &RequestHeaders,
        range: Option<(u64, Option<u64>)>,
    ) -> Result<(reqwest::Response, Vec<String>, String), EngineError> {
        let original_origin = origin_of(url);
        let mut current = url.to_string();
        let mut chain: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(current.clone());

        for _ in 0..MAX_REDIRECTS {
            let same_origin = origin_of(&current) == original_origin;
            let mut rb = client.get(&current);
            if let Some(ua) = &hdrs.user_agent {
                rb = rb.header("User-Agent", ua);
            }
            // P0: the bytes stored must be the representation bytes. Identity is requested
            // on EVERY request; a server that compresses anyway fails the content-encoding
            // gate instead of silently corrupting the file.
            rb = rb.header("Accept-Encoding", "identity");
            // RFC 9110 §15.4: origin-specific and resource-specific credentials must not be
            // forwarded to a different origin. Cross-origin hops keep only transport-neutral
            // headers (User-Agent) plus If-Range/Range, which validate the resource itself.
            // Same-origin hops keep the full header set (browser practice; documented in
            // PROTOCOL_COMPATIBILITY §3 — the strict-RFC variant would strip auth even
            // same-origin and break common CDN login redirects).
            if same_origin {
                if let Some(referer) = &hdrs.referer {
                    rb = rb.header("Referer", referer);
                }
                if !hdrs.cookies.is_empty() {
                    rb = rb.header("Cookie", cookie_string(&hdrs.cookies));
                }
                for (k, v) in &hdrs.extra {
                    rb = rb.header(k.as_str(), v.as_str());
                }
                if let Some(auth) = &hdrs.auth {
                    rb = match auth {
                        AuthConfig::Basic { user, password } => rb.basic_auth(user, Some(password)),
                        AuthConfig::Bearer { token } => rb.bearer_auth(token),
                    };
                }
            }
            if let Some(ir) = &hdrs.if_range {
                rb = rb.header("If-Range", ir);
            }
            if let Some((s, e)) = range {
                rb = rb.header(
                    "Range",
                    match e {
                        Some(e) => format!("bytes={s}-{e}"),
                        None => format!("bytes={s}-"),
                    },
                );
            }
            let resp = rb.send().await.map_err(|e| map_transport(&e))?;
            if resp.status().is_redirection() {
                let loc = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        EngineError::new(
                            ErrorKind::InvalidResponse,
                            "redirect without a Location header",
                        )
                    })?;
                drop(resp); // close: the aborted body must never be reused (RFC 9112 §9.3)
                chain.push(current.clone());
                let next = url::Url::parse(&current)
                    .and_then(|u| u.join(&loc))
                    .map_err(|e| {
                        EngineError::new(ErrorKind::InvalidResponse, format!("bad redirect: {e}"))
                    })?
                    .to_string();
                if !seen.insert(next.clone()) {
                    return Err(EngineError::new(
                        ErrorKind::InvalidResponse,
                        format!("redirect loop detected at {next}"),
                    ));
                }
                current = next;
                continue;
            }
            return Ok((resp, chain, current));
        }
        Err(EngineError::new(
            ErrorKind::InvalidResponse,
            format!("more than {MAX_REDIRECTS} redirects"),
        ))
    }
}

impl ReqwestBackend {
    /// Marker-guarded confirm-page walk for arbitrary hosts: when a sniffed HTML page
    /// carries a Google Drive confirm form (`Download anyway`/uuid+confirm inputs),
    /// follow it once and probe the target. None = not a confirm page (caller keeps
    /// the original HTML response; the page gate will reject it).
    async fn try_confirm_walk(
        &self,
        url: &str,
        hdrs: &RequestHeaders,
    ) -> Result<Option<ResourceInfo>, EngineError> {
        let page = self.fetch_page_body(url, hdrs).await?;
        let Some(confirm_url) = gdrive::parse_confirm_page(&page, url) else {
            return Ok(None);
        };
        // audit D3: the confirm target is page-controlled — explicit credentials from
        // the original site must not flow to a different origin (the RFC 6265 jar keeps
        // its own domain scoping)
        let scoped = hdrs.clone().scoped_credentials(url, &confirm_url);
        let info = self.inspect(&confirm_url, &scoped).await?;
        if info.sniffed == SniffedKind::Html {
            // still a page after confirmation: not downloadable — return the original
            // info so the standard page gate produces its honest failure
            return Ok(None);
        }
        let mut info = info;
        info.redirect_chain.insert(0, url.to_string());
        info.resolve_note = Some("download confirmed via interstitial page".into());
        Ok(Some(info))
    }

    /// Bounded fetch of a page body as text (Drive confirm interstitials are ~10-100 KiB).
    async fn fetch_page_body(
        &self,
        url: &str,
        hdrs: &RequestHeaders,
    ) -> Result<String, EngineError> {
        let client = self.client_for(hdrs.insecure_tls);
        let (resp, _chain, _final_url) = self.follow(&client, url, hdrs, None).await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            return Err(engine_core::error::from_status(
                status,
                "failed to fetch the interstitial page",
            ));
        }
        let mut resp = resp;
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        while buf.len() < MAX_PAGE_BYTES {
            match tokio::time::timeout(self.engine.stall_timeout, resp.chunk()).await {
                Ok(Ok(Some(chunk))) => buf.extend_from_slice(&chunk),
                _ => break,
            }
        }
        Ok(String::from_utf8_lossy(&buf[..buf.len().min(MAX_PAGE_BYTES)]).into_owned())
    }

    /// Google Drive semantics: rewrite `/file/d/<id>/view` share links to the download
    /// endpoint, serve small files directly, and walk the virus-scan confirm interstitial
    /// for large files (form action + hidden inputs, `Download anyway` anchor fallback).
    /// Private/quota'd/removed files fail with a precise, honest message.
    async fn resolve_gdrive(
        &self,
        file_id: &str,
        original_url: &str,
        hdrs: &RequestHeaders,
    ) -> Result<ResourceInfo, EngineError> {
        let target = gdrive::direct_download_url(file_id);
        let mut info = self.inspect(&target, hdrs).await?;
        if info.sniffed == SniffedKind::Html {
            let page = self.fetch_page_body(&target, hdrs).await?;
            match gdrive::parse_confirm_page(&page, &target) {
                Some(confirm_url) => {
                    // audit D3: page-controlled target — scope explicit credentials
                    let scoped = hdrs.clone().scoped_credentials(&target, &confirm_url);
                    info = self.inspect(&confirm_url, &scoped).await?;
                    if info.sniffed == SniffedKind::Html {
                        return Err(EngineError::new(
                            ErrorKind::NotAFile,
                            format!(
                                "Google Drive still returned a web page after the download \
                                 confirmation: {}",
                                gdrive::page_reason(&page)
                            ),
                        ));
                    }
                    info.resolve_note =
                        Some("Google Drive: confirmed via the download interstitial".into());
                }
                None => {
                    return Err(EngineError::new(
                        ErrorKind::NotAFile,
                        format!(
                            "Google Drive returned a web page, not a file. {}",
                            gdrive::page_reason(&page)
                        ),
                    ));
                }
            }
        }
        // the share link the user entered stays on the record; final_url is the download hop
        info.redirect_chain.insert(0, original_url.to_string());
        Ok(info)
    }
}

#[async_trait]
impl HttpPort for ReqwestBackend {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve(&self, url: &str, hdrs: &RequestHeaders) -> Result<ResourceInfo, EngineError> {
        match gdrive::classify_url(url) {
            gdrive::UrlKind::GDrive { id } => self.resolve_gdrive(&id, url, hdrs).await,
            gdrive::UrlKind::Media => extract::resolve_media(self, url, hdrs).await,
            gdrive::UrlKind::Other => {
                let info = self.inspect(url, hdrs).await?;
                if info.sniffed == SniffedKind::Html {
                    // generic, marker-guarded confirm-page walk: covers Drive confirm
                    // interstitials served from any host (mirrors/proxies) and keeps the
                    // regression fixture honest. Pages WITHOUT a Drive confirm marker are
                    // returned untouched — the core page gate rejects them.
                    if let Some(walked) = self.try_confirm_walk(url, hdrs).await? {
                        return Ok(walked);
                    }
                }
                Ok(info)
            }
        }
    }
    async fn resolve_with_format(
        &self,
        url: &str,
        hdrs: &RequestHeaders,
        choice: Option<engine_core::config::MediaFormat>,
        max_height: Option<u32>,
        want_fps: Option<u32>,
    ) -> Result<ResourceInfo, EngineError> {
        use engine_core::config::MediaFormat;
        // Direct-file URLs (and Drive pages) keep plain resolve semantics; only
        // media pages take the format-aware path.
        if crate::gdrive::classify_url(url) != crate::gdrive::UrlKind::Media || choice.is_none() {
            return self.resolve(url, hdrs).await;
        }
        let ch = match choice {
            Some(MediaFormat::Mp4) => crate::formats::MediaFormatChoice::Mp4,
            Some(MediaFormat::Mp3) => crate::formats::MediaFormatChoice::Mp3,
            None => unreachable!(),
        };
        crate::extract::resolve_media_with(self, url, hdrs, Some(ch), max_height, want_fps).await
    }

    async fn probe_media_formats(
        &self,
        url: &str,
        hdrs: &RequestHeaders,
    ) -> Result<serde_json::Value, EngineError> {
        if crate::gdrive::classify_url(url) != crate::gdrive::UrlKind::Media {
            return Err(EngineError::new(
                ErrorKind::UnsupportedUrl,
                "this URL is not a media page — format listing applies to YouTube-style pages",
            ));
        }
        let sum = crate::extract::probe_formats(self, url, hdrs).await?;
        serde_json::to_value(&sum).map_err(|e| {
            EngineError::new(ErrorKind::Io, format!("format summary serialization: {e}"))
        })
    }

    async fn inspect(&self, url: &str, hdrs: &RequestHeaders) -> Result<ResourceInfo, EngineError> {
        let client = self.client_for(hdrs.insecure_tls);
        // probe asks for the first 1 KiB, not 1 byte: the bounded prefix is read and
        // sniffed, so a web page can never pass inspection as a file (P0 root cause #1)
        let (resp, chain, final_url) = self
            .follow(
                &client,
                url,
                hdrs,
                Some((0, Some(SNIFF_PROBE_LEN as u64 - 1))),
            )
            .await?;
        let status = resp.status().as_u16();

        // capture every header we need before the body is read (avoids borrow conflicts)
        let (
            etag,
            last_modified,
            content_disposition,
            mime,
            content_encoding,
            www_authenticate,
            retry_after_raw,
            content_range_raw,
            content_length_hdr,
        ) = {
            let h = resp.headers();
            let gs = |name: &str| {
                h.get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            (
                gs("etag"),
                gs("last-modified"),
                gs("content-disposition"),
                gs("content-type"),
                gs("content-encoding"),
                gs("www-authenticate"),
                gs("retry-after"),
                gs("content-range"),
                gs("content-length").and_then(|v| v.parse::<u64>().ok()),
            )
        };

        // P0: we requested identity; a compressed body would be stored as encoded bytes
        if let Some(enc) = content_encoding.as_deref().map(str::trim) {
            let enc = enc.to_ascii_lowercase();
            if !enc.is_empty() && enc != "identity" {
                return Err(EngineError::new(
                    ErrorKind::InvalidResponse,
                    format!(
                        "server sent Content-Encoding: {enc} although identity was requested; \
                         storing encoded bytes would corrupt the file"
                    ),
                ));
            }
        }

        let etag_is_weak = etag
            .as_deref()
            .map(|v| v.starts_with("W/"))
            .unwrap_or(false);

        let (content_length, range_support) = match status {
            206 => match content_range_raw.as_deref().and_then(parse_content_range) {
                Some(CR::Range { total, .. }) => (total, RangeSupport::Yes),
                // Content-Length on a 206 is the PARTIAL length (RFC 9110 §14.4) — never the
                // total. Without a usable Content-Range the total is simply unknown.
                _ => (None, RangeSupport::Unknown),
            },
            416 => match content_range_raw.as_deref().and_then(parse_content_range) {
                Some(CR::Unsatisfied(Some(t))) => (Some(t), RangeSupport::Yes),
                _ => (None, RangeSupport::Unknown),
            },
            200 => (content_length_hdr, RangeSupport::No),
            _ => (content_length_hdr, RangeSupport::Unknown),
        };

        // bounded sniff read: ≤1 KiB from the body. On a 206 the server sends at most
        // the asked range; on a 200 (Range ignored) we read 1 KiB and drop the rest.
        let mut resp = resp;
        let mut head: Vec<u8> = Vec::with_capacity(SNIFF_PROBE_LEN);
        if status == 200 || status == 206 {
            while head.len() < SNIFF_PROBE_LEN {
                match tokio::time::timeout(self.engine.stall_timeout, resp.chunk()).await {
                    Ok(Ok(Some(chunk))) => {
                        let room = SNIFF_PROBE_LEN - head.len();
                        head.extend_from_slice(&chunk[..chunk.len().min(room)]);
                    }
                    _ => break, // clean EOF, transport error, or stall: sniff what we have
                }
            }
        }
        drop(resp); // any unread probe body is discarded — never persisted
        let sniffed = sniff_kind(&head);

        Ok(ResourceInfo {
            final_url,
            redirect_chain: chain,
            status,
            content_length,
            range_support,
            etag,
            etag_is_weak,
            last_modified,
            content_disposition,
            mime,
            content_encoding,
            www_authenticate,
            retry_after: retry_after_raw.and_then(|v| parse_retry_after(&v)),
            sniffed,
            suggested_name: None,
            resolve_note: None,
            media: None,
            thumbnail: None,
        })
    }

    async fn open_range(&self, req: RangeRequest) -> Result<RangeResponse, EngineError> {
        let mut hdrs = req.headers;
        hdrs.if_range = req.if_range.clone();
        let client = self.client_for(hdrs.insecure_tls);
        let range = if req.no_range_header {
            None
        } else {
            Some((req.start, req.end))
        };
        let (resp, _chain, _final_url) = self.follow(&client, &req.url, &hdrs, range).await?;
        let status = resp.status().as_u16();
        let h = resp.headers();
        let header_str = |name: &str| {
            h.get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };

        let content_range = if status == 206 {
            header_str("content-range").and_then(|v| match parse_content_range(&v) {
                Some(CR::Range { start, end, total }) => Some((start, end, total)),
                _ => None,
            })
        } else {
            None
        };
        let content_length = header_str("content-length").and_then(|v| v.parse::<u64>().ok());
        let etag = header_str("etag");

        // P0: identity was requested on the wire; a compressed body would be stored as
        // encoded bytes. Refuse honestly instead of writing a corrupt file.
        if let Some(enc) = header_str("content-encoding").map(|v| v.trim().to_ascii_lowercase()) {
            if !enc.is_empty() && enc != "identity" {
                return Err(EngineError::new(
                    ErrorKind::InvalidResponse,
                    format!(
                        "server sent Content-Encoding: {enc} although identity was requested; \
                         storing encoded bytes would corrupt the file"
                    ),
                ));
            }
        }

        Ok(RangeResponse {
            status,
            content_range,
            content_length,
            etag,
            retry_after: header_str("retry-after").and_then(|v| parse_retry_after(&v)),
            body: Box::new(HttpBody {
                resp,
                stall: self.engine.stall_timeout,
            }),
        })
    }
}

struct HttpBody {
    resp: reqwest::Response,
    stall: Duration,
}

#[async_trait]
impl ByteStream for HttpBody {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, EngineError> {
        // per-chunk stall watchdog: a silent connection longer than `stall` is abandoned
        match tokio::time::timeout(self.stall, self.resp.chunk()).await {
            Err(_) => Err(EngineError::new(
                ErrorKind::Timeout,
                format!(
                    "stalled connection: no data for {:.0}s",
                    self.stall.as_secs_f64()
                ),
            )),
            Ok(r) => r.map_err(|e| map_transport(&e)),
        }
    }
}

fn map_transport(e: &reqwest::Error) -> EngineError {
    let kind = if e.is_timeout() {
        ErrorKind::Timeout
    } else if e.is_connect() {
        ErrorKind::Connect // includes DNS resolution failures and refused sockets
    } else if e.is_body() || e.is_decode() {
        // Premature EOF mid-body and body-decode failures are the canonical transient
        // faults (server died, connection reset) — retryable from the checkpoint, bounded
        // by the retry policy. Genuinely malformed streams exhaust retries and fail.
        ErrorKind::Reset
    } else if e.is_redirect() {
        ErrorKind::InvalidResponse
    } else if e.is_request() {
        ErrorKind::Reset
    } else {
        ErrorKind::Io
    };
    EngineError::new(kind, format!("transport: {e}"))
}

/// Scheme + host + effective port. `https://host` and `http://host` share a `host_of`
/// value, so credentials must be scoped by the full origin triple (audit A1: without the
/// scheme, a https→http downgrade redirect would leak Authorization over plaintext).
fn origin_of(url: &str) -> String {
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

fn cookie_string(cookies: &std::collections::BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

pub enum CR {
    Range {
        start: u64,
        end: u64,
        total: Option<u64>,
    },
    Unsatisfied(Option<u64>),
}

/// Parse `Content-Range` (RFC 9110 §14.4). Overflow-safe u64 parsing; invalid values → None
/// (callers must never recombine content with an invalid Content-Range).
pub fn parse_content_range(v: &str) -> Option<CR> {
    let rest = v.trim().strip_prefix("bytes ")?;
    if let Some(t) = rest.strip_prefix("*/") {
        return Some(CR::Unsatisfied(if t.trim() == "*" {
            None
        } else {
            Some(t.trim().parse().ok()?)
        }));
    }
    let (range, total) = rest.split_once('/')?;
    let (s, e) = range.split_once('-')?;
    let start: u64 = s.trim().parse().ok()?;
    let end: u64 = e.trim().parse().ok()?;
    if end < start {
        return None;
    }
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse().ok()?)
    };
    Some(CR::Range { start, end, total })
}

/// reqwest 0.13's `rustls-no-provider` feature requires an explicit crypto provider
/// before the first Client build (it panics otherwise). We install the ring provider
/// exactly once per process (ADR-0002: no aws-lc-rs, no cmake dependency).
static CRYPTO_PROVIDER: std::sync::Once = std::sync::Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER.call_once(|| {
        let provider = rustls::crypto::ring::default_provider();
        let _ = provider.install_default();
    });
}

fn build_client(
    cfg: &EngineConfig,
    insecure: bool,
    jar: Option<SharedCookieStore>,
) -> Result<reqwest::Client, EngineError> {
    ensure_crypto_provider();
    let mut b = reqwest::Client::builder()
        .user_agent(&cfg.user_agent)
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.stall_timeout)
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(insecure || !cfg.tls_verify)
        // FDM/IDM-class connection reuse: segments and re-resolves reuse pooled
        // keep-alive connections instead of re-handshaking (TLS handshakes are
        // the dominant latency on high-RTT CDNs). tcp_nodelay avoids Nagle
        // stalls on range requests.
        .tcp_nodelay(true)
        .pool_max_idle_per_host(16)
        .pool_idle_timeout(Duration::from_secs(90));
    b = match jar {
        // shared RFC 6265 store backed by the persistent jar file
        Some(store) => b.cookie_provider(store),
        None => b.cookie_store(true),
    };
    if let Some(p) = &cfg.proxy {
        b = b.proxy(reqwest::Proxy::all(p).map_err(|e| {
            EngineError::new(ErrorKind::InvalidConfig, format!("bad proxy url: {e}"))
        })?);
    }
    b.build().map_err(|e| {
        EngineError::new(
            ErrorKind::InvalidConfig,
            format!("client build failed: {e}"),
        )
    })
}

fn load_or_empty_jar(cfg: &EngineConfig) -> Option<(std::path::PathBuf, SharedCookieStore)> {
    let path = cfg.cookie_jar_path.as_ref()?;
    let store = match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<cookie_store::CookieStore>(&bytes) {
            Ok(s) => s,
            Err(e) => {
                // do not silently convert a corrupt jar into an empty one: log and
                // continue with an empty store (cookies may carry session tokens)
                eprintln!(
                    "cookie jar at {} is corrupt ({e}); starting with an empty jar",
                    path.display()
                );
                cookie_store::CookieStore::default()
            }
        },
        Err(_) => cookie_store::CookieStore::default(), // first run: no jar yet
    };
    Some((
        path.clone(),
        std::sync::Arc::new(reqwest_cookie_store::CookieStoreMutex::new(store)),
    ))
}

/// Write the jar atomically with 0600 permissions (Unix): cookies may carry session tokens.
fn save_cookie_jar(path: &std::path::Path, store: &SharedCookieStore) {
    let ok = (|| -> std::io::Result<()> {
        let guard = store
            .lock()
            .map_err(|_| std::io::Error::other("cookie store poisoned"))?;
        let body = serde_json::to_vec(&*guard).map_err(|e| std::io::Error::other(e.to_string()))?;
        // unique per-process tmp name: two processes sharing a jar path must not tear
        // each other's tmp file (review finding 8b)
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        {
            #[cfg(unix)]
            {
                use std::io::Write as _;
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp)?;
                f.write_all(&body)?;
                // fsync the tmp before rename: power loss must not leave a truncated jar
                // that the next load would silently replace with an empty store
                f.sync_all()?;
            }
            #[cfg(not(unix))]
            {
                use std::io::Write as _;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&tmp)?;
                f.write_all(&body)?;
            }
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if let Err(e) = ok {
        tracing::warn!(error = %e, path = %path.display(), "cookie jar persist failed");
    }
}

impl Drop for ReqwestBackend {
    fn drop(&mut self) {
        self.save_cookies();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        let cr = parse_content_range("bytes 100-199/1234").unwrap();
        match cr {
            CR::Range { start, end, total } => {
                assert_eq!((start, end), (100, 199));
                assert_eq!(total, Some(1234));
            }
            _ => panic!(),
        }
        assert!(matches!(
            parse_content_range("bytes */500"),
            Some(CR::Unsatisfied(Some(500)))
        ));
        assert!(matches!(
            parse_content_range("bytes */*"),
            Some(CR::Unsatisfied(None))
        ));
        assert!(matches!(
            parse_content_range("bytes 0-99/*"),
            Some(CR::Range { total: None, .. })
        ));
        assert!(
            parse_content_range("bytes 5-2/10").is_none(),
            "end<start invalid"
        );
        assert!(
            parse_content_range("items 0-5/10").is_none(),
            "unknown unit"
        );
        assert!(
            parse_content_range("bytes 99999999999999999999999999-1/1").is_none(),
            "overflow"
        );
    }

    #[test]
    fn origin_distinguishes_scheme_and_port() {
        // audit A1: host_of alone conflated these; credentials must not cross origins
        assert_ne!(
            origin_of("https://a.example/f"),
            origin_of("http://a.example/f")
        );
        assert_ne!(
            origin_of("https://a.example/f"),
            origin_of("https://a.example:8443/f")
        );
        assert_eq!(
            origin_of("https://a.example/f"),
            origin_of("https://a.example:443/g")
        );
        assert_eq!(
            origin_of("http://a.example/f"),
            origin_of("http://a.example:80/x")
        );
    }

    #[test]
    fn http_date_retry_after_parses() {
        // IMF-fixdate (RFC 9110 §5.6.7) support (Part 6 — HTTP-date Retry-After)
        use engine_core::inspect::parse_retry_after_at;
        use std::time::Duration;
        let now = 1_794_995_970_u64; // 2026-11-18T09:59:30Z (00:00Z == 1_794_960_000)
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 10:00:00 GMT", now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 00:00:00 GMT", now),
            Some(Duration::from_secs(0))
        );
        assert_eq!(parse_retry_after_at("5", now), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after_at("soon-ish", now), None);
    }
}
