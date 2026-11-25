//! Engine and per-download configuration.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::retry::RetryPolicy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Persistent state directory (holds state.db). Set explicitly by the host application.
    pub data_dir: PathBuf,
    /// Default destination directory when a download config does not override it.
    pub default_out_dir: PathBuf,
    /// Hard ceiling on concurrently running downloads' connections per host (politeness).
    pub max_connections_per_host: usize,
    pub max_connections_per_download: usize,
    /// Splitting floor: never create segments smaller than 2×this via planning or stealing.
    pub min_segment_size: u64,
    pub user_agent: String,
    pub connect_timeout: Duration,
    /// Per-read stall timeout: a connection silent longer than this is abandoned and retried.
    pub stall_timeout: Duration,
    /// Durable checkpoint cadence (bytes per worker) — fsync + DB commit (ADR-0007).
    pub checkpoint_interval: u64,
    pub checkpoint_period: Duration,
    pub retry: RetryPolicy,
    /// Explicit proxy URL (http://, https://, socks5://). System proxy auto-detection is off.
    pub proxy: Option<String>,
    /// TLS certificate verification is on by default and only disabled by explicit per-download opt-in.
    pub tls_verify: bool,
    /// Persistent RFC 6265 cookie jar path (JSON, 0600). None = in-memory per-process store.
    pub cookie_jar_path: Option<std::path::PathBuf>,
    /// Maximum simultaneously RUNNING downloads (admission control; queued starts wait FIFO).
    /// None = unlimited (pre-queue behavior).
    pub max_concurrent_downloads: Option<usize>,
    /// Event bus capacity.
    pub event_capacity: usize,
    /// External media extractor (yt-dlp) executable path or bare PATH name.
    /// None = media-page URLs fail with UNSUPPORTED_URL and an explanation. The engine
    /// never bundles the extractor; see docs/EXTRACTORS.md for licensing and packaging.
    pub extractor_path: Option<PathBuf>,
    /// Media-extractor invocation timeout (audit D1: bounded; kill_on_drop reclaims the
    /// child process on expiry). Default 120 s; tests shorten it to provoke that path.
    #[serde(default = "default_extractor_timeout")]
    pub extractor_timeout: Duration,
}

fn default_extractor_timeout() -> Duration {
    Duration::from_secs(120)
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("velox-data"),
            default_out_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            max_connections_per_host: 8,
            max_connections_per_download: 8,
            min_segment_size: 8 * 1024 * 1024,
            user_agent: format!("velox/{}", env!("CARGO_PKG_VERSION")),
            connect_timeout: Duration::from_secs(10),
            stall_timeout: Duration::from_secs(30),
            checkpoint_interval: 4 * 1024 * 1024,
            checkpoint_period: Duration::from_secs(3),
            retry: RetryPolicy::default(),
            proxy: None,
            cookie_jar_path: None,
            max_concurrent_downloads: None,
            tls_verify: true,
            event_capacity: 256,
            extractor_path: None,
            extractor_timeout: default_extractor_timeout(),
        }
    }
}

impl EngineConfig {
    pub fn with_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = dir.into();
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub out_dir: Option<PathBuf>,
    pub filename: Option<String>,
    /// Extra request headers (e.g. "Accept", "X-Session").
    pub headers: std::collections::BTreeMap<String, String>,
    /// Cookies for the first request ("name=value" pairs; store updates follow RFC 6265 via the client).
    pub cookies: std::collections::BTreeMap<String, String>,
    pub referer: Option<String>,
    pub auth: Option<AuthConfig>,
    /// Override of per-download connection count (clamped by EngineConfig ceilings).
    pub max_connections: Option<usize>,
    /// Per-download floor for initial segment size (bytes). Overrides
    /// `EngineConfig::min_segment_size` when set; the initial wave is
    /// `ceil(total / (2 * min_segment))` connections, growing via tail-stealing.
    pub min_segment_size: Option<u64>,
    /// Global per-download cap in bytes/second (token bucket).
    pub bandwidth_limit_bps: Option<u64>,
    /// Expected checksum; verified before completion is claimed when present.
    pub verify: Option<VerifyRequest>,
    pub preallocate: bool,
    /// If the final name exists: overwrite=true replaces it, false picks `name(1).ext`.
    pub overwrite: bool,
    pub auto_start: bool,
    pub resume_policy: ResumePolicy,
    /// Explicit, per-download TLS verification opt-out (SECURITY §3). Never set by the engine itself.
    pub insecure_tls: bool,
    /// Accept a hypertext document as the download even when it looks like a web page.
    /// None (default) = automatic policy: HTML is accepted only when the user named the
    /// file *.html/.htm or the resolved URL path ends in .html/.htm.
    pub allow_html: Option<bool>,
    /// MediaPipeline (review §9): this download is stream 1 (video) of a two-stream
    /// media job; `audio_url` is stream 2, downloaded with the same config after
    /// stream 1 completes, then merged (`-c copy`) into the final artifact.
    /// The merge itself only runs when a MediaMergePort is configured on the manager.
    pub media_merge: Option<MediaMergeSpec>,
    /// User's format choice for media downloads (Add dialog): MP4 (merged
    /// video+audio) or MP3 (audio-only, converted with the bundled ffmpeg).
    /// Absent keeps the legacy "best available" resolution.
    #[serde(default)]
    pub media_format: Option<MediaFormat>,
    /// Requested video height cap in pixels (480/720/1080/…), from the honest
    /// quality list. The extractor only offers qualities the source actually
    /// has; this is the chosen one. `None` = best available.
    #[serde(default)]
    pub media_height: Option<u32>,
    /// Preferred fps for the selected height (e.g. Some(60) for "1080p60").
    #[serde(default)]
    pub media_fps: Option<u32>,
}

/// Container/format choice for media downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaFormat {
    /// Video + audio, merged to mp4 when the source is adaptive.
    Mp4,
    /// Audio only, converted to mp3 (libmp3lame, `-q:a 0` VBR best).
    Mp3,
}

/// Two-stream media job spec (DASH/HLS resolved by yt-dlp upstream — the URLs here
/// are direct media URLs, not pages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaMergeSpec {
    /// Direct media URL of the audio stream.
    pub audio_url: String,
    /// Extension of the final merged file (default "mp4").
    pub output_ext: Option<String>,
    /// Explicit final artifact NAME (e.g. "Title.mp4"). When absent the name is
    /// derived from the video row (`<video stem>.<output_ext>`); the auto media
    /// job sets it so the merged artifact carries the original media title.
    #[serde(default)]
    pub output_name: Option<String>,
    /// Explicit file NAME of the audio stream row (e.g. "Title (audio stream).m4a").
    /// Set by the extractor when it knows the audio container; absent → naming falls
    /// back to URL/mime-derived names.
    #[serde(default)]
    pub audio_name: Option<String>,
    /// Post-download conversion (MP3 extraction). When set, the media job
    /// converts the single downloaded stream instead of merging two.
    #[serde(default)]
    pub convert: Option<MediaConvert>,
}

/// Post-download conversion mode for media jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaConvert {
    /// Extract/transcode audio to MP3 (libmp3lame, VBR best `-q:a 0`).
    Mp3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthConfig {
    Basic { user: String, password: String },
    Bearer { token: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub algo: HashAlgo,
    pub expected: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgo {
    Sha256,
    Blake3,
}

impl HashAlgo {
    pub fn parse(s: &str) -> Option<HashAlgo> {
        match s.to_ascii_lowercase().as_str() {
            "sha256" | "sha-256" => Some(HashAlgo::Sha256),
            "blake3" => Some(HashAlgo::Blake3),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Blake3 => "blake3",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResumePolicy {
    /// If the server ever provided a validator, resume requires it to still match (default).
    #[default]
    RequireValidatorIfAvailable,
    /// Resume on size match even when validators changed/are missing (IDM-style; user-accepted risk).
    AllowUnverified,
}
